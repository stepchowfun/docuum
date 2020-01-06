mod format;
mod state;

use crate::format::CodeStr;
use atty::Stream;
use byte_unit::Byte;
use clap::{App, AppSettings, Arg};
use env_logger::{fmt::Color, Builder};
use log::{Level, LevelFilter};
use std::{
    env,
    io::{self, Write},
    process::exit,
    str::FromStr,
};

#[macro_use]
extern crate log;

// The program version
const VERSION: &str = env!("CARGO_PKG_VERSION");

// Defaults
const DEFAULT_LOG_LEVEL: LevelFilter = LevelFilter::Info;
const DEFAULT_CAPACITY: &str = "10 GiB";

// Command-line argument and option names
const CAPACITY_ARG: &str = "capacity";

// Set up the logger.
fn set_up_logging() {
    Builder::new()
        .filter_module(
            module_path!(),
            LevelFilter::from_str(
                &env::var("LOG_LEVEL").unwrap_or_else(|_| DEFAULT_LOG_LEVEL.to_string()),
            )
            .unwrap_or_else(|_| DEFAULT_LOG_LEVEL),
        )
        .format(|buf, record| {
            let mut style = buf.style();
            style.set_bold(true);
            match record.level() {
                Level::Error => {
                    style.set_color(Color::Red);
                }
                Level::Warn => {
                    style.set_color(Color::Yellow);
                }
                Level::Info => {
                    style.set_color(Color::Green);
                }
                Level::Debug | Level::Trace => {
                    style.set_color(Color::Blue);
                }
            }

            writeln!(
                buf,
                "{} {}",
                style.value(format!("[{}]", record.level())),
                record.args().to_string()
            )
        })
        .init();
}

// This struct represents the command-line arguments.
pub struct Settings {
    _capacity: Byte,
}

// Parse the command-line arguments.
fn settings() -> io::Result<Settings> {
    let matches = App::new("Docuum")
        .version(VERSION)
        .version_short("v")
        .author("Stephan Boyer <stephan@stephanboyer.com>")
        .about("Docuum performs LRU cache eviction for Docker images.")
        .setting(AppSettings::ColoredHelp)
        .setting(AppSettings::NextLineHelp)
        .setting(AppSettings::UnifiedHelpMessage)
        .arg(
            Arg::with_name(CAPACITY_ARG)
                .short("c")
                .long(CAPACITY_ARG)
                .value_name("CAPACITY")
                .help(&format!(
                    "Sets the maximum amount of space to be used for Docker images (default: {})",
                    DEFAULT_CAPACITY.code_str()
                ))
                .takes_value(true),
        )
        .get_matches();

    // Read the capacity.
    let default_capacity = Byte::from_str(DEFAULT_CAPACITY).unwrap(); // Manually verified safe
    let capacity = matches.value_of(CAPACITY_ARG).map_or_else(
        || Ok(default_capacity),
        |capacity| {
            Byte::from_str(capacity).map_err(|_| {
                io::Error::new(
                    io::ErrorKind::Other,
                    format!("Invalid capacity {}.", capacity.code_str()),
                )
            })
        },
    )?;

    Ok(Settings {
        _capacity: capacity,
    })
}

// Program entrypoint
fn entry() -> io::Result<()> {
    // Determine whether to print colored output.
    colored::control::set_override(atty::is(Stream::Stderr));

    // Set up the logger.
    set_up_logging();

    // Parse the command-line arguments;
    let _settings = settings()?;

    // Try to load the state from disk.
    info!("Attempting to load the state from disk\u{2026}");
    let state = state::load().unwrap_or_else(|error| {
        // We couldn't load any state from disk. Log the error.
        error!("Unable to load state from disk. Details: {}", error);

        // Start with the initial state.
        state::initial()
    });

    // Persist the state.
    info!("Persisting the state to disk\u{2026}");
    state::save(&state)?;

    Ok(())
}

// Let the fun begin!
fn main() {
    // Jump to the entrypoint and handle any resulting errors.
    if let Err(e) = entry() {
        error!("{}", e);
        exit(1);
    }
}
