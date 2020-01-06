mod format;
mod state;

use crate::format::CodeStr;
use atty::Stream;
use byte_unit::Byte;
use clap::{App, AppSettings, Arg};
use env_logger::{fmt::Color, Builder};
use log::{Level, LevelFilter};
use serde::{Deserialize, Serialize};
use std::{
    env,
    io::{self, BufRead, BufReader, Write},
    process::{exit, Command, Stdio},
    str::FromStr,
    time::{SystemTime, UNIX_EPOCH},
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

// Ask Docker for the ID of an image.
pub fn image_id(image: &str) -> io::Result<String> {
    // Query Docker for the image ID.
    let output = Command::new("docker")
        .args(&["image", "inspect", "--format", "{{.Id}}", image])
        .stderr(Stdio::inherit())
        .output()?;

    // Ensure the command succeeded.
    if !output.status.success() {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            format!("Unable to determine ID of image {}.", image.code_str()),
        ));
    }

    // Decode the output bytes into UTF-8 and trim any leading/trailing whitespace.
    String::from_utf8(output.stdout)
        .map(|output| output.trim().to_owned())
        .map_err(|error| io::Error::new(io::ErrorKind::Other, error))
}

// A Docker event
#[derive(Deserialize, Serialize, Debug)]
pub struct DockerEvent {
    #[serde(rename = "Type")]
    pub r#type: String,

    #[serde(rename = "Action")]
    pub action: String,

    #[serde(rename = "Actor")]
    pub actor: EventActor,
}

// A Docker event actor
#[derive(Deserialize, Serialize, Debug)]
pub struct EventActor {
    #[serde(rename = "Attributes")]
    pub attributes: EventAttributes,
}

// Docker event actor attributes
#[derive(Deserialize, Serialize, Debug)]
pub struct EventAttributes {
    pub image: String,
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
    let mut state = state::load().unwrap_or_else(|error| {
        // We couldn't load any state from disk. Log the error.
        info!(
            "Unable to load state from disk. Proceeding with initial state. Details: {}",
            error.to_string().code_str()
        );

        // Start with the initial state.
        state::initial()
    });

    // Spawn `docker events --format '{{json .}}'`.
    let child = Command::new("docker")
        .args(&["events", "--format", "{{json .}}"])
        .stdout(Stdio::piped())
        .spawn()?;

    // Buffered output
    let reader = BufReader::new(child.stdout.map_or_else(
        || {
            Err(io::Error::new(
                io::ErrorKind::Other,
                format!("Unable to read output from {}.", "docker events".code_str()),
            ))
        },
        Ok,
    )?);

    // Handle each incoming event.
    for line_option in reader.lines() {
        // Unwrap the line.
        let line = line_option?;
        debug!("Incoming event: {}", line.code_str());

        // Parse the line as an event.
        let event: DockerEvent = match serde_json::from_str(&line) {
            Ok(event) => {
                debug!("Parsed as: {}", format!("{:?}", event).code_str());
                event
            }
            Err(error) => {
                debug!("Skipping due to: {}", error);
                continue;
            }
        };

        // Check the event type and action.
        if event.r#type != "container" || event.action != "die" {
            debug!("Skipping due to irrelevance.");
            continue;
        }

        // Get the ID of the image.
        let image_id = image_id(&event.actor.attributes.image)?;

        // Update the timestamp for this image.
        info!(
            "Updating timestamp for image {}\u{2026}",
            image_id.code_str()
        );
        match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(duration) => {
                state.images.insert(image_id, duration);
                state::save(&state)?;
            }
            Err(error) => {
                return Err(io::Error::new(io::ErrorKind::Other, error));
            }
        }
    }

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
