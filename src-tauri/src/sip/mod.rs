use crate::sip::helpers::{
    create_transport_connection, extract_protocol_from_uri, get_first_non_loopback_interface,
};
use crate::sip::message_inspector::SipFlow;
use crate::sip::state::{ActiveCall, PendingCall, SipClientHandle};
use dashmap::DashMap;
use rsip::Uri;
use rsipstack::dialog::authenticate::Credential;
use rsipstack::dialog::dialog_layer::DialogLayer;
use rsipstack::dialog::invitation::InviteOption;
use rsipstack::dialog::registration::Registration;
use rsipstack::transport::TransportLayer;
use rsipstack::EndpointBuilder;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info};
use uuid::Uuid;

use crate::webrtc::WebRtcSession;

mod coming_request;
mod dialog;
mod helpers;
mod make_call;
pub mod message_inspector;
mod registration;
pub mod state;

pub struct SipClient;

impl SipClient {
    /// Connect to SIP server, perform registration, and return a handle for making calls.
    ///
    /// # Parameters
    /// - `enable_sip_flow`: 是否启用 SIP 消息流记录 (默认 true)
    /// - `sip_flow_log_dir`: SIP 消息流日志目录 (默认 "logs")
    pub async fn connect(
        app_handle: AppHandle,
        server: String,
        username: String,
        password: String,
        outbound_proxy: Option<String>,
        enable_sip_flow: Option<bool>,
        sip_flow_log_dir: Option<String>,
    ) -> rsipstack::Result<(SipClientHandle, CancellationToken)> {
        // Parse server URI
        let server_uri_str = if server.starts_with("sip:") || server.starts_with("sips:") {
            server
        } else {
            format!("sip:{}", server)
        };
        let server_uri = Uri::try_from(server_uri_str)
            .map_err(|e| rsipstack::Error::Error(format!("Invalid server URI: {:?}", e)))?;

        // Parse outbound proxy
        let outbound_proxy_uri = if let Some(proxy) = outbound_proxy {
            let proxy_str = if proxy.starts_with("sip:") || proxy.starts_with("sips:") {
                proxy
            } else {
                format!("sip:{}", proxy)
            };
            Some(
                Uri::try_from(proxy_str)
                    .map_err(|e| rsipstack::Error::Error(format!("Invalid proxy URI: {:?}", e)))?,
            )
        } else {
            None
        };

        info!(
            server = %server_uri,
            username = %username,
            "SIP client connecting"
        );

        let cancel_token = CancellationToken::new();

        // Get local IP
        let local_ip = get_first_non_loopback_interface()?;
        debug!(ip = %local_ip, "Detected local outbound IP");

        // Create transport layer
        let mut transport_layer = TransportLayer::new(cancel_token.clone());

        // Determine protocol
        let (protocol, target_sip_addr) = if let Some(ref proxy) = outbound_proxy_uri {
            let protocol = extract_protocol_from_uri(proxy);
            (
                protocol,
                rsipstack::transport::SipAddr {
                    r#type: Some(protocol.into()),
                    addr: server_uri.host_with_port.clone(),
                },
            )
        } else {
            let protocol = extract_protocol_from_uri(&server_uri);
            (
                protocol,
                rsipstack::transport::SipAddr {
                    r#type: Some(protocol.into()),
                    addr: server_uri.host_with_port.clone(),
                },
            )
        };

        debug!(protocol = %protocol.as_str(), target = %target_sip_addr.addr, "Transport protocol selected");

        // Configure outbound proxy
        if let Some(ref proxy) = outbound_proxy_uri {
            let sip_addr = rsipstack::transport::SipAddr {
                r#type: Some(protocol.into()),
                addr: proxy.host_with_port.clone(),
            };
            transport_layer.outbound = Some(sip_addr);
            info!(proxy = %proxy.host_with_port, "Outbound proxy configured");
        }

        // Create transport connection
        let local_addr: SocketAddr = format!("{}:0", local_ip).parse()?;
        let connection =
            create_transport_connection(local_addr, target_sip_addr, cancel_token.clone()).await?;
        // UDP is a listening transport (add_transport/add_listener).
        // TCP/TLS/WS are outbound connections: must use add_connection so that
        // rsipstack's transport_layer lookup() can find them in the connections
        // HashMap and reuse them (instead of auto-creating a new TLS connection
        // without our custom certificate verifier).
        match protocol {
            crate::sip::helpers::Protocol::Udp => transport_layer.add_transport(connection),
            _ => transport_layer.add_connection(connection),
        }

        // Create SIP flow inspector
        let enable_flow = enable_sip_flow.unwrap_or(false); // 默认关闭
        let sip_flow = Arc::new(SipFlow::new(sip_flow_log_dir.as_deref(), enable_flow));

        // Create endpoint with SIP flow inspector
        let endpoint = EndpointBuilder::new()
            .with_cancel_token(cancel_token.clone())
            .with_transport_layer(transport_layer)
            .with_user_agent("softphone-app/0.1.0")
            .with_inspector(Box::new(sip_flow.as_ref().clone()))
            .build();

        let credential = Credential {
            username: username.clone(),
            password: password.clone(),
            realm: None,
        };

        let incoming = endpoint.incoming_transactions()?;
        let dialog_layer = Arc::new(DialogLayer::new(endpoint.inner.clone()));
        let (state_sender, state_receiver) = dialog_layer.new_dialog_state_channel();

        let first_addr = endpoint
            .get_addrs()
            .first()
            .ok_or(rsipstack::Error::Error("no address found".to_string()))?
            .clone();

        info!(address = %first_addr.addr, username = %username, "SIP client ready");

        let contact = rsip::Uri {
            scheme: Some(rsip::Scheme::Sip),
            auth: Some(rsip::Auth {
                user: username.clone(),
                password: None,
            }),
            host_with_port: first_addr.addr.into(),
            ..Default::default()
        };

        // Save endpoint inner ref before moving endpoint
        let endpoint_inner = endpoint.inner.clone();

        // Spawn background tasks BEFORE registration (endpoint.serve() must run to receive responses)
        let mut tasks = Vec::new();

        // Initialize pending_incoming HashMap, active_call, and call cancellation tokens
        let pending_incoming = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let active_call = Arc::new(tokio::sync::Mutex::new(None));
        let active_call_tokens = Arc::new(DashMap::new());

        // Task 1: endpoint.serve()
        tasks.push(tokio::spawn(async move {
            let _ = endpoint.serve().await;
            info!("Endpoint service stopped");
        }));

        // Task 2: process_incoming_request
        let dl = dialog_layer.clone();
        let ss = state_sender.clone();
        let ct = contact.clone();
        let ah = app_handle.clone();
        let pi = pending_incoming.clone();
        let ac = active_call.clone();
        tasks.push(tokio::spawn(async move {
            if let Err(e) =
                coming_request::process_incoming_request(dl, incoming, ss, ct, ah, pi, ac).await
            {
                error!(error = ?e, "Incoming request loop error");
            }
        }));

        // Task 3: process_dialog (with app_handle for event emission and call tokens for cleanup)
        let dl = dialog_layer.clone();
        let ah = app_handle.clone();
        let tokens = active_call_tokens.clone();
        tasks.push(tokio::spawn(async move {
            if let Err(e) = dialog::process_dialog(dl, state_receiver, ah, tokens).await {
                error!(error = ?e, "Dialog loop error");
            }
        }));

        // Perform initial registration (after endpoint.serve() is running)
        let mut reg = Registration::new(endpoint_inner.clone(), Some(credential.clone()));
        let initial_expires = registration::register_once(&mut reg, server_uri.clone()).await?;

        // Emit registration success event
        let _ = app_handle.emit(
            "sip://registration-status",
            state::RegistrationStatusPayload {
                status: "registered".to_string(),
                message: None,
            },
        );

        // Task 4: registration refresh loop
        let cred = credential.clone();
        let srv = server_uri.clone();
        let ct = cancel_token.clone();
        tasks.push(tokio::spawn(async move {
            if let Err(e) = registration::registration_refresh_loop(
                endpoint_inner,
                srv,
                cred,
                initial_expires,
                ct,
            )
            .await
            {
                error!(error = ?e, "Registration refresh loop error");
            }
        }));

        Ok((
            SipClientHandle {
                app_handle,
                dialog_layer,
                state_sender,
                contact,
                credential,
                server: server_uri,
                active_call,
                pending_incoming,
                active_call_tokens,
                sip_flow: Some(sip_flow),
                _tasks: tasks,
            },
            cancel_token,
        ))
    }
}

