pub mod audio_bridge;
pub mod codec;

use rustrtc::config::MediaCapabilities;
use rustrtc::{AudioCapability, IceServer, MediaKind, PeerConnection, RtcConfiguration, RtpCodecParameters, SdpType, SessionDescription, TransportMode};
use tracing::{debug, info, warn};

use audio_bridge::AudioBridge;
use codec::NegotiatedCodec;

/// 检测 SDP 是否包含 SRTP 相关属性（使用 rustrtc 的标准 SDP 解析 API）
///
/// 检查项：
/// 1. SDES crypto 属性 (a=crypto:1 AES_CM_128_HMAC_SHA1_80 ...)
/// 2. DTLS fingerprint 属性 (a=fingerprint:sha-256 ...)
/// 3. Media protocol 字段包含 SAVP (RTP/SAVP 或 UDP/TLS/RTP/SAVPF)
fn detect_srtp_from_sdp(sdp: &str) -> bool {
    // 尝试解析 SDP（sdp_type 参数在这里不重要，只为结构化解析）
    // 使用 Offer 类型作为默认值，因为我们只检查属性，不依赖 sdp_type 逻辑
    let desc = match SessionDescription::parse(SdpType::Offer, sdp) {
        Ok(d) => d,
        Err(e) => {
            warn!(error = ?e, "Failed to parse SDP for SRTP detection, assuming RTP");
            return false;
        }
    };

    // 检查所有 media section
    for section in &desc.media_sections {
        // 方法 1：检查 crypto 属性（SDES SRTP）
        let crypto_attrs = section.get_crypto_attributes();
        if !crypto_attrs.is_empty() {
            debug!(count = crypto_attrs.len(), "Found SDES crypto attributes");
            return true;
        }

        // 方法 2：检查 fingerprint 属性（DTLS/SRTP）
        for attr in &section.attributes {
            if attr.key == "fingerprint" {
                debug!(fingerprint = ?attr.value, "Found DTLS fingerprint");
                return true;
            }
        }

        // 方法 3：检查 protocol 字段包含 SAVP（表示 Secure Audio/Video Profile）
        if section.protocol.contains("SAVP") {
            debug!(protocol = %section.protocol, "Found SRTP protocol in media line");
            return true;
        }
    }

    false
}

/// Build RFC 4733 telephone-event RTP payload (4 bytes).
///
/// Format:
/// ```
///  0                   1                   2                   3
///  0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
/// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
/// |     event     |E|R| volume    |          duration             |
/// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
/// ```
fn build_dtmf_payload(event: u8, end: u8, volume: u8, duration: u16) -> Vec<u8> {
    let mut payload = Vec::with_capacity(4);
    payload.push(event); // Byte 0: event code (0-15)
    payload.push((end << 7) | (volume & 0x3F)); // Byte 1: E(1) + R(1) + volume(6)
    payload.push((duration >> 8) as u8); // Byte 2: duration high byte
    payload.push((duration & 0xFF) as u8); // Byte 3: duration low byte
    payload
}

/// 创建 SIP/RTP 模式的 PeerConnection 配置（含所有支持的音频编解码器和 STUN 服务器）。
///
/// transport_mode 参数：
/// - TransportMode::Rtp: 原始 RTP，无加密（默认）
/// - TransportMode::Srtp: 启用 SDES SRTP 加密（仅在对端要求时使用）
fn create_rtp_config(transport_mode: TransportMode) -> RtcConfiguration {
    let mut config = RtcConfiguration {
        transport_mode,
        media_capabilities: Some(MediaCapabilities {
            audio: vec![
                AudioCapability::opus(),
                AudioCapability::g722(),
                AudioCapability::pcmu(),
                AudioCapability::pcma(),
                AudioCapability::g729(),
                AudioCapability::telephone_event(),
            ],
            video: vec![],
            application: None,
        }),
        enable_latching: true,
        ..Default::default()
    };
    config.ice_servers.push(IceServer::new(vec!["stun:stun.l.google.com:19302".to_string()]));
    config
}

