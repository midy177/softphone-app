use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use bytes::Bytes;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{DeviceId, SampleFormat, StreamConfig};
use ringbuf::traits::{Consumer, Observer, Producer, Split};
use ringbuf::HeapRb;
use rustrtc::media::frame::{AudioFrame, MediaSample};
use rustrtc::media::track::{sample_track, SampleStreamSource, SampleStreamTrack};
use rustrtc::media::MediaStreamTrack;
use tokio::sync::Notify;
use tracing::{debug, error, info, warn};

use super::codec::NegotiatedCodec;

/// AudioBridge connects cpal audio I/O to rustrtc media tracks.
pub struct AudioBridge {
    capture_stream: Option<cpal::Stream>,
    playback_stream: Option<cpal::Stream>,
    mic_muted: Arc<AtomicBool>,
    speaker_muted: Arc<AtomicBool>,
    stop_notify: Arc<Notify>,
    audio_source: SampleStreamSource,
    input_device_name: Option<String>,
}

impl AudioBridge {
    /// Create a new AudioBridge. Validates devices and creates the send track.
    /// Capture and playback are NOT started yet — call `start_capture()` and
    /// `start_playback()` after SDP negotiation to use the negotiated codec.
    ///
    /// Returns `(AudioBridge, Arc<SampleStreamTrack>)`.
    pub fn new(
        input_device_name: Option<&str>,
        output_device_name: Option<&str>,
    ) -> Result<(Self, Arc<SampleStreamTrack>), String> {
        let host = cpal::default_host();

        // Validate input device exists
        let input_device = if let Some(name) = input_device_name {
            find_device_by_id(&host, name)?
        } else {
            host.default_input_device()
                .ok_or_else(|| "No default input device".to_string())?
        };

        // Validate output device exists
        if let Some(name) = output_device_name {
            find_device_by_id(&host, name)?;
        }

        let input_desc = input_device
            .description()
            .map(|d| d.name().to_string())
            .unwrap_or_default();
        info!(input = %input_desc, "Audio input device selected");

        // Create sample track for sending captured audio
        let (audio_source, track, _feedback_rx) =
            sample_track(rustrtc::media::frame::MediaKind::Audio, 100);

        let bridge = AudioBridge {
            capture_stream: None,
            playback_stream: None,
            mic_muted: Arc::new(AtomicBool::new(false)),
            speaker_muted: Arc::new(AtomicBool::new(false)),
            stop_notify: Arc::new(Notify::new()),
            audio_source,
            input_device_name: input_device_name.map(|s| s.to_string()),
        };

        Ok((bridge, track))
    }

    /// Start capturing audio from the microphone using the negotiated codec.
    pub fn start_capture(&mut self, negotiated: &NegotiatedCodec) -> Result<(), String> {
        let host = cpal::default_host();
        let input_device = if let Some(ref name) = self.input_device_name {
            find_device_by_id(&host, name)?
        } else {
            host.default_input_device()
                .ok_or_else(|| "No default input device".to_string())?
        };

        let capture_stream = setup_capture_stream(
            &input_device,
            &self.audio_source,
            self.mic_muted.clone(),
            self.stop_notify.clone(),
            negotiated,
        )?;

        self.capture_stream = Some(capture_stream);
        info!(codec = %negotiated.codec, ptime = negotiated.ptime_ms, "Capture started");
        Ok(())
    }

    /// Start playing received audio from the remote track to the speaker.
    pub fn start_playback(
        &mut self,
        output_device_name: Option<&str>,
        remote_track: Arc<SampleStreamTrack>,
        negotiated: &NegotiatedCodec,
    ) -> Result<(), String> {
        let host = cpal::default_host();
        let output_device = if let Some(name) = output_device_name {
            find_device_by_id(&host, name)?
        } else {
            host.default_output_device()
                .ok_or_else(|| "No default output device".to_string())?
        };

        let playback_stream = setup_playback_stream(
            &output_device,
            remote_track,
            self.speaker_muted.clone(),
            self.stop_notify.clone(),
            negotiated,
        )?;

        self.playback_stream = Some(playback_stream);
        info!(codec = %negotiated.codec, ptime = negotiated.ptime_ms, "Playback started");
        Ok(())
    }