/// Make an outbound call using the SipClientHandle
pub async fn handle_make_call(
    handle: &SipClientHandle,
    callee: String,
    input_device: Option<String>,
    output_device: Option<String>,
    global_cancel_token: CancellationToken,
) -> rsipstack::Result<()> {
    let call_id = Uuid::new_v4().to_string();

    info!(call_id = %call_id, callee = %callee, "Making outbound call");

    let callee_uri = Uri {
        scheme: Some(rsip::Scheme::Sip),
        auth: Some(rsip::Auth {
            user: callee.clone(),
            password: None,
        }),
        host_with_port: handle.server.host_with_port.clone(),
        // Preserve transport params (e.g. transport=TCP) so rsipstack uses the correct connection
        params: handle.server.params.clone(),
        ..Default::default()
    };

    let invite_option = InviteOption {
        callee: callee_uri,
        caller: handle.contact.clone(),
        contact: handle.contact.clone(),
        credential: Some(handle.credential.clone()),
        call_id: Some(call_id.clone()),
        ..Default::default()
    };

    // 外呼不需要 STUN 映射：PBX 会根据我们发送的 RTP 源地址做 latching
    let (dialog, webrtc_session) = make_call::make_call(
        handle.dialog_layer.clone(),
        invite_option,
        handle.state_sender.clone(),
        input_device,
        output_device,
    )
    .await?;

    // Create child token from global cancel token
    let call_cancel_token = global_cancel_token.child_token();

    // Register token (use dialog ID as key for consistency with process_dialog)
    let dialog_id = match &dialog {
        rsipstack::dialog::dialog::Dialog::ClientInvite(d) => d.id().to_string(),
        _ => call_id.clone(),
    };
    handle
        .active_call_tokens
        .insert(dialog_id.clone(), call_cancel_token.clone());
    debug!(call_id = %call_id, dialog_id = %dialog_id, "Registered call cancellation token (child of global)");

    // Store active call with WebRTC session
    {
        let mut active = handle.active_call.lock().await;
        *active = Some(ActiveCall {
            call_id: call_id.clone(),
            dialog,
            webrtc_session: Some(webrtc_session),
            cancel_token: call_cancel_token,
        });
    }

    // Emit connected state
    let _ = handle.app_handle.emit(
        "sip://call-state",
        state::CallStatePayload {
            state: "connected".to_string(),
            call_id: Some(call_id),
            reason: None,
        },
    );

    Ok(())
}

