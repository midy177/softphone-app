use std::sync::Arc;
use dashmap::DashMap;
use rsipstack::dialog::dialog::{Dialog, DialogState, DialogStateReceiver};
use rsipstack::dialog::dialog_layer::DialogLayer;
use rsipstack::Error;
use tauri::{AppHandle, Emitter};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::sip::state::CallStatePayload;

pub async fn process_dialog(
    dialog_layer: Arc<DialogLayer>,
    state_receiver: DialogStateReceiver,
    app_handle: AppHandle,
    active_call_tokens: Arc<DashMap<String, CancellationToken>>,
) -> Result<(), Error> {
    let mut state_receiver = state_receiver;
    while let Some(state) = state_receiver.recv().await {
        match state {
            DialogState::Calling(id) => {
                let dialog = match dialog_layer.get_dialog(&id) {
                    Some(d) => d,
                    None => {
                        warn!(dialog_id = %id, "Dialog not found for Calling state");
                        continue;
                    }
                };
                match dialog {
                    Dialog::ServerInvite(_) => {
                        // Don't auto-reject - wait for user action (accept/reject)
                        debug!(dialog_id = %id, "Server invite dialog created, waiting for user action");
                    }
                    Dialog::ClientInvite(_) => {
                        debug!(dialog_id = %id, "Client invite dialog calling");
                        let _ = app_handle.emit("sip://call-state", CallStatePayload {
                            state: "calling".to_string(),
                            call_id: Some(id.to_string()),
                            reason: None,
                        });
                    }
                    _ => {
                        debug!(dialog_id = %id, "Other dialog type calling");
                    }
                }
            }
            DialogState::Early(id, _resp) => {
                debug!(dialog_id = %id, "Dialog entered Early state (ringing)");

                // Only emit ringing state for outbound calls (ClientInvite)
                // For inbound calls (ServerInvite), we don't change the state
                // because the frontend should already be in 'incoming' state
                let dialog = dialog_layer.get_dialog(&id);
                if let Some(Dialog::ClientInvite(_)) = dialog {
                    let _ = app_handle.emit("sip://call-state", CallStatePayload {
                        state: "ringing".to_string(),
                        call_id: Some(id.to_string()),
                        reason: None,
                    });
                }
            }
            DialogState::Terminated(id, reason) => {
                info!(dialog_id = %id, reason = ?reason, "Dialog terminated");
                dialog_layer.remove_dialog(&id);

                // Cancel and remove the call's cancellation token to trigger cleanup
                if let Some((_, token)) = active_call_tokens.remove(&id.to_string()) {
                    debug!(dialog_id = %id, "Cancelling call token for cleanup");
                    token.cancel();
                }

                let _ = app_handle.emit("sip://call-state", CallStatePayload {
                    state: "ended".to_string(),
                    call_id: Some(id.to_string()),
                    reason: Some(format!("{:?}", reason)),
                });
            }
            _ => {
                debug!(state = %state, "Dialog state changed");
            }
        }
    }
    Ok(())
}
