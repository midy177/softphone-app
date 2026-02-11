use rsip::prelude::HeadersExt;
use rsip::headers::UntypedHeader;
use rsipstack::dialog::dialog_layer::DialogLayer;
use rsipstack::transaction::TransactionReceiver;
use rsipstack::{Error, Result};
use std::sync::Arc;
use rsipstack::dialog::dialog::DialogStateSender;
use tracing::{debug, info, warn};

pub async fn process_incoming_request(
    dialog_layer: Arc<DialogLayer>,
    mut incoming: TransactionReceiver,
    state_sender: DialogStateSender,
    contact: rsip::Uri,
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
                // reject all invite
                if tx.original.method == rsip::Method::Invite {
                    warn!(call_id = %call_id, "Rejecting incoming INVITE, replying 486 Busy Here");
                    tx.reply(rsip::StatusCode::BusyHere).await?;
                    continue;
                }
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
