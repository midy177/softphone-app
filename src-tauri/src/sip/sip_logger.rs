use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

static SIP_LOG_FILE: Mutex<Option<std::fs::File>> = Mutex::new(None);

/// Initialize SIP message logger
pub fn init_sip_logger(log_dir: &str) -> std::io::Result<()> {
    let log_path = PathBuf::from(log_dir);
    fs::create_dir_all(&log_path)?;

    let file_path = log_path.join("sip_messages.log");
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(file_path)?;

    *SIP_LOG_FILE.lock().unwrap() = Some(file);

    log_message("INFO", "========== SIP Message Log Started ==========");

    Ok(())
}

/// Log a SIP message
pub fn log_sip_message(direction: &str, message: &str) {
    let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
    let separator = "=".repeat(80);

    let formatted = format!(
        "\n{}\n[{}] {}\n{}\n{}\n",
        separator, timestamp, direction, separator, message
    );

    log_message("SIP", &formatted);
}

/// Internal log function
fn log_message(_level: &str, message: &str) {
    if let Ok(mut guard) = SIP_LOG_FILE.lock() {
        if let Some(ref mut file) = *guard {
            let _ = writeln!(file, "{}", message);
            let _ = file.flush();
        }
    }
}
