pub mod audio_bridge;
pub mod codec;

use rustrtc::config::MediaCapabilities;
use rustrtc::{
    AudioCapability, MediaKind, PeerConnection, RtcConfiguration, RtpCodecParameters, SdpType,
    SessionDescription, TransportMode,
};
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

/// 创建配置（使用 RTP 模式兼容传统 SIP PBX）
///
/// transport_mode 参数：
/// - TransportMode::Rtp: 原始 RTP，无 ICE/DTLS（兼容传统 PBX）
/// - TransportMode::Srtp: SDES SRTP 加密，不使用 DTLS
///
/// external_ip 参数：
/// - Some(ip): 通过手动 STUN 查询得到的公网 IP，rustrtc 会将其写入 SDP candidate 地址
/// - None: 仅使用本地 IP（无 NAT 环境）
/// 创建配置（RTP + ICE 模式，兼容传统 PBX 且支持 NAT 穿透）
///
/// 根据 RFC 8839，使用 RTP/AVP 协议 + ICE 来实现：
/// - 兼容传统 SIP PBX（明文 RTP，无加密）
/// - 支持 NAT 穿透（通过 STUN 获取公网 IP:port）
/// - 动态地址适应（RTP latching）
///
/// 工作原理：
/// 1. rustrtc 查询 STUN 服务器获取 server-reflexive candidates
/// 2. SDP 中包含：
///    - 协议：RTP/AVP（明文 RTP）
///    - ICE 属性：a=ice-ufrag, a=ice-pwd, a=candidate
///    - 正确的公网 IP 和 NAT 映射端口
fn create_rtp_ice_config(transport_mode: TransportMode) -> RtcConfiguration {
    info!(transport_mode = ?transport_mode, "Creating RTP+ICE config for NAT traversal");

    RtcConfiguration {
        transport_mode,
        ice_servers: vec![
            rustrtc::IceServer::new(vec!["stun:stun.l.google.com:19302".to_string()]),
            rustrtc::IceServer::new(vec!["stun:stun1.l.google.com:19302".to_string()]),
            rustrtc::IceServer::new(vec!["stun:restsend.com:3478".to_string()]),
            rustrtc::IceServer::new(vec!["stun:stun.voip.blackberry.com:3478".to_string()]),
        ],
        media_capabilities: Some(MediaCapabilities {
            audio: vec![
                AudioCapability::pcmu(),
                AudioCapability::pcma(),
                AudioCapability::g722(),
                AudioCapability::g729(),
                AudioCapability::opus(),
                AudioCapability::telephone_event(),
            ],
            video: vec![],
            application: None,
        }),
        enable_latching: true, // 启用 RTP latching
        // 注意：不设置 rtp_start_port/rtp_end_port，让操作系统动态分配端口
        // 这样 ICE gathering 可以正常工作
        ..Default::default()
    }
}

/// Replace SDP addresses with public IP:port from server-reflexive candidate
/// and remove ICE attributes (for non-ICE peers)
fn replace_with_public_address(sdp: &str, public_ip: &str, public_port: u16) -> String {
    let lines: Vec<&str> = sdp.lines().collect();
    let mut result = Vec::new();

    for line in lines {
        // Replace c= line
        if line.starts_with("c=IN IP4") {
            result.push(format!("c=IN IP4 {}", public_ip));
        }
        // Replace o= line IP
        else if line.starts_with("o=") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 6 {
                result.push(format!(
                    "{} {} {} {} {} {}",
                    parts[0], parts[1], parts[2], parts[3], parts[4], public_ip
                ));
            } else {
                result.push(line.to_string());
            }
        }
        // Replace m= line port
        else if line.starts_with("m=audio") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 3 {
                let rest = parts[2..].join(" ");
                result.push(format!("m=audio {} {}", public_port, rest));
            } else {
                result.push(line.to_string());
            }
        }
        // Fix direction: replace sendonly with sendrecv
        else if line.starts_with("a=sendonly") {
            result.push("a=sendrecv".to_string());
        }
        // Remove ICE-related attributes AND rtcp-mux (PBX doesn't support it)
        else if line.starts_with("a=ice-")
            || line.starts_with("a=candidate:")
            || line.starts_with("a=end-of-candidates")
            || line.starts_with("a=rtcp-mux")
        {
            // Skip ICE and RTCP-mux attributes
            continue;
        } else {
            result.push(line.to_string());
        }
    }

    result.join("\r\n") + "\r\n"
}

