use std::fmt;
use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::fmt::{
    format::{FormatEvent, FormatFields, Writer},
    time::FormatTime,
    FmtContext,
};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::Layer;

const CRATE_NAME: &str = "softphone_app_lib";

/// SIP message logger that writes to file
struct SipMessageLogger {
    file: Arc<Mutex<File>>,
}

impl SipMessageLogger {
    fn new(log_dir: &str) -> std::io::Result<Self> {
        let log_path = PathBuf::from(log_dir);
        fs::create_dir_all(&log_path)?;

        let file_path = log_path.join("sip_messages.log");
        let file = File::create(file_path)?;

        Ok(Self {
            file: Arc::new(Mutex::new(file)),
        })
    }

    fn log_message(&self, timestamp: &str, direction: &str, message: &str) {
        if let Ok(mut file) = self.file.lock() {
            let separator = "=".repeat(80);
            let _ = writeln!(file, "\n{}", separator);
            let _ = writeln!(file, "[{}] {}", timestamp, direction);
            let _ = writeln!(file, "{}", separator);
            let _ = writeln!(file, "{}", message);
            let _ = file.flush();
        }
    }
}

/// Custom tracing layer that captures SIP messages and writes to file
struct SipMessageLayer {
    logger: Arc<SipMessageLogger>,
}

impl SipMessageLayer {
    fn new(logger: Arc<SipMessageLogger>) -> Self {
        Self { logger }
    }
}

impl<S> Layer<S> for SipMessageLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: tracing_subscriber::layer::Context<'_, S>) {
        let metadata = event.metadata();
        let target = metadata.target();

        // Capture SIP-related logs from rsipstack and rsip crates
        if target.contains("rsipstack") || target.contains("rsip") {
            let mut visitor = SipMessageVisitor::default();
            event.record(&mut visitor);

            // Build complete message from all fields
            let mut parts = Vec::new();
            if let Some(msg) = visitor.message {
                parts.push(msg);
            }
            for (k, v) in visitor.other_fields {
                parts.push(format!("{}: {}", k, v));
            }

            if !parts.is_empty() {
                let full_message = parts.join("\n");
                let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f").to_string();

                // Determine direction from message content or target
                let direction = if full_message.contains("send") || full_message.contains("Send")
                    || full_message.contains("SEND") || full_message.contains("->")
                {
                    "OUTGOING"
                } else if full_message.contains("recv") || full_message.contains("Recv")
                    || full_message.contains("RECV") || full_message.contains("<-")
                {
                    "INCOMING"
                } else {
                    "INFO"
                };

                self.logger.log_message(&timestamp, direction, &full_message);
            }
        }
    }
}

#[derive(Default)]
struct SipMessageVisitor {
    message: Option<String>,
    other_fields: Vec<(String, String)>,
}

impl tracing::field::Visit for SipMessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let field_name = field.name();
        let value_str = format!("{:?}", value);

        if field_name == "message" {
            self.message = Some(value_str);
        } else {
            self.other_fields.push((field_name.to_string(), value_str));
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        let field_name = field.name();

        if field_name == "message" {
            self.message = Some(value.to_string());
        } else {
            self.other_fields.push((field_name.to_string(), value.to_string()));
        }
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.other_fields.push((field.name().to_string(), value.to_string()));
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.other_fields.push((field.name().to_string(), value.to_string()));
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.other_fields.push((field.name().to_string(), value.to_string()));
    }
}

/// Custom event formatter that strips the crate name prefix from target
struct CompactFormat<T> {
    timer: T,
}

impl<S, N, T> FormatEvent<S, N> for CompactFormat<T>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
    T: FormatTime,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        self.timer.format_time(&mut writer)?;

        let meta = event.metadata();
        let level = *meta.level();
        if writer.has_ansi_escapes() {
            let color = match level {
                Level::ERROR => "\x1b[31m",
                Level::WARN  => "\x1b[33m",
                Level::INFO  => "\x1b[32m",
                Level::DEBUG => "\x1b[34m",
                Level::TRACE => "\x1b[35m",
            };
            write!(writer, " {color}{level:>5}\x1b[0m ")?;
        } else {
            write!(writer, " {level:>5} ")?;
        }

        let target = meta.target();
        let owned;
        let target = if target.starts_with(CRATE_NAME) {
            owned = target.replacen(CRATE_NAME, "crate", 1);
            &owned
        } else {
            target
        };
        write!(writer, "{target}")?;

        if let Some(line) = meta.line() {
            write!(writer, ":{line}")?;
        }

        write!(writer, " ")?;
        ctx.format_fields(writer.by_ref(), event)?;
        writeln!(writer)
    }
}

pub fn initialize_logging(log_level: &str, ansi: bool) {
    let level = match log_level.to_lowercase().as_str() {
        "trace" => Level::TRACE,
        "debug" => Level::DEBUG,
        "info" => Level::INFO,
        "warn" => Level::WARN,
        "error" => Level::ERROR,
        _ => {
            eprintln!("Invalid log level '{}', using default 'info'", log_level);
            Level::INFO
        }
    };

    let filter = EnvFilter::new(format!("{level},log=warn,rsipstack=debug,rsip=debug"));
    let timer = tracing_subscriber::fmt::time::LocalTime::rfc_3339();

    // Create SIP message logger
    let sip_logger = match SipMessageLogger::new("logs") {
        Ok(logger) => {
            eprintln!("SIP message logging enabled: logs/sip_messages.log");
            Some(Arc::new(logger))
        }
        Err(e) => {
            eprintln!("Failed to initialize SIP message logger: {}", e);
            None
        }
    };

    // Build the subscriber with layers
    let console_layer = tracing_subscriber::fmt::layer()
        .with_ansi(ansi)
        .event_format(CompactFormat { timer });

    let subscriber = tracing_subscriber::registry()
        .with(filter)
        .with(console_layer);

    // Add SIP message layer if available
    if let Some(logger) = sip_logger {
        let sip_layer = SipMessageLayer::new(logger);
        subscriber.with(sip_layer).try_init().ok();
    } else {
        subscriber.try_init().ok();
    }
}
