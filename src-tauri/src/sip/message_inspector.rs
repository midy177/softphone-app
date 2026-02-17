use rsip::{headers::UntypedHeader, prelude::HeadersExt, SipMessage};
use rsipstack::{transaction::endpoint::MessageInspector, transport::SipAddr};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tracing::{error, info};

/// SIP 消息流检查器，支持动态开关日志记录
#[derive(Clone)]
pub struct SipFlow {
    log_file: Arc<Mutex<Option<std::fs::File>>>,
    enabled: Arc<Mutex<bool>>,
    log_dir: Arc<Mutex<PathBuf>>,
}

impl SipFlow {
    /// 创建新的 SIP 消息检查器
    ///
    /// # 参数
    /// - `log_dir`: 日志目录，如果为 None 则使用系统临时目录
    /// - `enabled`: 是否启用日志记录
    pub fn new(log_dir: Option<&str>, enabled: bool) -> Self {
        // 确定日志目录
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

    /// 打开日志文件
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

    /// 启用日志记录
    pub fn enable(&self) {
        let mut enabled = self.enabled.lock().unwrap();
        if *enabled {
            return; // 已经启用
        }

        *enabled = true;
        let log_dir = self.log_dir.lock().unwrap();
        let mut log_file = self.log_file.lock().unwrap();
        *log_file = Self::open_log_file(&log_dir);
        info!("SIP flow logging enabled");
    }

    /// 禁用日志记录
    pub fn disable(&self) {
        let mut enabled = self.enabled.lock().unwrap();
        if !*enabled {
            return; // 已经禁用
        }

        *enabled = false;
        let mut log_file = self.log_file.lock().unwrap();
        *log_file = None;
        info!("SIP flow logging disabled");
    }

    /// 检查是否启用
    pub fn is_enabled(&self) -> bool {
        *self.enabled.lock().unwrap()
    }

    /// 设置日志目录（如果日志已启用，会重新打开文件）
    pub fn set_log_dir(&self, dir: PathBuf) -> Result<(), String> {
        let mut log_dir = self.log_dir.lock().unwrap();
        *log_dir = dir.clone();

        // 如果当前已启用，重新打开日志文件
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

    /// 获取当前日志目录
    pub fn get_log_dir(&self) -> PathBuf {
        self.log_dir.lock().unwrap().clone()
    }

    /// 记录 SIP 消息
    fn record(&self, direction: &str, msg: &SipMessage) {
        // 检查是否启用
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

            // 写入日志文件
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
