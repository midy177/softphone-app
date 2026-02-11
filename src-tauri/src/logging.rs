use std::fmt;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::fmt::{
    format::{FormatEvent, FormatFields, Writer},
    time::FormatTime,
    FmtContext,
};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::registry::LookupSpan;

const CRATE_NAME: &str = "sip_tts_caller";

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

    let filter = EnvFilter::new(format!("{level},log=warn"));
    let timer = tracing_subscriber::fmt::time::LocalTime::rfc_3339();

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(ansi)
        .event_format(CompactFormat { timer })
        .try_init()
        .ok();
}
