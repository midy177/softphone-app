use std::fmt;
use tracing::Level;
use tracing_subscriber::fmt::{
    format::{FormatEvent, FormatFields, Writer},
    time::FormatTime,
    FmtContext,
};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

const CRATE_NAME: &str = "softphone_app_lib";

/// Custom event formatter that strips the crate name prefix from target
struct CompactFormat<T> {
    timer: T,
}

impl<S, N, T> FormatEvent<S, N> for CompactFormat<T>
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
    T: FormatTime,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &tracing::Event<'_>,
    ) -> fmt::Result {
        self.timer.format_time(&mut writer)?;

        let meta = event.metadata();
        let level = *meta.level();
        if writer.has_ansi_escapes() {
            let color = match level {
                Level::ERROR => "\x1b[31m",
                Level::WARN => "\x1b[33m",
                Level::INFO => "\x1b[32m",
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

    // Log filter configuration:
    // - {level}: global log level (applies to all crates including rsipstack, rsip, etc.)
    // - log=warn: set the `log` crate level to WARN (reduce noise from underlying libraries)
    // Format: directive1,directive2,... e.g. "info,log=warn,my_crate=debug"
    let filter = EnvFilter::new(format!("{level},log=warn"));
    let timer = tracing_subscriber::fmt::time::LocalTime::rfc_3339();

    let console_layer = tracing_subscriber::fmt::layer()
        .with_ansi(ansi)
        .event_format(CompactFormat { timer });

    tracing_subscriber::registry()
        .with(filter)
        .with(console_layer)
        .try_init()
        .ok();
}