/// Hang up the active call
pub async fn handle_hangup(handle: &SipClientHandle) -> rsipstack::Result<()> {
    let mut active = handle.active_call.lock().await;
    if let Some(mut call) = active.take() {
        info!(call_id = %call.call_id, "Hanging up call");

        // Cancel the call token first to trigger cleanup
        call.cancel_token.cancel();

        // Stop audio
        if let Some(ref mut session) = call.webrtc_session {
            session.close().await;
        }

        // Get dialog ID before moving
        let dialog_id = match &call.dialog {
            rsipstack::dialog::dialog::Dialog::ClientInvite(d) => d.id().to_string(),
            rsipstack::dialog::dialog::Dialog::ServerInvite(d) => d.id().to_string(),
            _ => call.call_id.clone(),
        };

        // Remove from active_call_tokens
        handle.active_call_tokens.remove(&dialog_id);

        match call.dialog {
            rsipstack::dialog::dialog::Dialog::ClientInvite(d) => {
                d.bye().await.map_err(|e| {
                    error!(call_id = %call.call_id, error = ?e, "Failed to send BYE");
                    rsipstack::Error::Error(format!("Failed to send BYE: {:?}", e))
                })?;
            }
            rsipstack::dialog::dialog::Dialog::ServerInvite(d) => {
                d.bye().await.map_err(|e| {
                    error!(call_id = %call.call_id, error = ?e, "Failed to send BYE");
                    rsipstack::Error::Error(format!("Failed to send BYE: {:?}", e))
                })?;
            }
            _ => {
                debug!(call_id = %call.call_id, "Other dialog type, skipping BYE");
            }
        }
        info!(call_id = %call.call_id, "Call hung up");
    } else {
        debug!("No active call to hang up");
    }
    Ok(())
}

