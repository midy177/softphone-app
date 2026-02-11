/// G.711 PCMU (μ-law) and PCMA (A-law) encoder/decoder per ITU-T G.711.

const ULAW_BIAS: i16 = 0x84;
const ULAW_CLIP: i16 = 32635;

// ── PCMU (μ-law) ──

fn linear_to_ulaw(mut sample: i16) -> u8 {
    let sign: u8 = if sample < 0 {
        sample = -sample;
        0x80
    } else {
        0
    };

    if sample > ULAW_CLIP {
        sample = ULAW_CLIP;
    }
    sample += ULAW_BIAS;

    let exponent = match sample {
        s if s <= 0x00FF => 0u8,
        s if s <= 0x01FF => 1,
        s if s <= 0x03FF => 2,
        s if s <= 0x07FF => 3,
        s if s <= 0x0FFF => 4,
        s if s <= 0x1FFF => 5,
        s if s <= 0x3FFF => 6,
        _ => 7,
    };

    let mantissa = ((sample >> (exponent + 3)) & 0x0F) as u8;
    !(sign | (exponent << 4) | mantissa)
}

fn ulaw_to_linear(sample: u8) -> i16 {
    let sample = !sample;
    let sign = sample & 0x80;
    let exponent = ((sample >> 4) & 0x07) as i16;
    let mantissa = (sample & 0x0F) as i16;

    let mut magnitude = ((mantissa << 1) | 0x21) << (exponent + 2);
    magnitude -= ULAW_BIAS;

    if sign != 0 {
        -magnitude
    } else {
        magnitude
    }
}

pub fn pcmu_encode(pcm: &[i16]) -> Vec<u8> {
    pcm.iter().map(|&s| linear_to_ulaw(s)).collect()
}

pub fn pcmu_decode(ulaw: &[u8]) -> Vec<i16> {
    ulaw.iter().map(|&s| ulaw_to_linear(s)).collect()
}

// ── PCMA (A-law) ──

fn linear_to_alaw(mut sample: i16) -> u8 {
    let sign_mask: u8 = if sample >= 0 { 0xD5 } else { 0x55 };
    if sample < 0 {
        sample = -sample;
    }
    if sample > 32767 {
        sample = 32767;
    }

    let (exponent, mantissa) = if sample < 256 {
        (0u8, (sample >> 4) as u8)
    } else {
        let exp = match sample {
            s if s < 512 => 1u8,
            s if s < 1024 => 2,
            s if s < 2048 => 3,
            s if s < 4096 => 4,
            s if s < 8192 => 5,
            s if s < 16384 => 6,
            _ => 7,
        };
        (exp, (sample >> (exp + 3)) as u8 & 0x0F)
    };

    (sign_mask ^ ((exponent << 4) | mantissa)) as u8
}

fn alaw_to_linear(alaw: u8) -> i16 {
    let val = alaw ^ 0xD5;
    let sign = val & 0x80;
    let exponent = ((val >> 4) & 0x07) as i16;
    let mantissa = (val & 0x0F) as i16;

    let magnitude = if exponent == 0 {
        (mantissa << 4) | 0x08
    } else {
        ((mantissa << 1) | 0x21) << (exponent + 2)
    };

    if sign != 0 {
        magnitude
    } else {
        -magnitude
    }
}

pub fn pcma_encode(pcm: &[i16]) -> Vec<u8> {
    pcm.iter().map(|&s| linear_to_alaw(s)).collect()
}

pub fn pcma_decode(alaw: &[u8]) -> Vec<i16> {
    alaw.iter().map(|&s| alaw_to_linear(s)).collect()
}

// ── Codec type for negotiated codec ──

/// Supported codec types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodecType {
    Pcmu, // G.711 μ-law, PT=0
    Pcma, // G.711 A-law, PT=8
}

impl CodecType {
    /// Determine codec from RTP payload type
    pub fn from_payload_type(pt: u8) -> Option<Self> {
        match pt {
            0 => Some(Self::Pcmu),
            8 => Some(Self::Pcma),
            _ => None,
        }
    }

    pub fn encode(&self, pcm: &[i16]) -> Vec<u8> {
        match self {
            Self::Pcmu => pcmu_encode(pcm),
            Self::Pcma => pcma_encode(pcm),
        }
    }

    pub fn decode(&self, data: &[u8]) -> Vec<i16> {
        match self {
            Self::Pcmu => pcmu_decode(data),
            Self::Pcma => pcma_decode(data),
        }
    }
}

impl std::fmt::Display for CodecType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pcmu => write!(f, "PCMU"),
            Self::Pcma => write!(f, "PCMA"),
        }
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
            codec: CodecType::Pcmu,
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

        // m=audio 5004 RTP/AVP 0 8
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

        // a=rtpmap:0 PCMU/8000
        if line.starts_with("a=rtpmap:") {
            if let Some(rest) = line.strip_prefix("a=rtpmap:") {
                let parts: Vec<&str> = rest.splitn(2, ' ').collect();
                if parts.len() == 2 {
                    if let Ok(pt) = parts[0].parse::<u8>() {
                        let codec_parts: Vec<&str> = parts[1].split('/').collect();
                        if let Some(&codec_name) = codec_parts.first() {
                            let codec = match codec_name.to_uppercase().as_str() {
                                "PCMU" => Some(CodecType::Pcmu),
                                "PCMA" => Some(CodecType::Pcma),
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
            if let Some(c) = CodecType::from_payload_type(pt) {
                result.codec = c;
                result.payload_type = pt;
                result.clock_rate = 8000; // G.711 is always 8kHz
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
        let encoded = pcmu_encode(&pcm);
        let decoded = pcmu_decode(&encoded);
        for s in &decoded {
            assert!(s.abs() < 10, "expected near-zero, got {}", s);
        }
    }

    #[test]
    fn roundtrip_pcma_silence() {
        let pcm = vec![0i16; 160];
        let encoded = pcma_encode(&pcm);
        let decoded = pcma_decode(&encoded);
        for s in &decoded {
            assert!(s.abs() < 16, "expected near-zero, got {}", s);
        }
    }

    #[test]
    fn parse_sdp_pcmu_default() {
        let sdp = "v=0\r\no=- 0 0 IN IP4 0.0.0.0\r\ns=-\r\nt=0 0\r\nm=audio 5004 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";
        let codec = parse_negotiated_codec(sdp);
        assert_eq!(codec.codec, CodecType::Pcmu);
        assert_eq!(codec.payload_type, 0);
        assert_eq!(codec.clock_rate, 8000);
        assert_eq!(codec.ptime_ms, 20); // default
    }

    #[test]
    fn parse_sdp_pcma_with_ptime() {
        let sdp = "v=0\r\nm=audio 5004 RTP/AVP 8\r\na=rtpmap:8 PCMA/8000\r\na=ptime:30\r\n";
        let codec = parse_negotiated_codec(sdp);
        assert_eq!(codec.codec, CodecType::Pcma);
        assert_eq!(codec.payload_type, 8);
        assert_eq!(codec.ptime_ms, 30);
        assert_eq!(codec.frame_samples(), 240); // 8000 * 30 / 1000
    }
}
