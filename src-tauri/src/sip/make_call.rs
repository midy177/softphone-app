use std::sync::Arc;

use rsipstack::dialog::dialog::DialogStateSender;
use rsipstack::dialog::dialog_layer::DialogLayer;
use rsipstack::dialog::invitation::InviteOption;
use rsipstack::Error;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::webrtc::WebRtcSession;

/// Make an outbound call with internally-generated SDP (from rustrtc).
/// Returns (Dialog, WebRtcSession) on success.
///
/// 优先尝试 SRTP，若对端返回 488 Not Acceptable 则自动降级为 RTP（使用新的 call_id）。
pub async fn make_call(
    dialog_layer: Arc<DialogLayer>,
    mut invite_option: InviteOption,
    state_sender: DialogStateSender,
    input_device: Option<String>,
    output_device: Option<String>,
) -> rsipstack::Result<(rsipstack::dialog::dialog::Dialog, WebRtcSession)> {
    let caller = invite_option.caller.to_string();
    let callee = invite_option.callee.to_string();
    let call_id = invite_option.call_id.clone().unwrap_or_default();

    debug!(call_id = %call_id, caller = %caller, callee = %callee, "Preparing outbound call");

    // 优先尝试 SRTP
    let result = try_call_with_mode(
        &dialog_layer,
        &mut invite_option,
        state_sender.clone(),
        &input_device,
        &output_device,
        &call_id,
        &callee,
        true, // prefer_srtp = true
    )
    .await;

    // 若对端返回 488 Not Acceptable，降级为 RTP 重试
    if let Err(Error::Error(ref msg)) = result {
        if msg.contains("488") || msg.contains("NotAcceptableHere") {
            warn!(call_id = %call_id, "Remote rejected SRTP (488), retrying with RTP");

            // 生成新的 call_id 用于重试
            let new_call_id = Uuid::new_v4().to_string();
            invite_option.call_id = Some(new_call_id.clone());

            info!(old_call_id = %call_id, new_call_id = %new_call_id, "Retrying with new call_id");

            return try_call_with_mode(
                &dialog_layer,
                &mut invite_option,
                state_sender,
                &input_device,
                &output_device,
                &new_call_id,
                &callee,
                false, // prefer_srtp = false
            )
            .await;
        }
    }

    result
}

/// Internal helper: attempt call with specific transport mode
async fn try_call_with_mode(
    dialog_layer: &Arc<DialogLayer>,
    invite_option: &mut InviteOption,
    state_sender: DialogStateSender,
    input_device: &Option<String>,
    output_device: &Option<String>,
    call_id: &str,
    callee: &str,
    prefer_srtp: bool,
) -> rsipstack::Result<(rsipstack::dialog::dialog::Dialog, WebRtcSession)> {
    // Create WebRTC session and generate SDP offer with ICE candidates
    let (mut session, sdp_offer) = WebRtcSession::new_outbound(
        input_device.as_deref(),
        output_device.as_deref(),
        prefer_srtp,
    )
    .await
    .map_err(|e| Error::Error(format!("WebRTC session creation failed: {}", e)))?;

    debug!(
        call_id = %call_id,
        sdp_len = sdp_offer.len(),
        srtp = prefer_srtp,
        "SDP offer generated"
    );

    // Set the SDP offer
    invite_option.offer = Some(sdp_offer.into_bytes());

    // Send INVITE and wait for response
    info!(call_id = %call_id, srtp = prefer_srtp, "Sending INVITE");
    let (dialog, resp) = dialog_layer.do_invite(invite_option.clone(), state_sender).await?;
    let resp = resp.ok_or(Error::Error("No response from remote".to_string()))?;

    if resp.status_code != rsip::StatusCode::OK {
        warn!(
            call_id = %call_id,
            callee = %callee,
            status_code = ?resp.status_code,
            "Call rejected by remote"
        );
        session.close().await;
        return Err(Error::Error(format!(
            "Call rejected: {}",
            resp.status_code
        )));
    }

    info!(call_id = %call_id, callee = %callee, "Call answered (200 OK)");

    let sdp_answer = String::from_utf8_lossy(resp.body()).to_string();
    debug!(call_id = %call_id, sdp_answer_len = sdp_answer.len(), "Received SDP answer");

    // Apply SDP answer and start audio
    session
        .apply_answer(&sdp_answer, output_device.as_deref())
        .await
        .map_err(|e| Error::Error(format!("Failed to apply SDP answer: {}", e)))?;

    Ok((
        rsipstack::dialog::dialog::Dialog::ClientInvite(dialog),
        session,
    ))
}