    pub fn toggle_mic_mute(&self) -> bool {
        let prev = self.mic_muted.fetch_xor(true, Ordering::Relaxed);
        let new_state = !prev;
        debug!(muted = new_state, "Mic mute toggled");
        new_state
    }

    pub fn toggle_speaker_mute(&self) -> bool {
        let prev = self.speaker_muted.fetch_xor(true, Ordering::Relaxed);
        let new_state = !prev;
        debug!(muted = new_state, "Speaker mute toggled");
        new_state
    }

    pub fn is_mic_muted(&self) -> bool {
        self.mic_muted.load(Ordering::Relaxed)
    }

    pub fn is_speaker_muted(&self) -> bool {
        self.speaker_muted.load(Ordering::Relaxed)
    }

    pub fn close(&mut self) {
        info!("Closing audio bridge");
        self.stop_notify.notify_waiters();
        self.capture_stream.take();
        self.playback_stream.take();
    }
}

impl Drop for AudioBridge {
    fn drop(&mut self) {
        self.close();
    }
}

/// Find a cpal device by its ID string (format: "host:device_id").
fn find_device_by_id(
    host: &cpal::Host,
    id_str: &str,
) -> Result<cpal::Device, String> {
    let device_id: DeviceId = id_str
        .parse()
        .map_err(|e| format!("Invalid device ID '{}': {}", id_str, e))?;
    host.device_by_id(&device_id)
        .ok_or_else(|| format!("Audio device not found: {}", id_str))
}

