use rsipstack::dialog::authenticate::Credential;
use rsipstack::dialog::registration::Registration;
use rsipstack::transaction::endpoint::EndpointInnerRef;
use rsipstack::Result;
use std::time::Duration;
use tokio::select;
use tokio::time::{interval, MissedTickBehavior};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info};
use uuid::Uuid;

/// Perform a single REGISTER request, returns expires value on success
pub async fn register_once(registration: &mut Registration, sip_server: rsip::Uri) -> Result<u64> {
    let resp = registration.register(sip_server.clone(), None).await?;

    if resp.status_code != rsip::StatusCode::OK {
        error!(server = %sip_server, status_code = ?resp.status_code, "Registration failed");
        return Err(rsipstack::Error::Error("Failed to register".to_string()));
    }

    let expires = registration.expires().max(60) as u64;
    info!(server = %sip_server, expires = expires, "Registered successfully");
    debug!(server = %sip_server, response = %resp.to_string(), "Registration response");

    Ok(expires)
}

/// Send REGISTER with expires=0 to unregister
pub async fn unregister(registration: &mut Registration, sip_server: rsip::Uri) -> Result<()> {
    info!(server = %sip_server, "Sending unregister (expires=0)");
    registration.register(sip_server.clone(), Some(0)).await?;
    info!(server = %sip_server, "Unregistered successfully");
    Ok(())
}

/// Background loop that refreshes registration periodically
pub async fn registration_refresh_loop(
    endpoint: EndpointInnerRef,
    sip_server: rsip::Uri,
    credential: Credential,
    initial_expires: u64,
    cancel_token: CancellationToken,
) -> Result<()> {
    let mut registration = Registration::new(endpoint, Some(credential));
    // Override the default Call-ID (which uses @restsend.com) with a proper UUID-based one.
    // Extract the server host for the domain part of the Call-ID.
    let server_host = sip_server.host_with_port.host.to_string();
    registration.call_id = rsip::headers::CallId::from(
        format!("{}@{}", Uuid::new_v4().simple(), server_host),
    );
    let refresh_time = initial_expires * 3 / 4;

    debug!(server = %sip_server, refresh_in = refresh_time, "Starting registration refresh loop");

    let mut ticker = interval(Duration::from_secs(refresh_time));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    ticker.tick().await; // first tick fires immediately, skip it

    select! {
        biased;
        _ = cancel_token.cancelled() => {
            let _ = unregister(&mut registration, sip_server.clone()).await;
            info!(server = %sip_server, "Registration refresh loop stopped by cancellation");
        }
        result = async {
            loop {
                ticker.tick().await;
                match register_once(&mut registration, sip_server.clone()).await {
                    Ok(expires) => {
                        let new_refresh = expires * 3 / 4;
                        ticker.reset_after(Duration::from_secs(new_refresh));
                        debug!(server = %sip_server, refresh_in = new_refresh, "Registration refreshed");
                    }
                    Err(e) => {
                        error!(server = %sip_server, error = ?e, "Registration refresh failed");
                        return Err(e);
                    }
                }
            }
        } => {
            result?;
        }
    }

    Ok(())
}