/// 等待 RTP 连接建立后，启动音频采集和播放。
async fn start_audio(
    pc: &PeerConnection,
    audio_bridge: &mut AudioBridge,
    output_device: Option<&str>,
    negotiated: &NegotiatedCodec,
) -> Result<(), String> {
    match tokio::time::timeout(
        std::time::Duration::from_secs(10),
        pc.wait_for_connected(),
    )
    .await
    {
        Ok(Ok(_)) => info!("RTP connection established"),
        Ok(Err(e)) => return Err(format!("Connection failed: {}", e)),
        Err(_) => return Err("Connection timed out".to_string()),
    }

    audio_bridge.start_capture(negotiated)?;

    let transceivers = pc.get_transceivers();
    for t in &transceivers {
        if t.kind() == MediaKind::Audio {
            if let Some(receiver) = t.receiver() {
                let remote_track = receiver.track();
                audio_bridge.start_playback(output_device, remote_track, negotiated)?;
                info!("Audio playback started");
                break;
            }
        }
    }

    Ok(())
}

/// A WebRTC session wrapping a PeerConnection and audio bridge for one call.
pub struct WebRtcSession {
    pc: PeerConnection,
    audio_bridge: AudioBridge,
}

impl WebRtcSession {
    /// Create a new outbound session. Returns `(session, sdp_offer_string)`.
    ///
    /// This sets up:
    /// - PeerConnection with configurable RTP/SRTP mode
    /// - Audio track with PCMU/PCMA codecs
    /// - AudioBridge (capture NOT started yet — waits for SDP negotiation)
    ///
    /// Parameters:
    /// - `prefer_srtp`: 默认为 false (RTP)。仅在对端 SDP 中要求 SRTP 时传入 true。
    pub async fn new_outbound(
        input_device: Option<&str>,
        output_device: Option<&str>,
        prefer_srtp: bool,
    ) -> Result<(Self, String), String> {
        // Choose transport mode based on preference
        let transport_mode = if prefer_srtp {
            TransportMode::Srtp
        } else {
            TransportMode::Rtp
        };

        let pc = PeerConnection::new(create_rtp_config(transport_mode));

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

        let uses_srtp = detect_srtp_from_sdp(&sdp_string);
        info!(srtp = uses_srtp, sdp_len = sdp_string.len(), "SDP offer created");

        pc.set_local_description(offer)
            .map_err(|e| format!("Failed to set local description: {}", e))?;

        let session = WebRtcSession { pc, audio_bridge };

        info!("WebRTC outbound session created");
        Ok((session, sdp_string))
    }

    /// Create a new inbound session from an SDP offer. Returns `(session, sdp_answer_string)`.
    ///
    /// This sets up:
    /// - 自动从 SDP offer 中检测对端是否要求 SRTP (检查 crypto/fingerprint 属性)
    /// - 如果对端要求 SRTP 则使用 SRTP 模式,否则使用 RTP 模式
    /// - Audio track with negotiated codec
    /// - AudioBridge (capture NOT started yet — waits for connection)
    pub async fn new_inbound(
        sdp_offer: &str,
        input_device: Option<&str>,
        output_device: Option<&str>,
    ) -> Result<(Self, String), String> {
        // Parse negotiated codec from SDP offer
        let negotiated = codec::parse_negotiated_codec(sdp_offer);

        // Auto-detect SRTP support from remote SDP
        let uses_srtp = detect_srtp_from_sdp(sdp_offer);
        let transport_mode = if uses_srtp {
            TransportMode::Srtp
        } else {
            TransportMode::Rtp
        };

        info!(
            codec = ?negotiated.codec,
            pt = negotiated.payload_type,
            rate = negotiated.clock_rate,
            ptime = negotiated.ptime_ms,
            srtp = uses_srtp,
            "Parsed codec from incoming SDP offer"
        );

        let pc = PeerConnection::new(create_rtp_config(transport_mode));

        // Create audio bridge (validates devices, creates track, but does NOT start capture)
        let (audio_bridge, send_track) =
            AudioBridge::new(input_device, output_device)?;

        // Add the capture track to PeerConnection with negotiated codec parameters
        let params = RtpCodecParameters {
            payload_type: negotiated.payload_type,
            clock_rate: negotiated.clock_rate,
            channels: 1,
        };
        pc.add_track(send_track, params)
            .map_err(|e| format!("Failed to add audio track: {}", e))?;

        // Parse and set remote description (offer)
        let offer = rustrtc::SessionDescription::parse(rustrtc::SdpType::Offer, sdp_offer)
            .map_err(|e| format!("Failed to parse SDP offer: {}", e))?;

        pc.set_remote_description(offer)
            .await
            .map_err(|e| format!("Failed to set remote description: {}", e))?;

        // Create SDP answer
        let answer = pc
            .create_answer()
            .await
            .map_err(|e| format!("Failed to create SDP answer: {}", e))?;

        let sdp_answer_string = answer.to_sdp_string();
        debug!(sdp_len = sdp_answer_string.len(), "SDP answer created");

        pc.set_local_description(answer)
            .map_err(|e| format!("Failed to set local description: {}", e))?;

        let session = WebRtcSession { pc, audio_bridge };

        info!(srtp = uses_srtp, "WebRTC inbound session created");
        Ok((session, sdp_answer_string))
    }