/// Set up the capture stream: mic → ringbuf → tokio task → encode → send to rustrtc
fn setup_capture_stream(
    device: &cpal::Device,
    audio_source: &SampleStreamSource,
    mic_muted: Arc<AtomicBool>,
    stop_notify: Arc<Notify>,
    negotiated: &NegotiatedCodec,
) -> Result<cpal::Stream, String> {
    let supported_config = device
        .default_input_config()
        .map_err(|e| format!("No input config: {}", e))?;

    let device_sample_rate = supported_config.sample_rate();
    let channels = supported_config.channels() as usize;
    debug!(sample_rate = device_sample_rate, channels, "Input device config");

    let stream_config = StreamConfig {
        channels: supported_config.channels(),
        sample_rate: device_sample_rate,
        buffer_size: cpal::BufferSize::Default,
    };

    // Codec parameters from SDP negotiation
    let codec_sample_rate = negotiated.clock_rate;
    let frame_samples = negotiated.frame_samples();
    let frame_duration_ms = negotiated.ptime_ms;
    let codec_type = negotiated.codec;

    // Ring buffer: ~200ms of audio at device sample rate
    let rb_capacity = (device_sample_rate as usize / 1000) * 200;
    let rb = HeapRb::<f32>::new(rb_capacity);
    let (mut producer, mut consumer) = rb.split();

    // cpal capture callback → write raw f32 samples to ring buffer
    let stream = match supported_config.sample_format() {
        SampleFormat::F32 => device.build_input_stream(
            &stream_config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                if channels > 1 {
                    for chunk in data.chunks(channels) {
                        let mono: f32 = chunk.iter().sum::<f32>() / channels as f32;
                        let _ = producer.try_push(mono);
                    }
                } else {
                    for &s in data {
                        let _ = producer.try_push(s);
                    }
                }
            },
            |err| error!("Capture stream error: {}", err),
            None,
        ),
        SampleFormat::I16 => device.build_input_stream(
            &StreamConfig {
                channels: supported_config.channels(),
                sample_rate: device_sample_rate,
                buffer_size: cpal::BufferSize::Default,
            },
            move |data: &[i16], _: &cpal::InputCallbackInfo| {
                if channels > 1 {
                    for chunk in data.chunks(channels) {
                        let mono: f32 = chunk.iter().map(|&s| s as f32 / 32768.0).sum::<f32>()
                            / channels as f32;
                        let _ = producer.try_push(mono);
                    }
                } else {
                    for &s in data {
                        let _ = producer.try_push(s as f32 / 32768.0);
                    }
                }
            },
            |err| error!("Capture stream error: {}", err),
            None,
        ),
        fmt => return Err(format!("Unsupported sample format: {:?}", fmt)),
    }
    .map_err(|e| format!("Failed to build input stream: {}", e))?;

    stream
        .play()
        .map_err(|e| format!("Failed to start capture: {}", e))?;

    // Tokio task: read from ring buffer → resample → encode → send AudioFrame
    let audio_source_clone = audio_source.clone();
    tokio::spawn(async move {
        let needs_resample = device_sample_rate != codec_sample_rate;
        let ratio = device_sample_rate as f64 / codec_sample_rate as f64;
        let device_frame_samples = if needs_resample {
            (frame_samples as f64 * ratio).ceil() as usize
        } else {
            frame_samples
        };

        let mut resampler = if needs_resample {
            Some(
                rubato::FftFixedOut::<f32>::new(
                    device_sample_rate as usize,
                    codec_sample_rate as usize,
                    frame_samples,
                    1,
                    1,
                )
                .expect("Failed to create resampler"),
            )
        } else {
            None
        };

        let mut device_buf = vec![0.0f32; device_frame_samples];
        let mut rtp_timestamp: u32 = 0;
        let frame_interval = tokio::time::Duration::from_millis(frame_duration_ms as u64);
        let mut interval = tokio::time::interval(frame_interval);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                _ = interval.tick() => {},
                _ = stop_notify.notified() => {
                    debug!("Capture task stopping");
                    break;
                }
            }

            // If mic is muted, send silence
            if mic_muted.load(Ordering::Relaxed) {
                let silence = vec![0u8; frame_samples];
                let frame = AudioFrame {
                    rtp_timestamp,
                    clock_rate: codec_sample_rate,
                    data: Bytes::from(silence),
                    ..Default::default()
                };
                if audio_source_clone.send_audio(frame).await.is_err() {
                    break;
                }
                rtp_timestamp = rtp_timestamp.wrapping_add(frame_samples as u32);
                continue;
            }

            // Read from ring buffer
            let available = consumer.occupied_len();
            let needed = device_frame_samples;
            if available < needed {
                let silence = vec![0u8; frame_samples];
                let frame = AudioFrame {
                    rtp_timestamp,
                    clock_rate: codec_sample_rate,
                    data: Bytes::from(silence),
                    ..Default::default()
                };
                if audio_source_clone.send_audio(frame).await.is_err() {
                    break;
                }
                rtp_timestamp = rtp_timestamp.wrapping_add(frame_samples as u32);
                continue;
            }

            for i in 0..needed {
                device_buf[i] = consumer.try_pop().unwrap_or(0.0);
            }

            // Resample if needed (device rate → codec rate)
            let pcm_f32 = if let Some(ref mut resampler) = resampler {
                use rubato::Resampler;
                let input = vec![device_buf[..needed].to_vec()];
                match resampler.process(&input, None) {
                    Ok(output) => output.into_iter().next().unwrap_or_default(),
                    Err(e) => {
                        warn!("Resample error: {}", e);
                        vec![0.0f32; frame_samples]
                    }
                }
            } else {
                device_buf[..frame_samples].to_vec()
            };

            // Convert f32 → i16 → encode with negotiated codec
            let pcm_i16: Vec<i16> = pcm_f32
                .iter()
                .map(|&s| {
                    let clamped = s.clamp(-1.0, 1.0);
                    (clamped * 32767.0) as i16
                })
                .collect();
            let encoded = codec_type.encode(&pcm_i16);

            let frame = AudioFrame {
                rtp_timestamp,
                clock_rate: codec_sample_rate,
                data: Bytes::from(encoded),
                ..Default::default()
            };

            if audio_source_clone.send_audio(frame).await.is_err() {
                debug!("Audio source closed, stopping capture");
                break;
            }

            rtp_timestamp = rtp_timestamp.wrapping_add(frame_samples as u32);
        }
    });

    Ok(stream)
}

