use rsip::prelude::HeadersExt;
use rsip::headers::UntypedHeader;
use rsipstack::dialog::dialog_layer::DialogLayer;
use rsipstack::transaction::TransactionReceiver;
use rsipstack::{Error, Result};
use std::collections::HashMap;
use std::sync::Arc;
use rsipstack::dialog::dialog::DialogStateSender;
use tauri::Emitter;
use tracing::{debug, info, warn};

use crate::sip::state::{IncomingCallPayload, PendingCall};

pub async fn process_incoming_request(
    dialog_layer: Arc<DialogLayer>,
    mut incoming: TransactionReceiver,
    state_sender: DialogStateSender,
    contact: rsip::Uri,
    app_handle: tauri::AppHandle,
    pending_incoming: Arc<tokio::sync::Mutex<HashMap<String, PendingCall>>>,
) -> Result<()> {
    while let Some(mut tx) = incoming.recv().await {
        let method = tx.original.method.to_string();
        let call_id = tx.original.call_id_header()
            .map(|h| h.value().to_string())
            .unwrap_or("no_call_id".to_string());

        debug!(method = %method, call_id = %call_id, "Received incoming request");

        match tx.original.to_header()?.tag()?.as_ref() {
            Some(_) => match dialog_layer.match_dialog(&tx) {
                Some(mut d) => {
                    debug!(method = %method, call_id = %call_id, "Matched existing dialog");
                    tokio::spawn(async move {
                        d.handle(&mut tx).await?;
                        Ok::<_, Error>(())
                    });
                    continue;
                }
                None => {
                    warn!(method = %method, call_id = %call_id, "Dialog not found, replying 481");
                    tx.reply(rsip::StatusCode::CallTransactionDoesNotExist)
                        .await?;
                    continue;
                }
            },
            None => {}
        }
        // out dialog, new server dialog
        match tx.original.method {
            rsip::Method::Invite | rsip::Method::Ack => {
                // Handle incoming INVITE
                if tx.original.method == rsip::Method::Invite {
                    // Check if we already have a pending call for this call_id
                    let already_pending = {
                        let pending = pending_incoming.lock().await;
                        pending.contains_key(&call_id)
                    };

                    if already_pending {
                        debug!(call_id = %call_id, "INVITE retransmission for pending call, ignoring");
                        continue;
                    }

                    // Extract caller information
                    let caller = tx.original.from_header()
                        .ok()
                        .and_then(|h| h.uri().ok())
                        .map(|uri| uri.to_string())
                        .unwrap_or_else(|| "Unknown".to_string());

                    let callee = tx.original.to_header()
                        .ok()
                        .and_then(|h| h.uri().ok())
                        .map(|uri| uri.to_string());

                    // Extract SDP offer
                    let sdp_offer = String::from_utf8_lossy(&tx.original.body).to_string();

                    info!(call_id = %call_id, caller = %caller, "Received incoming INVITE");

                    // Create server dialog but don't respond yet - wait for user action
                    let dialog = match dialog_layer.get_or_create_server_invite(
                        &tx,
                        state_sender.clone(),
                        None,
                        Some(contact.clone()),
                    ) {
                        Ok(d) => d,
                        Err(e) => {
                            warn!(call_id = %call_id, error = ?e, "Failed to create server dialog, replying 481");
                            tx.reply(rsip::StatusCode::CallTransactionDoesNotExist)
                                .await?;
                            continue;
                        }
                    };

                    info!(call_id = %call_id, "Created server dialog, notifying frontend");

                    // Send 180 Ringing to keep dialog alive while waiting for user action
                    if let Err(e) = dialog.ringing(None, None) {
                        warn!(call_id = %call_id, error = ?e, "Failed to send 180 Ringing");
                        tx.reply(rsip::StatusCode::ServerInternalError).await?;
                        continue;
                    }

                    info!(call_id = %call_id, "Sent 180 Ringing, waiting for user action");

                    // Store pending call
                    {
                        let mut pending = pending_incoming.lock().await;
                        pending.insert(call_id.clone(), PendingCall {
                            call_id: call_id.clone(),
                            caller: caller.clone(),
                            dialog: rsipstack::dialog::dialog::Dialog::ServerInvite(dialog.clone()),
                            sdp_offer: sdp_offer.clone(),
                        });
                    }

                    // Spawn task to hold transaction alive until call is answered/rejected
                    // This prevents the channel from closing before accept()/reject() is called
                    let call_id_clone = call_id.clone();
                    let pending_clone = pending_incoming.clone();
                    tokio::spawn(async move {
                        // Hold tx here and wait for the call to be removed from pending
                        // (which happens when user answers or rejects)
                        loop {
                            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                            let pending = pending_clone.lock().await;
                            if !pending.contains_key(&call_id_clone) {
                                // Call was answered or rejected, tx can be dropped now
                                debug!(call_id = %call_id_clone, "Call no longer pending, dropping transaction");
                                break;
                            }
                            drop(pending); // Release lock before next iteration
                        }
                        // tx is dropped here after call is answered/rejected
                    });

                    // Emit event to frontend
                    let payload = IncomingCallPayload {
                        call_id: call_id.clone(),
                        caller,
                        callee,
                    };

                    if let Err(e) = app_handle.emit("sip://incoming-call", payload) {
                        warn!(call_id = %call_id, error = ?e, "Failed to emit incoming call event");
                    }

                    continue;
                }
                // Handle ACK for pending calls
                let mut dialog = match dialog_layer.get_or_create_server_invite(
                    &tx,
                    state_sender.clone(),
                    None,
                    Some(contact.clone()),
                ) {
                    Ok(d) => d,
                    Err(e) => {
                        warn!(call_id = %call_id, error = ?e, "Failed to create server dialog, replying 481");
                        tx.reply(rsip::StatusCode::CallTransactionDoesNotExist)
                            .await?;
                        continue;
                    }
                };
                info!(method = %method, call_id = %call_id, "Created new server dialog");
                tokio::spawn(async move {
                    dialog.handle(&mut tx).await?;
                    Ok::<_, Error>(())
                });
            }
            _ => {
                debug!(method = %method, call_id = %call_id, "Replying 200 OK");
                tx.reply(rsip::StatusCode::OK).await?;
            }
        }
    }
    Ok::<_, Error>(())
}
