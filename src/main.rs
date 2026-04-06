mod format;
mod run;
mod state;

use crate::{format::CodeStr, run::run};
use byte_unit::Byte;
use chrono::Local;
use clap::{ArgAction, Parser};
use env_logger::{Builder, fmt::style::Effects};
use humantime::parse_duration;
use log::LevelFilter;
use regex::RegexSet;
use std::{
    env,
    io::{self, IsTerminal, Write},
    process::exit,
    str::FromStr,
    sync::{Arc, Mutex},
    thread::sleep,
    time::Duration,
};

#[macro_use]
extern crate log;

// Defaults
const DEFAULT_DELETION_CHUNK_SIZE: usize = 1;
const DEFAULT_LOG_LEVEL: LevelFilter = LevelFilter::Debug;
const DEFAULT_THRESHOLD: &str = "10 GB";

// Size threshold argument, absolute or relative to filesystem size
#[derive(Copy, Clone)]
enum Threshold {
    Absolute(Byte),

    #[cfg(target_os = "linux")]
    Percentage(f64),
}

impl Threshold {
    // Parse a `Threshold`. Relative thresholds are only supported on Linux.
    #[cfg(target_os = "linux")]
    fn from_str(threshold: &str) -> io::Result<Threshold> {
        match threshold.strip_suffix('%') {
            Some(threshold) => {
                if cfg!(target_os = "linux") {
                    threshold
                        .trim()
                        .parse::<f64>()
                        .map_err(|_| {
                            io::Error::new(
                                io::ErrorKind::InvalidInput,
                                format!("Invalid relative threshold {}.", threshold.code_str()),
                            )
                        })
                        .and_then(|f| {
                            if f.is_normal() && (0.0_f64..=100.0_f64).contains(&f) {
                                Ok(f)
                            } else {
                                Err(io::Error::new(
                                    io::ErrorKind::InvalidInput,
                                    format!("Invalid relative threshold {}.", threshold.code_str()),
                                ))
                            }
                        })
                        .map(|f| Threshold::Percentage(f / 100.0))
                } else {
                    Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "Relative thresholds are only supported on Linux.",
                    ))
                }
            }
            None => Byte::parse_str(threshold, true)
                .map_err(|_| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("Invalid absolute threshold {}.", threshold.code_str()),
                    )
                })
                .map(Threshold::Absolute),
        }
    }

    #[cfg(not(target_os = "linux"))]
    fn from_str(threshold: &str) -> io::Result<Threshold> {
        Byte::parse_str(threshold, true)
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("Invalid absolute threshold {}.", threshold.code_str()),
                )
            })
            .map(Threshold::Absolute)
    }
}

// This struct represents the raw command-line arguments.
#[derive(Parser)]
#[command(
    about = concat!(
        env!("CARGO_PKG_DESCRIPTION"),
        "\n\n",
        "More information can be found at: ",
        env!("CARGO_PKG_HOMEPAGE")
    ),
    version,
    disable_version_flag = true
)]
struct Cli {
    #[arg(short, long, help = "Print version", action = ArgAction::Version)]
    _version: Option<bool>,

    #[arg(
        short,
        long,
        help = "Set the maximum amount of space to use for Docker images",
        default_value = DEFAULT_THRESHOLD
    )]
    threshold: String,

    #[arg(
        short,
        long,
        value_name = "REGEX",
        help = "Prevent deletion of images for which repository:tag matches <REGEX>"
    )]
    keep: Vec<String>,

    #[arg(
        short,
        long,
        value_name = "DELETION CHUNK SIZE",
        help = "Remove the specified quantity of images at a time",
        default_value_t = DEFAULT_DELETION_CHUNK_SIZE
    )]
    deletion_chunk_size: usize,

    #[arg(
        short,
        long,
        value_name = "MIN AGE",
        help = "Set the minimum age of images to consider for deletion"
    )]
    min_age: Option<String>,
}

// This struct represents the parsed command-line arguments.
pub struct Settings {
    deletion_chunk_size: usize,
    keep: Option<RegexSet>,
    min_age: Option<Duration>,
    threshold: Threshold,
}

