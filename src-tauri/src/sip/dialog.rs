use std::sync::Arc;
use rsipstack::dialog::dialog::{Dialog, DialogState, DialogStateReceiver};
use rsipstack::dialog::dialog_layer::DialogLayer;
use rsipstack::Error;
use tauri::{AppHandle, Emitter};
use tracing::{debug, info, warn};

use crate::sip::state::CallStatePayload;

pub async fn process_dialog(
    dialog_layer: Arc<DialogLayer>,
    state_receiver: DialogStateReceiver,
    app_handle: AppHandle,
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
                let _ = app_handle.emit("sip://call-state", CallStatePayload {
                    state: "ringing".to_string(),
                    call_id: Some(id.to_string()),
                    reason: None,
                });
            }
            DialogState::Terminated(id, reason) => {
                info!(dialog_id = %id, reason = ?reason, "Dialog terminated");
                dialog_layer.remove_dialog(&id);
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