    /// Start media for an inbound session (after SDP negotiation complete).
    /// This should be called after new_inbound and after sending the SDP answer.
    pub async fn start_inbound_media(
        &mut self,
        sdp_offer: &str,
        output_device: Option<&str>,
    ) -> Result<(), String> {
        // Parse negotiated codec from SDP offer
        let negotiated = codec::parse_negotiated_codec(sdp_offer);

        info!("Starting inbound media, waiting for connection...");

        start_audio(&self.pc, &mut self.audio_bridge, output_device, &negotiated).await
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

        // Check if remote supports SRTP
        let remote_uses_srtp = detect_srtp_from_sdp(sdp_answer);

        info!(
            codec = ?negotiated.codec,
            pt = negotiated.payload_type,
            rate = negotiated.clock_rate,
            ptime = negotiated.ptime_ms,
            srtp = remote_uses_srtp,
            "Negotiated codec from SDP answer"
        );

        let answer = rustrtc::SessionDescription::parse(rustrtc::SdpType::Answer, sdp_answer)
            .map_err(|e| format!("Failed to parse SDP answer: {}", e))?;

        self.pc
            .set_remote_description(answer)
            .await
            .map_err(|e| format!("Failed to set remote description: {}", e))?;

        info!(srtp = remote_uses_srtp, "Remote SDP answer applied, waiting for connection...");

        start_audio(&self.pc, &mut self.audio_bridge, output_device, &negotiated).await
    }

    /// Toggle microphone mute. Returns new mute state.
    pub fn toggle_mic_mute(&self) -> bool {
        self.audio_bridge.toggle_mic_mute()
    }

    /// Toggle speaker mute. Returns new mute state.
    pub fn toggle_speaker_mute(&self) -> bool {
        self.audio_bridge.toggle_speaker_mute()
    }

    /// Send DTMF digit (0-9, *, #, A-D) via RFC 4733 telephone-event.
    pub async fn send_dtmf(&self, digit: char) -> Result<(), String> {
        // Map digit to event code (RFC 4733)
        let event_code = match digit {
            '0' => 0,
            '1' => 1,
            '2' => 2,
            '3' => 3,
            '4' => 4,
            '5' => 5,
            '6' => 6,
            '7' => 7,
            '8' => 8,
            '9' => 9,
            '*' => 10,
            '#' => 11,
            'A' | 'a' => 12,
            'B' | 'b' => 13,
            'C' | 'c' => 14,
            'D' | 'd' => 15,
            _ => return Err(format!("Invalid DTMF digit: {}", digit)),
        };

        info!(digit = %digit, event_code = event_code, "Sending DTMF");

        // Get the audio transceiver
        let transceivers = self.pc.get_transceivers();
        let _audio_transceiver = transceivers
            .iter()
            .find(|t| t.kind() == MediaKind::Audio)
            .ok_or("No audio transceiver found")?;

        // TODO: Implement actual RTP packet sending
        // rustrtc may not expose direct RTP transport sending API
        // Options:
        // 1. Use rustrtc's DTMFSender if available
        // 2. Add rtp-rs dependency and send via raw socket
        // 3. Wait for rustrtc API updates

        // For now, simulate DTMF by logging
        // Duration: 160ms (1280 timestamp units at 8kHz), send 8 packets (20ms each)
        const PACKET_DURATION: u16 = 160; // 20ms at 8kHz
        const TOTAL_PACKETS: usize = 8;
        const VOLUME: u8 = 10; // 0 = loudest, 63 = silence

        for i in 0..TOTAL_PACKETS {
            let duration = PACKET_DURATION * (i as u16 + 1);
            let end_bit = if i >= TOTAL_PACKETS - 3 { 1 } else { 0 }; // Mark last 3 packets as End

            // Build telephone-event payload (4 bytes)
            let _payload = build_dtmf_payload(event_code, end_bit, VOLUME, duration);

            // TODO: Send RTP packet with payload type 101
            // Currently blocked by rustrtc API limitations

            // Wait 20ms before next packet
            tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
        }

        info!(digit = %digit, "DTMF simulation completed (actual RTP sending not yet implemented)");
        Ok(())
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