/// Set up the playback stream: remote track → decode → resample → ringbuf → speaker
fn setup_playback_stream(
    device: &cpal::Device,
    remote_track: Arc<SampleStreamTrack>,
    speaker_muted: Arc<AtomicBool>,
    stop_notify: Arc<Notify>,
    negotiated: &NegotiatedCodec,
) -> Result<cpal::Stream, String> {
    let supported_config = device
        .default_output_config()
        .map_err(|e| format!("No output config: {}", e))?;

    let device_sample_rate = supported_config.sample_rate();
    let channels = supported_config.channels() as usize;
    debug!(sample_rate = device_sample_rate, channels, "Output device config");

    let stream_config = StreamConfig {
        channels: supported_config.channels(),
        sample_rate: device_sample_rate,
        buffer_size: cpal::BufferSize::Default,
    };

    // Codec parameters from SDP negotiation
    let codec_sample_rate = negotiated.clock_rate;
    let frame_samples = negotiated.frame_samples();
    let codec_type = negotiated.codec;

    // Ring buffer: ~200ms of audio at device sample rate, per channel
    let rb_capacity = (device_sample_rate as usize / 1000) * 200 * channels;
    let rb = HeapRb::<f32>::new(rb_capacity);
    let (mut producer, mut consumer) = rb.split();

    // Tokio task: receive from remote track → decode → resample → write to ring buffer
    let stop = stop_notify.clone();
    let muted = speaker_muted.clone();
    tokio::spawn(async move {
        let needs_resample = device_sample_rate != codec_sample_rate;

        let mut resampler = if needs_resample {
            Some(
                rubato::FftFixedIn::<f32>::new(
                    codec_sample_rate as usize,
                    device_sample_rate as usize,
                    frame_samples,
                    1,
                    1,
                )
                .expect("Failed to create playback resampler"),
            )
        } else {
            None
        };

        loop {
            tokio::select! {
                result = remote_track.recv() => {
                    match result {
                        Ok(MediaSample::Audio(frame)) => {
                            if muted.load(Ordering::Relaxed) {
                                continue;
                            }

                            // Decode with negotiated codec → i16 → f32
                            let pcm_i16 = codec_type.decode(&frame.data);
                            let pcm_f32: Vec<f32> = pcm_i16
                                .iter()
                                .map(|&s| s as f32 / 32768.0)
                                .collect();

                            // Resample if needed (codec rate → device rate)
                            let output_samples = if let Some(ref mut resampler) = resampler {
                                use rubato::Resampler;
                                let input = vec![pcm_f32];
                                match resampler.process(&input, None) {
                                    Ok(output) => output.into_iter().next().unwrap_or_default(),
                                    Err(e) => {
                                        warn!("Playback resample error: {}", e);
                                        continue;
                                    }
                                }
                            } else {
                                pcm_f32
                            };

                            // Write to ring buffer, duplicating to all channels
                            for &s in &output_samples {
                                for _ in 0..channels {
                                    let _ = producer.try_push(s);
                                }
                            }
                        }
                        Ok(_) => {}
                        Err(_) => {
                            debug!("Remote track ended");
                            break;
                        }
                    }
                }
                _ = stop.notified() => {
                    debug!("Playback task stopping");
                    break;
                }
            }
        }
    });

    // cpal playback callback: read from ring buffer → output to speaker
    let stream = device
        .build_output_stream(
            &stream_config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                for sample in data.iter_mut() {
                    *sample = consumer.try_pop().unwrap_or(0.0);
                }
            },
            |err| error!("Playback stream error: {}", err),
            None,
        )
        .map_err(|e| format!("Failed to build output stream: {}", e))?;

    stream
        .play()
        .map_err(|e| format!("Failed to start playback: {}", e))?;

    Ok(stream)
}
