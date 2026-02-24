use rsip::{headers::UntypedHeader, prelude::HeadersExt, SipMessage};
use rsipstack::{transaction::endpoint::MessageInspector, transport::SipAddr};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tracing::{error, info};

/// SIP message flow inspector with dynamic enable/disable of logging
#[derive(Clone)]
pub struct SipFlow {
    log_file: Arc<Mutex<Option<std::fs::File>>>,
    enabled: Arc<Mutex<bool>>,
    log_dir: Arc<Mutex<PathBuf>>,
}

impl SipFlow {
    /// Create a new SIP message inspector
    ///
    /// # Parameters
    /// - `log_dir`: log directory; uses the system temp dir if None
    /// - `enabled`: whether to enable logging on creation
    pub fn new(log_dir: Option<&str>, enabled: bool) -> Self {
        // Resolve log directory
        let dir = log_dir.map(PathBuf::from).unwrap_or_else(|| {
            let mut temp = std::env::temp_dir();
            temp.push("softphone-sip-logs");
            temp
        });

        let log_file = if enabled {
            Self::open_log_file(&dir)
        } else {
            None
        };

        Self {
            log_file: Arc::new(Mutex::new(log_file)),
            enabled: Arc::new(Mutex::new(enabled)),
            log_dir: Arc::new(Mutex::new(dir)),
        }
    }

    /// Open (or create) the log file in the given directory
    fn open_log_file(dir: &PathBuf) -> Option<std::fs::File> {
        if let Err(e) = fs::create_dir_all(dir) {
            error!("Failed to create SIP flow log directory: {}", e);
            return None;
        }

        let file_path = dir.join("sip-flow.log");
        match OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file_path)
        {
            Ok(file) => {
                info!("SIP flow logging enabled: {}", file_path.display());
                Some(file)
            }
            Err(e) => {
                error!("Failed to create SIP flow log file: {}", e);
                None
            }
        }
    }

    /// Enable SIP message logging
    pub fn enable(&self) {
        let mut enabled = self.enabled.lock().unwrap();
        if *enabled {
            return; // already enabled
        }

        *enabled = true;
        let log_dir = self.log_dir.lock().unwrap();
        let mut log_file = self.log_file.lock().unwrap();
        *log_file = Self::open_log_file(&log_dir);
        info!("SIP flow logging enabled");
    }

    /// Disable SIP message logging
    pub fn disable(&self) {
        let mut enabled = self.enabled.lock().unwrap();
        if !*enabled {
            return; // already disabled
        }

        *enabled = false;
        let mut log_file = self.log_file.lock().unwrap();
        *log_file = None;
        info!("SIP flow logging disabled");
    }

    /// Check whether logging is currently enabled
    pub fn is_enabled(&self) -> bool {
        *self.enabled.lock().unwrap()
    }

    /// Update the log directory (reopens the log file if logging is currently enabled)
    pub fn set_log_dir(&self, dir: PathBuf) -> Result<(), String> {
        let mut log_dir = self.log_dir.lock().unwrap();
        *log_dir = dir.clone();

        // Reopen log file in the new directory if currently enabled
        let enabled = *self.enabled.lock().unwrap();
        if enabled {
            let mut log_file = self.log_file.lock().unwrap();
            *log_file = Self::open_log_file(&dir);
            if log_file.is_none() {
                return Err(format!(
                    "Failed to open log file in directory: {}",
                    dir.display()
                ));
            }
            info!("SIP flow log directory changed to: {}", dir.display());
        }

        Ok(())
    }

    /// Get the current log directory
    pub fn get_log_dir(&self) -> PathBuf {
        self.log_dir.lock().unwrap().clone()
    }

    /// Record a SIP message to the log file
    fn record(&self, direction: &str, msg: &SipMessage) {
        // Skip if logging is disabled
        if !self.is_enabled() {
            return;
        }

        let call_id = match msg {
            rsip::SipMessage::Request(req) => req.call_id_header(),
            rsip::SipMessage::Response(resp) => resp.call_id_header(),
        };

        if let Ok(id) = call_id {
            let call_id_str = id.value().to_string();
            let timestamp = chrono::Utc::now();
            let content = msg.to_string();

            // Write to log file
            if let Ok(mut log_file_guard) = self.log_file.lock() {
                if let Some(ref mut file) = *log_file_guard {
                    let separator = "=".repeat(80);
                    let timestamp_str = timestamp.format("%Y-%m-%d %H:%M:%S%.3f");

                    let _ = writeln!(file, "\n{}", separator);
                    let _ = writeln!(
                        file,
                        "[{}] {} (Call-ID: {})",
                        timestamp_str, direction, call_id_str
                    );
                    let _ = writeln!(file, "{}", separator);
                    let _ = writeln!(file, "{}", content);
                    let _ = file.flush();
                }
            }
        }
    }
}

impl MessageInspector for SipFlow {
    fn before_send(&self, msg: SipMessage, _dest: Option<&SipAddr>) -> SipMessage {
        self.record("OUTGOING", &msg);
        msg
    }

    fn after_received(&self, msg: SipMessage, _from: &SipAddr) -> SipMessage {
        self.record("INCOMING", &msg);
        msg
    }
}
