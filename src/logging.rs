use crate::config::Log;
use std::error::Error;
use std::str::FromStr;
use tracing::Subscriber;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer, Registry, fmt, registry};

pub struct Guard {
    _stdout_guard: Option<WorkerGuard>,
    _file_guard: Option<WorkerGuard>,
}

pub fn setup_logging(cfg: &Log) -> Result<Guard, Box<dyn Error + Send + Sync>> {
    let filter = construct_env_filter(cfg);

    let (stdout_layer, stdout_guard) = construct_stdout_layer(cfg)
        .map(|(l, g)| (Some(l), Some(g)))
        .unwrap_or((None, None));

    let (file_layer, file_guard) = construct_log_file_layer(cfg)?
        .map(|(l, g)| (Some(l), Some(g)))
        .unwrap_or((None, None));

    Registry::default()
        .with(filter)
        .with(stdout_layer)
        .with(file_layer)
        .init();

    Ok(Guard {
        _stdout_guard: stdout_guard,
        _file_guard: file_guard,
    })
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

fn construct_stdout_layer<S>(cfg: &Log) -> Option<(Box<dyn Layer<S> + Send + Sync>, WorkerGuard)>
where
    S: Subscriber + for<'a> registry::LookupSpan<'a>,
{
    if !cfg.enable_stdout {
        return None;
    }

    let (non_blocking, guard) = tracing_appender::non_blocking(std::io::stdout());
    Some((common_layer(non_blocking), guard))
}

#[allow(clippy::type_complexity)]
fn construct_log_file_layer<S>(
    cfg: &Log,
) -> Result<Option<(Box<dyn Layer<S> + Send + Sync>, WorkerGuard)>, Box<dyn Error + Send + Sync>>
where
    S: Subscriber + for<'a> registry::LookupSpan<'a>,
{
    if !cfg.enable_log_file {
        return Ok(None);
    }

    match &cfg.log_file_directory {
        None => {
            eprintln!("No log file path specified. Skipping log file configuration.");
            Ok(None)
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
                        cfg.log_file_directory.as_deref().unwrap_or(""),
                        e
                    )
                })?;

            let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
            let layer = common_layer(non_blocking);

            Ok(Some((layer, guard)))
        }
    }
}

fn common_layer<S, W>(w: W) -> Box<dyn Layer<S> + Send + Sync>
where
    S: Subscriber + for<'a> registry::LookupSpan<'a>,
    W: for<'writer> MakeWriter<'writer> + Send + Sync + 'static,
{
    fmt::layer()
        .with_writer(w)
        .with_ansi(true)
        .with_target(true)
        .with_line_number(true)
        .with_level(true)
        .with_span_events(FmtSpan::CLOSE)
        .compact()
        .boxed()
}
