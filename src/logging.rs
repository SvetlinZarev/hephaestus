use crate::config::Log;
use std::error::Error;
use std::str::FromStr;
use tracing::Subscriber;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, registry, EnvFilter, Layer, Registry};

pub fn setup_logging(cfg: &Log) -> Result<Option<WorkerGuard>, Box<dyn Error + Send + Sync>> {
    let filter = construct_env_filter(cfg);
    let stdout_layer = construct_stdout_layer(cfg);
    let (file_layer, guard) = construct_log_file_layer(cfg)?;

    Registry::default()
        .with(filter)
        .with(stdout_layer)
        .with(file_layer)
        .init();

    Ok(guard)
}

fn construct_env_filter(cfg: &Log) -> EnvFilter {
    let mut filter = EnvFilter::builder()
        .with_default_directive(
            LevelFilter::from_str(&cfg.level)
                .unwrap_or(LevelFilter::INFO)
                .into(),
        )
        .from_env_lossy();

    for directive in &cfg.directives {
        match directive.parse() {
            Ok(directive) => {
                filter = filter.add_directive(directive);
            }

            Err(e) => {
                eprintln!("Skipping invalid log directive '{:?}': {}", directive, e);
            }
        }
    }

    filter
}

fn construct_stdout_layer<S>(cfg: &Log) -> Option<Box<dyn Layer<S> + Send + Sync>>
where
    S: Subscriber + for<'a> registry::LookupSpan<'a>,
{
    if !cfg.enable_stdout {
        return None;
    }

    Some(
        fmt::layer()
            .with_writer(std::io::stdout)
            .with_ansi(true)
            .with_target(true)
            .with_line_number(true)
            .with_level(true)
            .with_span_events(FmtSpan::NONE)
            .compact()
            .boxed(),
    )
}

fn construct_log_file_layer<S>(
    cfg: &Log,
) -> Result<
    (Option<Box<dyn Layer<S> + Send + Sync>>, Option<WorkerGuard>),
    Box<dyn Error + Send + Sync>,
>
where
    S: Subscriber + for<'a> registry::LookupSpan<'a>,
{
    if !cfg.enable_log_file {
        return Ok((None, None));
    }

    match &cfg.log_file_directory {
        None => {
            eprintln!("No log file path specified. Skipping log file configuration.");
            Ok((None, None))
        }

        Some(directory) => {
            let file_appender = RollingFileAppender::builder()
                .rotation(Rotation::DAILY)
                .filename_prefix("hephaestus")
                .filename_suffix("log")
                .max_log_files(cfg.max_log_files)
                .build(directory)
                .map_err(|e| {
                    format!(
                        "Failed to create file appender for directory [{}]. Error=[{}]",
                        cfg.log_file_directory
                            .as_ref()
                            .map(|s| s.as_str())
                            .unwrap_or(""),
                        e
                    )
                })?;

            let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

            let layer = fmt::layer()
                .with_writer(non_blocking)
                .with_ansi(false)
                .with_target(true)
                .with_line_number(true)
                .with_level(true)
                .with_span_events(FmtSpan::NONE)
                .compact()
                .boxed();

            Ok((Some(layer), Some(guard)))
        }
    }
}