/// Inject fake ICE attributes into SDP offer to trick rustrtc into doing ICE gathering
fn inject_ice_attributes(sdp: &str) -> String {
    let mut lines: Vec<String> = sdp.lines().map(|s| s.to_string()).collect();

    // Find the m=audio line index
    let audio_idx = lines.iter().position(|l| l.starts_with("m=audio"));

    if let Some(idx) = audio_idx {
        // Insert fake ICE attributes after m=audio line
        lines.insert(idx + 1, "a=ice-ufrag:fake".to_string());
        lines.insert(idx + 2, "a=ice-pwd:fakefakefakefakefakefake".to_string());
    }

    lines.join("\r\n") + "\r\n"
}

/// 等待 RTP 连接建立后，启动音频采集和播放。
async fn start_audio(
    pc: &PeerConnection,
    audio_bridge: &mut AudioBridge,
    output_device: Option<&str>,
    negotiated: &NegotiatedCodec,
) -> Result<(), String> {
    info!("Waiting for RTP connection...");
    match tokio::time::timeout(std::time::Duration::from_secs(10), pc.wait_for_connected()).await {
        Ok(Ok(_)) => info!("RTP connection established"),
        Ok(Err(e)) => return Err(format!("Connection failed: {}", e)),
        Err(_) => return Err("Connection timed out".to_string()),
    }

    info!("Starting audio capture...");
    audio_bridge.start_capture(negotiated)?;

    let transceivers = pc.get_transceivers();
    info!(transceiver_count = transceivers.len(), "Got transceivers");
    for t in &transceivers {
        if t.kind() == MediaKind::Audio {
            info!("Found audio transceiver");
            if let Some(receiver) = t.receiver() {
                let remote_track = receiver.track();
                info!("Got remote track, starting playback...");
                audio_bridge.start_playback(output_device, remote_track, negotiated)?;
                info!("Audio playback started");
                break;
            } else {
                warn!("Audio transceiver has no receiver");
            }
        }
    }

    Ok(())
}