/// Toggle mic mute for the active call
pub async fn handle_toggle_mic_mute(handle: &SipClientHandle) -> Result<bool, String> {
    let active = handle.active_call.lock().await;
    if let Some(ref call) = *active {
        if let Some(ref session) = call.webrtc_session {
            Ok(session.toggle_mic_mute())
        } else {
            Err("No WebRTC session".to_string())
        }
    } else {
        Err("No active call".to_string())
    }
}

/// Toggle speaker mute for the active call
pub async fn handle_toggle_speaker_mute(handle: &SipClientHandle) -> Result<bool, String> {
    let active = handle.active_call.lock().await;
    if let Some(ref call) = *active {
        if let Some(ref session) = call.webrtc_session {
            Ok(session.toggle_speaker_mute())
        } else {
            Err("No WebRTC session".to_string())
        }
    } else {
        Err("No active call".to_string())
    }
}

/// Answer an incoming call
pub async fn handle_answer_call(
    handle: &SipClientHandle,
    call_id: String,
    input_device: Option<String>,
    output_device: Option<String>,
    global_cancel_token: CancellationToken,
) -> rsipstack::Result<()> {
    info!(call_id = %call_id, "Answering incoming call");

    // Retrieve pending call
    let pending_call = {
        let mut pending = handle.pending_incoming.lock().await;
        pending.remove(&call_id)
    };

    let pending_call = pending_call.ok_or_else(|| {
        rsipstack::Error::Error(format!("No pending call found for call_id: {}", call_id))
    })?;

    // Create inbound WebRTC session with RTP+ICE (automatic STUN)
    let (mut webrtc_session, sdp_answer) = WebRtcSession::new_inbound(
        &pending_call.sdp_offer,
        input_device.as_deref(),
        output_device.as_deref(),
    )
    .await
    .map_err(|e| rsipstack::Error::Error(format!("Failed to create WebRTC session: {}", e)))?;

    info!(call_id = %call_id, "WebRTC session created, starting audio capture before 200 OK");

    // Start audio capture BEFORE sending 200 OK to ensure we send RTP first
    // This allows NAT to create a mapping before PBX starts sending
    webrtc_session
        .start_inbound_media_early(&pending_call.sdp_offer)
        .await
        .map_err(|e| rsipstack::Error::Error(format!("Failed to start audio capture: {}", e)))?;

    info!(call_id = %call_id, "Audio capture started, now sending 200 OK");

    // Destructure pending_call to get dialog
    let PendingCall {
        dialog,
        sdp_offer: _,
    } = pending_call;

    // Accept the dialog with SDP answer
    match dialog {
        rsipstack::dialog::dialog::Dialog::ServerInvite(d) => {
            // Create child token from global cancel token
            let call_cancel_token = global_cancel_token.child_token();
            let dialog_id = d.id().to_string();

            // Prepare ContentType header for SDP answer
            let headers =
                vec![rsip::typed::ContentType(rsip::typed::MediaType::Sdp(vec![])).into()];

            d.accept(Some(headers), Some(sdp_answer.into_bytes()))
                .map_err(|e| {
                    error!(call_id = %call_id, error = ?e, "Failed to send 200 OK");
                    rsipstack::Error::Error(format!("Failed to accept call: {:?}", e))
                })?;

            info!(call_id = %call_id, "200 OK sent successfully");

            // Register token before storing active call
            handle
                .active_call_tokens
                .insert(dialog_id.clone(), call_cancel_token.clone());
            debug!(call_id = %call_id, dialog_id = %dialog_id, "Registered call cancellation token (child of global)");

            // Store active call
            {
                let mut active = handle.active_call.lock().await;
                *active = Some(ActiveCall {
                    call_id: call_id.clone(),
                    dialog: rsipstack::dialog::dialog::Dialog::ServerInvite(d),
                    webrtc_session: None, // Will be set after playback starts
                    cancel_token: call_cancel_token,
                });
            }

            // Start playback (audio capture already started before 200 OK)
            webrtc_session
                .start_inbound_playback(&pending_call.sdp_offer, output_device.as_deref())
                .await
                .map_err(|e| rsipstack::Error::Error(format!("Failed to start playback: {}", e)))?;

            // Update active call with WebRTC session
            {
                let mut active = handle.active_call.lock().await;
                if let Some(ref mut call) = *active {
                    call.webrtc_session = Some(webrtc_session);
                }
            }

            // Emit connected state
            let _ = handle.app_handle.emit(
                "sip://call-state",
                state::CallStatePayload {
                    state: "connected".to_string(),
                    call_id: Some(call_id.clone()),
                    reason: None,
                },
            );

            info!(call_id = %call_id, "Incoming call answered successfully");
            Ok(())
        }
        _ => Err(rsipstack::Error::Error(
            "Invalid dialog type for incoming call".to_string(),
        )),
    }
}

