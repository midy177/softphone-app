use tracing::info;

/// Wait for system shutdown signal.
///
/// - Unix (Linux / macOS): SIGINT (Ctrl+C), SIGTERM (`kill`), SIGQUIT (`kill -3` / Ctrl+\)
/// - Windows: Ctrl+C, Ctrl+Close (close window), Ctrl+Shutdown (system shutdown)
pub async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};

        let mut sigterm = signal(SignalKind::terminate()).expect("failed to register SIGTERM");
        let mut sigquit = signal(SignalKind::quit()).expect("failed to register SIGQUIT");

        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!(signal = "SIGINT", "Shutting down...");
            }
            _ = sigterm.recv() => {
                info!(signal = "SIGTERM", "Shutting down...");
            }
            _ = sigquit.recv() => {
                info!(signal = "SIGQUIT", "Shutting down...");
            }
        }
    }

    #[cfg(windows)]
    {
        use tokio::signal::windows;

        let mut ctrl_close = windows::ctrl_close().expect("failed to register ctrl_close");
        let mut ctrl_shutdown = windows::ctrl_shutdown().expect("failed to register ctrl_shutdown");

        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!(signal = "Ctrl+C", "Shutting down...");
            }
            _ = ctrl_close.recv() => {
                info!(signal = "Ctrl+Close", "Shutting down...");
            }
            _ = ctrl_shutdown.recv() => {
                info!(signal = "Ctrl+Shutdown", "Shutting down...");
            }
        }
    }
}
