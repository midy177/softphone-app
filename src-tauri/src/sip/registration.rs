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

/// SIP registration manager.
///
/// Wraps rsipstack's `Registration` and owns all state needed for the full
/// registration lifecycle: initial REGISTER, periodic refresh, and unregister
/// on shutdown.
///
/// Create once via `SipRegistration::new()`; the UUID-based Call-ID is
/// generated at construction time and reused for every subsequent request,
/// as required by RFC 3261.
pub struct Registrant {
    inner: Registration,
    sip_server: rsip::Uri,
}

impl Registrant {
    /// Create a new registration manager.
    ///
    /// Initialises the underlying `Registration` with a fresh UUID Call-ID.
    pub fn new(endpoint: EndpointInnerRef, credential: Credential, sip_server: rsip::Uri) -> Self {
        let mut inner = Registration::new(endpoint, Some(credential));
        inner.call_id = rsip::headers::CallId::from(Uuid::new_v4().to_string());
        Self { inner, sip_server }
    }

    /// Send a single REGISTER request and return the negotiated expires value.
    pub async fn register_once(&mut self) -> Result<u64> {
        let resp = self.inner.register(self.sip_server.clone(), None).await?;

        if resp.status_code != rsip::StatusCode::OK {
            error!(server = %self.sip_server, status_code = ?resp.status_code, "Registration failed");
            return Err(rsipstack::Error::Error("Failed to register".to_string()));
        }

        let expires = self.inner.expires().max(60) as u64;
        info!(server = %self.sip_server, expires = expires, "Registered successfully");
        debug!(server = %self.sip_server, "Registration response OK");

        Ok(expires)
    }

    /// Send REGISTER with expires=0 to unregister.
    async fn unregister(&mut self) {
        info!(server = %self.sip_server, "Sending unregister (expires=0)");
        if let Err(e) = self.inner.register(self.sip_server.clone(), Some(0)).await {
            error!(server = %self.sip_server, error = ?e, "Unregister failed");
        } else {
            info!(server = %self.sip_server, "Unregistered successfully");
        }
    }

    /// Run the periodic refresh loop.
    ///
    /// Refreshes at 75% of the current expires interval, capped at
    /// `max_keepalive_secs` when set.  For connection-oriented transports
    /// (TCP/TLS/WS/WSS) pass a value such as `Some(25)`: rsipstack does not
    /// auto-remove dead connections from its send map, so the only reliable
    /// way to prevent a stale-connection send error is to keep the TCP
    /// session alive with periodic REGISTER traffic before the server idles
    /// it out.
    ///
    /// Sends an unregister on cancellation before returning.
    pub async fn run_refresh_loop(
        mut self,
        initial_expires: u64,
        cancel_token: CancellationToken,
        max_keepalive_secs: Option<u64>,
    ) -> Result<()> {
        let cap = |t: u64| match max_keepalive_secs {
            Some(max) => t.min(max),
            None => t,
        };

        let refresh_time = cap(initial_expires * 3 / 4);
        debug!(
            server = %self.sip_server,
            refresh_in = refresh_time,
            max_keepalive = ?max_keepalive_secs,
            "Starting registration refresh loop"
        );

        let mut ticker = interval(Duration::from_secs(refresh_time));
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        ticker.tick().await; // first tick fires immediately, skip it

        select! {
            biased;
            _ = cancel_token.cancelled() => {
                self.unregister().await;
                info!(server = %self.sip_server, "Registration refresh loop stopped by cancellation");
            }
            result = async {
                loop {
                    ticker.tick().await;
                    match self.register_once().await {
                        Ok(expires) => {
                            let new_refresh = cap(expires * 3 / 4);
                            ticker.reset_after(Duration::from_secs(new_refresh));
                            debug!(server = %self.sip_server, refresh_in = new_refresh, "Registration refreshed");
                        }
                        Err(e) => {
                            error!(server = %self.sip_server, error = ?e, "Registration refresh failed");
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
}