/// Reject an incoming call
pub async fn handle_reject_call(
    handle: &SipClientHandle,
    call_id: String,
    reason_code: Option<u16>,
) -> rsipstack::Result<()> {
    info!(call_id = %call_id, reason_code = ?reason_code, "Rejecting incoming call");

    // Retrieve pending call
    let pending_call = {
        let mut pending = handle.pending_incoming.lock().await;
        pending.remove(&call_id)
    };

    let pending_call = pending_call.ok_or_else(|| {
        rsipstack::Error::Error(format!("No pending call found for call_id: {}", call_id))
    })?;

    // Determine rejection status code
    let status = match reason_code {
        Some(code) => rsip::StatusCode::try_from(code).unwrap_or(rsip::StatusCode::BusyHere),
        None => rsip::StatusCode::BusyHere,
    };

    // Reject the dialog
    match pending_call.dialog {
        rsipstack::dialog::dialog::Dialog::ServerInvite(d) => {
            d.reject(Some(status), Some("Call rejected".into()))
                .map_err(|e| {
                    error!(call_id = %call_id, error = ?e, "Failed to send rejection");
                    rsipstack::Error::Error(format!("Failed to reject call: {:?}", e))
                })?;

            // Emit ended state
            let _ = handle.app_handle.emit(
                "sip://call-state",
                state::CallStatePayload {
                    state: "ended".to_string(),
                    call_id: Some(call_id.clone()),
                    reason: Some("rejected".to_string()),
                },
            );

            info!(call_id = %call_id, "Incoming call rejected");
            Ok(())
        }
        _ => Err(rsipstack::Error::Error(
            "Invalid dialog type for incoming call".to_string(),
        )),
    }
}

/// Send DTMF digit during active call
pub async fn handle_send_dtmf(handle: &SipClientHandle, digit: String) -> Result<(), String> {
    let digit_char = digit
        .chars()
        .next()
        .ok_or("DTMF digit must be a single character")?;

    // Check if there's an active call
    let active = handle.active_call.lock().await;
    if let Some(call) = active.as_ref() {
        if let Some(session) = call.webrtc_session.as_ref() {
            info!(digit = %digit_char, call_id = %call.call_id, "Sending DTMF digit");
            session.send_dtmf(digit_char).await
        } else {
            Err("No active WebRTC session".to_string())
        }
    } else {
        Err("No active call".to_string())
    }
}

/// Enable SIP message flow logging
pub fn handle_enable_sip_flow(handle: &SipClientHandle) -> Result<(), String> {
    if let Some(ref sip_flow) = handle.sip_flow {
        sip_flow.enable();
        Ok(())
    } else {
        Err("SIP flow not available".to_string())
    }
}

/// Disable SIP message flow logging
pub fn handle_disable_sip_flow(handle: &SipClientHandle) -> Result<(), String> {
    if let Some(ref sip_flow) = handle.sip_flow {
        sip_flow.disable();
        Ok(())
    } else {
        Err("SIP flow not available".to_string())
    }
}

/// Check if SIP message flow logging is enabled
pub fn handle_is_sip_flow_enabled(handle: &SipClientHandle) -> Result<bool, String> {
    if let Some(ref sip_flow) = handle.sip_flow {
        Ok(sip_flow.is_enabled())
    } else {
        Err("SIP flow not available".to_string())
    }
}

/// Set SIP flow log directory
pub fn handle_set_sip_flow_dir(handle: &SipClientHandle, dir: String) -> Result<(), String> {
    if let Some(ref sip_flow) = handle.sip_flow {
        sip_flow.set_log_dir(std::path::PathBuf::from(dir))
    } else {
        Err("SIP flow not available".to_string())
    }
}

/// Get SIP flow log directory
pub fn handle_get_sip_flow_dir(handle: &SipClientHandle) -> Result<String, String> {
    if let Some(ref sip_flow) = handle.sip_flow {
        Ok(sip_flow.get_log_dir().to_string_lossy().to_string())
    } else {
        Err("SIP flow not available".to_string())
    }
}
