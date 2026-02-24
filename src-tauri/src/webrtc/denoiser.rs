use audio_codec::Resampler;
use nnnoiseless::DenoiseState;

/// Real-time microphone noise reducer using RNNoise (nnnoiseless).
///
/// Processing chain per frame:
///   i16 PCM @ codec_rate
///     → resample to 48 kHz  (nnnoiseless requires 48 kHz)
///     → f32 i16-scale        (range −32768..32767, not normalised)
///     → DenoiseState::process_frame() in 480-sample chunks (10 ms)
///     → f32 → i16
///     → resample back to codec_rate
///     → resize to exact expected_len
///
/// The DenoiseState is stateful across frames — create once per call, not
/// once per packet.
pub struct NoiseReducer {
    denoiser: Box<DenoiseState<'static>>,
    /// codec_rate → 48 000 Hz (None when codec_rate already is 48 000)
    up_resampler: Option<Resampler>,
    /// 48 000 Hz → codec_rate (None when codec_rate already is 48 000)
    down_resampler: Option<Resampler>,
}

// DenoiseState contains raw pointers, but we only touch it from a single task.
unsafe impl Send for NoiseReducer {}

impl NoiseReducer {
    /// `codec_sample_rate` must match the rate of PCM passed to `process()`.
    pub fn new(codec_sample_rate: u32) -> Self {
        let (up, down) = if codec_sample_rate == 48_000 {
            (None, None)
        } else {
            (
                Some(Resampler::new(codec_sample_rate as usize, 48_000)),
                Some(Resampler::new(48_000, codec_sample_rate as usize)),
            )
        };
        Self {
            denoiser: DenoiseState::new(),
            up_resampler: up,
            down_resampler: down,
        }
    }

    /// Denoise one PCM frame.
    ///
    /// * `pcm`          – input samples at codec_rate
    /// * `expected_len` – target output length (= frame_samples for the codec);
    ///                    the result is zero-padded or truncated to this size.
    pub fn process(&mut self, pcm: &[i16], expected_len: usize) -> Vec<i16> {
        // 1. Resample up to 48 kHz (or skip if already at 48 kHz)
        let upsampled: Vec<i16> = match self.up_resampler {
            Some(ref mut r) => r.resample(pcm),
            None => pcm.to_vec(),
        };
        let up_len = upsampled.len();

        // 2. Convert to f32 in i16 scale (nnnoiseless operates in −32768..32767)
        let input_f32: Vec<f32> = upsampled.iter().map(|&s| s as f32).collect();

        // 3. Run DenoiseState in FRAME_SIZE (480 sample) chunks
        //    Both input and output slices must be exactly FRAME_SIZE long.
        let mut output_f32 = vec![0.0f32; up_len];
        let mut pad = vec![0.0f32; DenoiseState::FRAME_SIZE];
        let mut out_chunk = vec![0.0f32; DenoiseState::FRAME_SIZE];
        let mut offset = 0;

        while offset < up_len {
            let remaining = up_len - offset;
            let chunk_len = remaining.min(DenoiseState::FRAME_SIZE);

            // Build exactly-FRAME_SIZE input (zero-pad the last partial chunk)
            let input_chunk: &[f32] = if chunk_len < DenoiseState::FRAME_SIZE {
                pad[..chunk_len].copy_from_slice(&input_f32[offset..offset + chunk_len]);
                pad[chunk_len..].fill(0.0);
                &pad
            } else {
                &input_f32[offset..offset + chunk_len]
            };

            self.denoiser.process_frame(&mut out_chunk, input_chunk);

            // Copy only the valid (non-padded) samples to output
            let write_len = chunk_len.min(up_len - offset);
            output_f32[offset..offset + write_len]
                .copy_from_slice(&out_chunk[..write_len]);

            offset += chunk_len;
        }

        // 4. f32 → i16 (clamp to avoid overflow)
        let denoised: Vec<i16> = output_f32
            .iter()
            .map(|&s| s.clamp(i16::MIN as f32, i16::MAX as f32) as i16)
            .collect();

        // 5. Resample back to codec_rate (or skip if 48 kHz)
        let mut result: Vec<i16> = match self.down_resampler {
            Some(ref mut r) => r.resample(&denoised),
            None => denoised,
        };

        // 6. Guarantee exact output length (the resampler may drift by ±1 sample)
        result.resize(expected_len, 0);
        result
    }
}
