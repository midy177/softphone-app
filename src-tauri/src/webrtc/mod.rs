pub mod audio_bridge;
pub mod codec;

use rustrtc::config::MediaCapabilities;
use rustrtc::{
    AudioCapability, MediaKind, PeerConnection, RtcConfiguration, RtpCodecParameters,
    TransportMode,
};
use tracing::{debug, info};

use audio_bridge::AudioBridge;

/// A WebRTC session wrapping a PeerConnection and audio bridge for one call.
pub struct WebRtcSession {
    pc: PeerConnection,
    audio_bridge: AudioBridge,
}

impl WebRtcSession {
    /// Create a new outbound session. Returns `(session, sdp_offer_string)`.
    ///
    /// This sets up:
    /// - PeerConnection in RTP mode (no ICE/DTLS)
    /// - Audio track with PCMU/PCMA codecs
    /// - AudioBridge (capture NOT started yet — waits for SDP negotiation)
    pub async fn new_outbound(
        input_device: Option<&str>,
        output_device: Option<&str>,
    ) -> Result<(Self, String), String> {
        // Configure for SIP/RTP mode with PCMU codec
        let config = RtcConfiguration {
            transport_mode: TransportMode::Rtp,
            media_capabilities: Some(MediaCapabilities {
                audio: vec![
                    AudioCapability::pcmu(),
                    AudioCapability::pcma(),
                    AudioCapability::telephone_event(),
                ],
                video: vec![],
                application: None,
            }),
            enable_latching: true,
            ..Default::default()
        };

        let pc = PeerConnection::new(config);

        // Create audio bridge (validates devices, creates track, but does NOT start capture)
        let (audio_bridge, send_track) =
            AudioBridge::new(input_device, output_device)?;

        // Add the capture track to PeerConnection with PCMU codec parameters
        let params = RtpCodecParameters {
            payload_type: 0,
            clock_rate: 8000,
            channels: 1,
        };
        pc.add_track(send_track, params)
            .map_err(|e| format!("Failed to add audio track: {}", e))?;

        // Create SDP offer
        let _ = pc
            .create_offer()
            .await
            .map_err(|e| format!("Failed to trigger gathering: {}", e))?;
        pc.wait_for_gathering_complete().await;

        let offer = pc
            .create_offer()
            .await
            .map_err(|e| format!("Failed to create SDP offer: {}", e))?;

        let sdp_string = offer.to_sdp_string();
        debug!(sdp_len = sdp_string.len(), "SDP offer created");

        pc.set_local_description(offer)
            .map_err(|e| format!("Failed to set local description: {}", e))?;

        let session = WebRtcSession { pc, audio_bridge };

        info!("WebRTC outbound session created");
        Ok((session, sdp_string))
    }

    /// Apply the remote SDP answer and start audio capture/playback
    /// using the negotiated codec parameters.
    pub async fn apply_answer(
        &mut self,
        sdp_answer: &str,
        output_device: Option<&str>,
    ) -> Result<(), String> {
        // Parse negotiated codec from SDP answer
        let negotiated = codec::parse_negotiated_codec(sdp_answer);
        info!(
            codec = %negotiated.codec,
            pt = negotiated.payload_type,
            rate = negotiated.clock_rate,
            ptime = negotiated.ptime_ms,
            "Negotiated codec from SDP answer"
        );

        let answer = rustrtc::SessionDescription::parse(rustrtc::SdpType::Answer, sdp_answer)
            .map_err(|e| format!("Failed to parse SDP answer: {}", e))?;

        self.pc
            .set_remote_description(answer)
            .await
            .map_err(|e| format!("Failed to set remote description: {}", e))?;

        info!("Remote SDP answer applied, waiting for connection...");

        // Wait for connection (with timeout)
        match tokio::time::timeout(
            std::time::Duration::from_secs(10),
            self.pc.wait_for_connected(),
        )
        .await
        {
            Ok(Ok(_)) => info!("RTP connection established"),
            Ok(Err(e)) => return Err(format!("Connection failed: {}", e)),
            Err(_) => return Err("Connection timed out".to_string()),
        }

        // Start capture with negotiated codec
        self.audio_bridge.start_capture(&negotiated)?;

        // Start playback from remote track with negotiated codec
        let transceivers = self.pc.get_transceivers();
        for t in &transceivers {
            if t.kind() == MediaKind::Audio {
                if let Some(receiver) = t.receiver() {
                    let remote_track = receiver.track();
                    self.audio_bridge
                        .start_playback(output_device, remote_track, &negotiated)?;
                    info!("Audio playback started");
                    break;
                }
            }
        }

        Ok(())
    }

    /// Toggle microphone mute. Returns new mute state.
    pub fn toggle_mic_mute(&self) -> bool {
        self.audio_bridge.toggle_mic_mute()
    }

    /// Toggle speaker mute. Returns new mute state.
    pub fn toggle_speaker_mute(&self) -> bool {
        self.audio_bridge.toggle_speaker_mute()
    }

    /// Close the session: stop audio, close PeerConnection.
    pub fn close(&mut self) {
        info!("Closing WebRTC session");
        self.audio_bridge.close();
        self.pc.close();
    }
}

impl Drop for WebRtcSession {
    fn drop(&mut self) {
        self.close();
    }
}
