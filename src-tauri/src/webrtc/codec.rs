/// Codec support using audio-codec crate.
///
/// Directly uses audio-codec's CodecType to support all available codecs:
/// PCMU, PCMA, G722, G729, Opus, etc.

pub use audio_codec::CodecType;
use audio_codec::{create_decoder, create_encoder};

/// Extension trait for CodecType to add helper methods
pub trait CodecTypeExt {
    /// Determine codec from RTP payload type
    fn from_payload_type(pt: u8) -> Option<CodecType>;

    /// Get RTP payload type for this codec
    fn to_payload_type(&self) -> u8;

    /// Get default clock rate for this codec
    fn default_clock_rate(&self) -> u32;

    /// Encode PCM samples
    fn encode(&self, pcm: &[i16]) -> Vec<u8>;

    /// Decode encoded data to PCM samples
    fn decode(&self, data: &[u8]) -> Vec<i16>;
}

impl CodecTypeExt for CodecType {
    fn from_payload_type(pt: u8) -> Option<CodecType> {
        match pt {
            0 => Some(CodecType::PCMU),
            8 => Some(CodecType::PCMA),
            9 => Some(CodecType::G722),
            18 => Some(CodecType::G729),
            111 => Some(CodecType::Opus), // Common dynamic PT for Opus
            _ => None,
        }
    }

    fn to_payload_type(&self) -> u8 {
        match self {
            CodecType::PCMU => 0,
            CodecType::PCMA => 8,
            CodecType::G722 => 9,
            CodecType::G729 => 18,
            CodecType::Opus => 111,
            CodecType::TelephoneEvent => 101, // RFC 4733
        }
    }

    fn default_clock_rate(&self) -> u32 {
        match self {
            CodecType::PCMU | CodecType::PCMA => 8000,
            CodecType::G722 => 16000,
            CodecType::G729 => 8000,
            CodecType::Opus => 48000,
            CodecType::TelephoneEvent => 8000,
        }
    }

    fn encode(&self, pcm: &[i16]) -> Vec<u8> {
        let mut encoder = create_encoder(*self);
        encoder.encode(pcm)
    }

    fn decode(&self, data: &[u8]) -> Vec<i16> {
        let mut decoder = create_decoder(*self);
        decoder.decode(data)
    }
}

/// Parameters negotiated from SDP answer
#[derive(Debug, Clone)]
pub struct NegotiatedCodec {
    pub codec: CodecType,
    pub payload_type: u8,
    pub clock_rate: u32,
    pub ptime_ms: u32,
}

impl NegotiatedCodec {
    /// Samples per frame = clock_rate * ptime_ms / 1000
    pub fn frame_samples(&self) -> usize {
        (self.clock_rate * self.ptime_ms / 1000) as usize
    }
}

impl Default for NegotiatedCodec {
    fn default() -> Self {
        Self {
            codec: CodecType::PCMU,
            payload_type: 0,
            clock_rate: 8000,
            ptime_ms: 20,
        }
    }
}

