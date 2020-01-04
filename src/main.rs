mod failure;
mod format;

use crate::{failure::Failure, format::CodeStr};
use atty::Stream;
use byte_unit::Byte;
use clap::{App, AppSettings, Arg};
use env_logger::{fmt::Color, Builder};
use log::{Level, LevelFilter};
use std::{env, io::Write, process::exit, str::FromStr};

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
fn settings() -> Result<Settings, Failure> {
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
                .help("Sets the maximum amount of space used for Docker images (e.g., 30 GB)")
                .takes_value(true),
        )
        .get_matches();

    // Read the capacity.
    let default_capacity = Byte::from_str(DEFAULT_CAPACITY).unwrap(); // Manually verified safe
    let capacity = matches.value_of(CAPACITY_ARG).map_or_else(
        || Ok(default_capacity),
        |capacity| {
            Byte::from_str(capacity).map_err(|_| {
                Failure::User(format!("Invalid capacity {}.", capacity.code_str()), None)
            })
        },
    )?;

    Ok(Settings {
        _capacity: capacity,
    })
}

// Program entrypoint
fn entry() -> Result<(), Failure> {
    // Determine whether to print colored output.
    colored::control::set_override(atty::is(Stream::Stderr));

    // Set up the logger.
    set_up_logging();

    // Parse the command-line arguments;
    let _settings = settings()?;

    info!("Hello, world!");
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