/// A WebRTC session wrapping a PeerConnection and audio bridge for one call.
pub struct WebRtcSession {
    pc: PeerConnection,
    audio_bridge: AudioBridge,
    closed: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl WebRtcSession {
    /// Create a new outbound session. Returns `(session, sdp_offer_string)`.
    ///
    /// This sets up:
    /// - PeerConnection with RTP+ICE mode (RFC 8839: RTP/AVP with ICE)
    /// - Audio track with all supported codecs
    /// - AudioBridge (capture NOT started yet — waits for SDP negotiation)
    ///
    /// NAT Traversal Flow:
    /// 1. Create offer (triggers ICE gathering)
    /// 2. Wait for STUN query to complete
    /// 3. Generate final offer with server-reflexive candidates (public IP:port)
    pub async fn new_outbound(
        input_device: Option<&str>,
        output_device: Option<&str>,
        prefer_srtp: bool,
    ) -> Result<(Self, String), String> {
        let transport_mode = if prefer_srtp {
            TransportMode::Srtp
        } else {
            TransportMode::Rtp
        };

        info!(
            srtp = prefer_srtp,
            "Creating outbound WebRTC session with ICE"
        );

        let pc = PeerConnection::new(create_rtp_ice_config(transport_mode));

        // Create audio bridge (validates devices, creates track, but does NOT start capture)
        let (audio_bridge, send_track) = AudioBridge::new(input_device, output_device)?;

        // Add the capture track to PeerConnection with PCMU codec parameters
        let params = RtpCodecParameters {
            payload_type: 0,
            clock_rate: 8000,
            channels: 1,
        };
        pc.add_track(send_track, params)
            .map_err(|e| format!("Failed to add audio track: {}", e))?;

        // Step 1: Create initial offer (triggers ICE gathering)
        info!("Creating initial offer to trigger ICE gathering...");
        let _initial_offer = pc
            .create_offer()
            .await
            .map_err(|e| format!("Failed to create initial offer: {}", e))?;

        // Step 2: Wait for ICE gathering to complete (STUN queries finish)
        info!("Waiting for ICE gathering to complete...");
        pc.wait_for_gathering_complete().await;

        // Step 3: Create final offer with all ICE candidates
        info!("Creating final offer with ICE candidates...");
        let offer = pc
            .create_offer()
            .await
            .map_err(|e| format!("Failed to create final offer: {}", e))?;

        let sdp_string = offer.to_sdp_string();

        let uses_srtp = detect_srtp_from_sdp(&sdp_string);
        info!(
            srtp = uses_srtp,
            sdp_len = sdp_string.len(),
            "SDP offer created with ICE candidates"
        );
        info!(sdp_offer = %sdp_string, "Local SDP offer content");

        // Verify we have ICE candidates
        let candidates = pc.ice_transport().local_candidates();
        let srflx_count = candidates
            .iter()
            .filter(|c| {
                matches!(
                    c.typ,
                    rustrtc::transports::ice::IceCandidateType::ServerReflexive
                )
            })
            .count();
        info!(
            total_candidates = candidates.len(),
            server_reflexive = srflx_count,
            "ICE candidates collected"
        );

        pc.set_local_description(offer)
            .map_err(|e| format!("Failed to set local description: {}", e))?;

        let session = WebRtcSession {
            pc,
            audio_bridge,
            closed: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        };

        info!("WebRTC outbound session created");
        Ok((session, sdp_string))
    }

    /// Create a new inbound session from an SDP offer. Returns `(session, sdp_answer_string)`.
    ///
    /// This sets up:
    /// - PeerConnection with RTP+ICE mode (RFC 8839: RTP/AVP with ICE)
    /// - Audio track with negotiated codec
    /// - AudioBridge (capture NOT started yet — waits for connection)
    ///
    /// NAT Traversal Flow (Standard Answerer mode):
    /// 1. Set remote description from incoming SDP offer
    /// 2. Create answer (triggers ICE gathering)
    /// 3. Wait for STUN queries to complete
    /// 4. Extract public IP:port from server-reflexive candidate
    /// 5. Build custom SDP answer string with public address (no ICE attributes for non-ICE peers)
    ///
    /// Note: We use standard Answerer mode to ensure proper WebRTC signaling state machine.
    pub async fn new_inbound(
        sdp_offer: &str,
        input_device: Option<&str>,
        output_device: Option<&str>,
    ) -> Result<(Self, String), String> {
        // Parse negotiated codec from SDP offer
        let negotiated = codec::parse_negotiated_codec(sdp_offer);

        // Auto-detect SRTP from remote SDP
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

        // Check if remote offer has ICE attributes
        let remote_has_ice = sdp_offer.contains("a=ice-ufrag") && sdp_offer.contains("a=ice-pwd");
        info!(
            remote_has_ice = remote_has_ice,
            "Checking remote ICE support"
        );

        let pc = PeerConnection::new(create_rtp_ice_config(transport_mode));

        // Create audio bridge (validates devices, creates track, but does NOT start capture)
        let (audio_bridge, send_track) = AudioBridge::new(input_device, output_device)?;

        // Add the capture track to PeerConnection with negotiated codec parameters
        let params = RtpCodecParameters {
            payload_type: negotiated.payload_type,
            clock_rate: negotiated.clock_rate,
            channels: 1,
        };
        pc.add_track(send_track, params)
            .map_err(|e| format!("Failed to add audio track: {}", e))?;

        // CRITICAL FIX: Set remote description FIRST before creating answer
        // This is required for proper WebRTC signaling state machine
        info!("Setting remote description from incoming SDP offer...");
        let remote_desc = if remote_has_ice {
            // Remote supports ICE, use original offer as-is
            SessionDescription::parse(SdpType::Offer, sdp_offer)
                .map_err(|e| format!("Failed to parse remote SDP offer: {}", e))?
        } else {
            // Remote doesn't support ICE, inject fake ICE attributes to trick rustrtc
            let offer_with_ice = inject_ice_attributes(sdp_offer);
            SessionDescription::parse(SdpType::Offer, &offer_with_ice)
                .map_err(|e| format!("Failed to parse modified SDP offer: {}", e))?
        };

        pc.set_remote_description(remote_desc)
            .await
            .map_err(|e| format!("Failed to set remote description: {}", e))?;

        info!("Remote description set, now creating answer with ICE gathering...");

        // Step 1: Create initial answer (triggers ICE gathering)
        info!("Creating initial answer to trigger ICE gathering...");
        let _initial_answer = pc
            .create_answer()
            .await
            .map_err(|e| format!("Failed to create initial answer: {}", e))?;

        // Step 2: Wait for ICE gathering to complete (STUN queries finish)
        info!("Waiting for ICE gathering to complete...");
        let start = std::time::Instant::now();
        pc.wait_for_gathering_complete().await;
        let duration = start.elapsed();
        info!("ICE gathering completed in {:?}", duration);

        // Step 3: Create final answer with all ICE candidates
        info!("Creating final answer with ICE candidates...");
        let answer = pc
            .create_answer()
            .await
            .map_err(|e| format!("Failed to create final answer: {}", e))?;

        // Step 4: Set local description (establishes RTP socket bindings)
        pc.set_local_description(answer.clone())
            .map_err(|e| format!("Failed to set local description: {}", e))?;

        let offer_sdp = answer.to_sdp_string();

        // Step 5: Extract server-reflexive candidate (public IP:port)
        let candidates = pc.ice_transport().local_candidates();
        let srflx_count = candidates
            .iter()
            .filter(|c| {
                matches!(
                    c.typ,
                    rustrtc::transports::ice::IceCandidateType::ServerReflexive
                )
            })
            .count();
        info!(
            total_candidates = candidates.len(),
            server_reflexive = srflx_count,
            "ICE candidates collected"
        );

        let public_addr = candidates
            .iter()
            .find(|c| {
                matches!(
                    c.typ,
                    rustrtc::transports::ice::IceCandidateType::ServerReflexive
                )
            })
            .map(|c| {
                let ip = c.address.ip().to_string();
                let port = c.address.port();
                info!(public_ip = %ip, public_port = port, "Found server-reflexive candidate");
                (ip, port)
            });

        // Step 6: Build SDP answer string
        let final_sdp = if !remote_has_ice {
            if let Some((public_ip, public_port)) = public_addr {
                info!(public_ip = %public_ip, public_port = public_port, "Building SDP answer with public address");
                // Use the offer SDP as template and replace with public address
                replace_with_public_address(&offer_sdp, &public_ip, public_port)
            } else {
                warn!("No public address found, using offer SDP with internal address");
                // Remove ICE attributes even if we don't have public address
                let lines: Vec<&str> = offer_sdp.lines().collect();
                let mut result = Vec::new();
                for line in lines {
                    if line.starts_with("a=ice-")
                        || line.starts_with("a=candidate:")
                        || line.starts_with("a=end-of-candidates")
                        || line.starts_with("a=rtcp-mux")
                    {
                        continue;
                    }
                    if line.starts_with("a=sendonly") {
                        result.push("a=sendrecv".to_string());
                    } else {
                        result.push(line.to_string());
                    }
                }
                result.join("\r\n") + "\r\n"
            }
        } else {
            // Remote supports ICE, use normal offer SDP
            offer_sdp
        };

        info!(sdp_len = final_sdp.len(), "SDP answer created");
        debug!(sdp_answer = %final_sdp, "Local SDP answer content");

        let session = WebRtcSession {
            pc,
            audio_bridge,
            closed: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        };

        info!("WebRTC inbound session created with Answerer mode");
        Ok((session, final_sdp))
    }

    /// Start audio capture early (before sending 200 OK) to trigger NAT mapping.
    /// This allows RTP packets to be sent before PBX starts sending, ensuring NAT works.
    pub async fn start_inbound_media_early(&mut self, sdp_offer: &str) -> Result<(), String> {
        // Parse negotiated codec from SDP offer
        let negotiated = codec::parse_negotiated_codec(sdp_offer);

        info!("Starting audio capture early (before 200 OK)...");

        // Start capture immediately to send RTP packets and establish NAT mapping
        self.audio_bridge.start_capture(&negotiated)?;
        info!("Audio capture started, RTP packets being sent");

        Ok(())
    }

    /// Start playback after 200 OK has been sent.
    /// Call this after start_inbound_media_early() and after sending 200 OK.
    pub async fn start_inbound_playback(
        &mut self,
        sdp_offer: &str,
        output_device: Option<&str>,
    ) -> Result<(), String> {
        // Parse negotiated codec from SDP offer
        let negotiated = codec::parse_negotiated_codec(sdp_offer);

        info!("Waiting for RTP connection...");
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

        let transceivers = self.pc.get_transceivers();
        info!(transceiver_count = transceivers.len(), "Got transceivers");
        for t in &transceivers {
            if t.kind() == MediaKind::Audio {
                info!("Found audio transceiver");
                if let Some(receiver) = t.receiver() {
                    let remote_track = receiver.track();
                    info!("Got remote track, starting playback...");
                    self.audio_bridge
                        .start_playback(output_device, remote_track, &negotiated)?;
                    info!("Audio playback started");
                    break;
                } else {
                    warn!("Audio transceiver has no receiver");
                }
            }
        }

        Ok(())
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

        info!(
            srtp = remote_uses_srtp,
            "Remote SDP answer applied, waiting for connection..."
        );

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
    pub async fn close(&mut self) {
        // Check if already closed to prevent double-close
        if self.closed.swap(true, std::sync::atomic::Ordering::SeqCst) {
            debug!("WebRTC session already closed, skipping");
            return;
        }

        info!("Closing WebRTC session");

        // Step 1: Close audio first (unrelated to ICE)
        debug!("Closing audio bridge...");
        self.audio_bridge.close();

        // Step 2: Close PeerConnection (ICE transport will be closed)
        debug!("Closing PeerConnection (ICE transport will be closed)...");
        self.pc.close();

        debug!("WebRTC session closed");
    }
}

impl Drop for WebRtcSession {
    fn drop(&mut self) {
        // Only close if not already closed
        if self.closed.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return;
        }

        // Synchronous cleanup: close audio and PeerConnection
        // Note: async cleanup in close() method is preferred when possible
        info!("Dropping WebRTC session");
        self.audio_bridge.close();
        self.pc.close();

        // Can't await in Drop, so synchronous close may still cause ICE warnings
        // Always call close().await explicitly before dropping when possible
    }
}
