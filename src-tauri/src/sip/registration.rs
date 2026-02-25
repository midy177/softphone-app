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

/// Create a Registration instance with a UUID-based Call-ID.
/// Always use this instead of Registration::new to ensure a proper call_id.
pub fn create_registration(endpoint: EndpointInnerRef, credential: Option<Credential>) -> Registration {
    let mut reg = Registration::new(endpoint, credential);
    reg.call_id = rsip::headers::CallId::from(Uuid::new_v4().to_string());
    reg
}

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
    mut registration: Registration,
    sip_server: rsip::Uri,
    initial_expires: u64,
    cancel_token: CancellationToken,
) -> Result<()> {
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