/// Parse negotiated codec from SDP answer text.
/// Extracts the first supported audio codec and ptime.
pub fn parse_negotiated_codec(sdp: &str) -> NegotiatedCodec {
    let mut result = NegotiatedCodec::default();
    let mut in_audio_section = false;
    let mut media_pt: Option<u8> = None;

    for line in sdp.lines() {
        let line = line.trim();

        // m=audio 5004 RTP/AVP 0 8 111
        if line.starts_with("m=audio") {
            in_audio_section = true;
            // First format in m= line is the preferred codec
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 4 {
                if let Ok(pt) = parts[3].parse::<u8>() {
                    media_pt = Some(pt);
                }
            }
        } else if line.starts_with("m=") {
            in_audio_section = false;
        }

        if !in_audio_section {
            continue;
        }

        // a=rtpmap:0 PCMU/8000 or a=rtpmap:111 opus/48000/2
        if line.starts_with("a=rtpmap:") {
            if let Some(rest) = line.strip_prefix("a=rtpmap:") {
                let parts: Vec<&str> = rest.splitn(2, ' ').collect();
                if parts.len() == 2 {
                    if let Ok(pt) = parts[0].parse::<u8>() {
                        let codec_parts: Vec<&str> = parts[1].split('/').collect();
                        if let Some(&codec_name) = codec_parts.first() {
                            let codec = match codec_name.to_uppercase().as_str() {
                                "PCMU" => Some(CodecType::PCMU),
                                "PCMA" => Some(CodecType::PCMA),
                                "G722" => Some(CodecType::G722),
                                "G729" => Some(CodecType::G729),
                                "OPUS" => Some(CodecType::Opus),
                                _ => None,
                            };
                            // Only use this if it matches the preferred PT from m= line
                            if let (Some(c), Some(mpt)) = (codec, media_pt) {
                                if pt == mpt {
                                    result.codec = c;
                                    result.payload_type = pt;
                                    if let Some(rate_str) = codec_parts.get(1) {
                                        if let Ok(rate) = rate_str.parse::<u32>() {
                                            result.clock_rate = rate;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // a=ptime:20
        if line.starts_with("a=ptime:") {
            if let Some(val) = line.strip_prefix("a=ptime:") {
                if let Ok(ptime) = val.trim().parse::<u32>() {
                    if ptime > 0 && ptime <= 200 {
                        result.ptime_ms = ptime;
                    }
                }
            }
        }
    }

    // If no rtpmap matched, determine from PT alone
    if media_pt.is_some() && result.payload_type != media_pt.unwrap() {
        if let Some(pt) = media_pt {
            if let Some(c) = <CodecType as CodecTypeExt>::from_payload_type(pt) {
                result.codec = c;
                result.payload_type = pt;
                result.clock_rate = c.default_clock_rate();
            }
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_pcmu_silence() {
        let pcm = vec![0i16; 160];
        let encoded = CodecType::PCMU.encode(&pcm);
        let decoded = CodecType::PCMU.decode(&encoded);
        for s in &decoded {
            assert!(s.abs() < 10, "expected near-zero, got {}", s);
        }
    }

    #[test]
    fn roundtrip_pcma_silence() {
        let pcm = vec![0i16; 160];
        let encoded = CodecType::PCMA.encode(&pcm);
        let decoded = CodecType::PCMA.decode(&encoded);
        for s in &decoded {
            assert!(s.abs() < 16, "expected near-zero, got {}", s);
        }
    }

    #[test]
    fn parse_sdp_pcmu_default() {
        let sdp = "v=0\r\no=- 0 0 IN IP4 0.0.0.0\r\ns=-\r\nt=0 0\r\nm=audio 5004 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";
        let codec = parse_negotiated_codec(sdp);
        assert_eq!(codec.codec, CodecType::PCMU);
        assert_eq!(codec.payload_type, 0);
        assert_eq!(codec.clock_rate, 8000);
        assert_eq!(codec.ptime_ms, 20); // default
    }

    #[test]
    fn parse_sdp_pcma_with_ptime() {
        let sdp = "v=0\r\nm=audio 5004 RTP/AVP 8\r\na=rtpmap:8 PCMA/8000\r\na=ptime:30\r\n";
        let codec = parse_negotiated_codec(sdp);
        assert_eq!(codec.codec, CodecType::PCMA);
        assert_eq!(codec.payload_type, 8);
        assert_eq!(codec.ptime_ms, 30);
        assert_eq!(codec.frame_samples(), 240); // 8000 * 30 / 1000
    }

    #[test]
    fn parse_sdp_opus() {
        let sdp = "v=0\r\nm=audio 5004 RTP/AVP 111\r\na=rtpmap:111 opus/48000/2\r\na=ptime:20\r\n";
        let codec = parse_negotiated_codec(sdp);
        assert_eq!(codec.codec, CodecType::Opus);
        assert_eq!(codec.payload_type, 111);
        assert_eq!(codec.clock_rate, 48000);
        assert_eq!(codec.ptime_ms, 20);
        assert_eq!(codec.frame_samples(), 960); // 48000 * 20 / 1000
    }

    #[test]
    fn parse_sdp_g722() {
        let sdp = "v=0\r\nm=audio 5004 RTP/AVP 9\r\na=rtpmap:9 G722/16000\r\na=ptime:20\r\n";
        let codec = parse_negotiated_codec(sdp);
        assert_eq!(codec.codec, CodecType::G722);
        assert_eq!(codec.payload_type, 9);
        assert_eq!(codec.clock_rate, 16000);
        assert_eq!(codec.ptime_ms, 20);
        assert_eq!(codec.frame_samples(), 320); // 16000 * 20 / 1000
    }

    #[test]
    fn test_codec_extensions() {
        // Test from_payload_type
        assert_eq!(<CodecType as CodecTypeExt>::from_payload_type(0), Some(CodecType::PCMU));
        assert_eq!(<CodecType as CodecTypeExt>::from_payload_type(8), Some(CodecType::PCMA));
        assert_eq!(<CodecType as CodecTypeExt>::from_payload_type(9), Some(CodecType::G722));
        assert_eq!(<CodecType as CodecTypeExt>::from_payload_type(111), Some(CodecType::Opus));

        // Test to_payload_type
        assert_eq!(CodecType::PCMU.to_payload_type(), 0);
        assert_eq!(CodecType::PCMA.to_payload_type(), 8);
        assert_eq!(CodecType::G722.to_payload_type(), 9);
        assert_eq!(CodecType::Opus.to_payload_type(), 111);

        // Test default_clock_rate
        assert_eq!(CodecType::PCMU.default_clock_rate(), 8000);
        assert_eq!(CodecType::G722.default_clock_rate(), 16000);
        assert_eq!(CodecType::Opus.default_clock_rate(), 48000);
    }
}
