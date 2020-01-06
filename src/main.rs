mod docker;
mod format;
mod state;

use crate::{
    docker::{Event, SpaceRecord},
    format::CodeStr,
    state::State,
};
use atty::Stream;
use byte_unit::Byte;
use clap::{App, AppSettings, Arg};
use env_logger::{fmt::Color, Builder};
use log::{Level, LevelFilter};
use std::{
    collections::HashSet,
    env,
    io::{self, BufRead, BufReader, Write},
    process::{exit, Command, Stdio},
    str::FromStr,
    time::{Duration, SystemTime, UNIX_EPOCH},
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

// This struct represents the command-line arguments.
pub struct Settings {
    _capacity: Byte,
}

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
        .args(&["image", "inspect", "--format", "{{.ID}}", image])
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

// Ask Docker for the IDs of all the images.
pub fn image_ids() -> io::Result<HashSet<String>> {
    // Query Docker for the image IDs.
    let output = Command::new("docker")
        .args(&["image", "ls", "--all", "--no-trunc", "--format", "{{.ID}}"])
        .stderr(Stdio::inherit())
        .output()?;

    // Ensure the command succeeded.
    if !output.status.success() {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            "Unable to determine IDs of all images.",
        ));
    }

    // Decode the output bytes into UTF-8 and collect the lines.
    String::from_utf8(output.stdout)
        .map(|output| {
            output
                .lines()
                .map(|line| line.trim().to_owned())
                .filter(|line| !line.is_empty())
                .collect()
        })
        .map_err(|error| io::Error::new(io::ErrorKind::Other, error))
}

// Ask Docker for the images used by any existing containers. It's not clear from the Docker
// documentation whether the results are image IDs (vs. tags or something else), so don't assume
// the results are IDs.
pub fn images_in_use() -> io::Result<HashSet<String>> {
    // Query Docker for the image IDs.
    let output = Command::new("docker")
        .args(&[
            "container",
            "ls",
            "--all",
            "--no-trunc",
            "--format",
            "{{.Image}}",
        ])
        .stderr(Stdio::inherit())
        .output()?;

    // Ensure the command succeeded.
    if !output.status.success() {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            "Unable to determine IDs of all images.",
        ));
    }

    // Decode the output bytes into UTF-8 and collect the lines.
    String::from_utf8(output.stdout)
        .map(|output| {
            output
                .lines()
                .map(|line| line.trim().to_owned())
                .filter(|line| !line.is_empty())
                .collect()
        })
        .map_err(|error| io::Error::new(io::ErrorKind::Other, error))
}

// Update the timestamp for an image.
fn update_timestamp(state: &mut State, image_id: &str) -> io::Result<()> {
    info!(
        "Updating last-used timestamp for image {}\u{2026}",
        image_id.code_str(),
    );
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => {
            state.images.insert(image_id.to_owned(), duration);
            state::save(&state)
        }
        Err(error) => Err(io::Error::new(io::ErrorKind::Other, error)),
    }
}

// Get the total space used by Docker images.
fn space_usage() -> io::Result<Byte> {
    // Query Docker for the space usage.
    let output = Command::new("docker")
        .args(&["system", "df", "--format", "{{json .}}"])
        .stderr(Stdio::inherit())
        .output()?;

    // Ensure the command succeeded.
    if !output.status.success() {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            "Unable to determine IDs of all images.",
        ));
    }

    // Find the relevant line of output.
    String::from_utf8(output.stdout)
        .map_err(|error| io::Error::new(io::ErrorKind::Other, error))
        .and_then(|output| {
            for line in output.lines() {
                // Parse the line as a space record.
                if let Ok(space_record) = serde_json::from_str::<SpaceRecord>(&line) {
                    // Return early if we found the record we're looking for.
                    if space_record.r#type == "Images" {
                        return Byte::from_str(&space_record.size).map_err(|_| {
                            io::Error::new(
                                io::ErrorKind::Other,
                                format!("Invalid capacity {}.", space_record.size.code_str()),
                            )
                        });
                    }
                }
            }

            Err(io::Error::new(
                io::ErrorKind::Other,
                format!(
                    "Unable to parse output of {}: {}",
                    "docker system df".code_str(),
                    output.code_str(),
                ),
            ))
        })
}

// The main vacuum logic
fn vacuum(state: &mut State) -> io::Result<()> {
    info!("Vacuuming\u{2026}");

    // Remove non-existent images from `state`.
    let image_ids = image_ids()?;
    state.images.retain(|image_id, _| {
        debug!(
            "Removing record for non-existent image {}\u{2026}",
            image_id.code_str(),
        );
        image_ids.contains(image_id)
    });

    // Add any missing images to `state`.
    for image_id in image_ids {
        state.images.entry(image_id.clone()).or_insert_with(|| {
            debug!(
                "Adding record for missing image {}\u{2026}",
                &image_id.code_str(),
            );
            Duration::new(0, 0)
        });
    }

    // Update the timestamps of any images in use.
    for image in images_in_use()? {
        // Get the ID of the image.
        let image_id = image_id(&image)?;

        // Update the timestamp for this image.
        update_timestamp(state, &image_id)?;
    }

    // TODO: Prune!
    info!(
        "Docker images are currently occupying {} bytes.",
        space_usage()?.to_string().code_str(),
    );

    // Persist the state.
    state::save(&state)
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
        debug!(
            "Unable to load state from disk. Proceeding with initial state. Details: {}",
            error.to_string().code_str()
        );

        // Start with the initial state.
        state::initial()
    });

    // Run the main vacuum logic.
    vacuum(&mut state)?;

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
    info!("Listening for Docker events\u{2026}");
    for line_option in reader.lines() {
        // Unwrap the line.
        let line = line_option?;
        debug!("Incoming event: {}", line.code_str());

        // Parse the line as an event.
        let event = match serde_json::from_str::<Event>(&line) {
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
        update_timestamp(&mut state, &image_id)?;

        // Run the main vacuum logic.
        vacuum(&mut state)?;
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