// Set up the logger.
fn set_up_logging() {
    Builder::new()
        .filter_module(
            module_path!(),
            LevelFilter::from_str(
                &env::var("LOG_LEVEL").unwrap_or_else(|_| DEFAULT_LOG_LEVEL.to_string()),
            )
            .unwrap_or(DEFAULT_LOG_LEVEL),
        )
        .format(|buf, record| {
            let style = buf
                .default_level_style(record.level())
                .effects(Effects::BOLD);

            writeln!(
                buf,
                "{style}[{} {}]{style:#} {}",
                Local::now().format("%Y-%m-%d %H:%M:%S %:z"),
                record.level(),
                record.args(),
            )
        })
        .init();
}

// Parse the command-line arguments.
fn settings() -> io::Result<Settings> {
    let cli = Cli::parse();

    // Determine how many images to delete at once.
    let deletion_chunk_size = cli.deletion_chunk_size;

    // Determine what images need to be preserved at all costs.
    let keep = if cli.keep.is_empty() {
        None
    } else {
        match RegexSet::new(cli.keep) {
            Ok(set) => Some(set),
            Err(e) => return Err(io::Error::new(io::ErrorKind::InvalidInput, e)),
        }
    };

    // Determine the minimum age for images to be considered for deletion.
    let min_age = match cli.min_age.as_deref() {
        Some(value) => match parse_duration(value) {
            Ok(duration) => {
                debug!("{} parsed as {:?}.", "--min-age".code_str(), duration);
                Some(duration)
            }
            Err(e) => return Err(io::Error::new(io::ErrorKind::InvalidInput, e)),
        },
        None => None,
    };

    // Read the threshold.
    let threshold = Threshold::from_str(&cli.threshold)?;

    Ok(Settings {
        deletion_chunk_size,
        keep,
        min_age,
        threshold,
    })
}

// This function consumes and runs all the registered destructors. We use this mechanism instead of
// RAII for things that need to be cleaned up even when the process is killed due to a signal.
#[allow(clippy::type_complexity)]
fn run_destructors(destructors: &Arc<Mutex<Vec<Box<dyn FnOnce() + Send>>>>) {
    let mut mutex_guard = destructors.lock().unwrap();
    let destructor_fns = std::mem::take(&mut *mutex_guard);
    for destructor in destructor_fns {
        destructor();
    }
}

// Let the fun begin!
fn main() {
    // If Docuum is in the foreground process group for some TTY, the process will receive a SIGINT
    // when the user types CTRL+C at the terminal. The default behavior is to crash when this signal
    // is received. However, we would rather clean up resources before terminating, so we trap the
    // signal here. This code also traps SIGHUP and SIGTERM, since we compile the `ctrlc` crate with
    // the `termination` feature [ref:ctrlc_term].
    let destructors = Arc::new(Mutex::new(Vec::<Box<dyn FnOnce() + Send>>::new()));
    let destructors_clone = destructors.clone();
    if let Err(error) = ctrlc::set_handler(move || {
        run_destructors(&destructors_clone);
        exit(1);
    }) {
        // Log the error and proceed anyway.
        error!("{error}");
    }

    // Determine whether to print colored output.
    colored::control::set_override(io::stderr().is_terminal());

    // Set up the logger.
    set_up_logging();

    // Parse the command-line arguments.
    let settings = match settings() {
        Ok(settings) => settings,
        Err(error) => {
            error!("{error}");
            exit(1);
        }
    };

    // Try to load the state from disk.
    let (mut state, mut first_run) = state::load().map_or_else(
        |error| {
            // We couldn't load any state from disk. Log the error.
            warn!(
                "Unable to load state from disk. Proceeding with initial state. Details: {}",
                error.to_string().code_str(),
            );

            // Start with the initial state.
            (state::initial(), true)
        },
        |state| (state, false),
    );

    // Stream Docker events and vacuum when necessary. Restart if an error occurs.
    loop {
        // This will run until an error occurs (it never returns `Ok`).
        if let Err(error) = run(&settings, &mut state, &mut first_run, &destructors) {
            error!("{error}");
        }

        // Clean up any resources left over from that run.
        run_destructors(&destructors);

        // Wait a moment and then retry.
        info!("Retrying in 5 seconds\u{2026}");
        sleep(Duration::from_secs(5));
    }
}

#[cfg(test)]
mod tests {
    use super::Cli;
    use clap::CommandFactory;

    #[test]
    fn verify_cli() {
        Cli::command().debug_assert();
    }
}
