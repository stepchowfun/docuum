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
    thread::sleep,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[macro_use]
extern crate log;

// The program version
const VERSION: &str = env!("CARGO_PKG_VERSION");

// Defaults
const DEFAULT_LOG_LEVEL: LevelFilter = LevelFilter::Info;
const DEFAULT_THRESHOLD: &str = "10 GB";

// Command-line argument and option names
const THRESHOLD_ARG: &str = "threshold";

// This struct represents the command-line arguments.
pub struct Settings {
    threshold: Byte,
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
                style.value(format!("[{} {}]", buf.timestamp(), record.level())),
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
            Arg::with_name(THRESHOLD_ARG)
                .short("t")
                .long(THRESHOLD_ARG)
                .value_name("THRESHOLD")
                .help(&format!(
                    "Sets the maximum amount of space to be used for Docker images (default: {})",
                    DEFAULT_THRESHOLD.code_str()
                ))
                .takes_value(true),
        )
        .get_matches();

    // Read the threshold.
    let default_threshold = Byte::from_str(DEFAULT_THRESHOLD).unwrap(); // Manually verified safe
    let threshold = matches.value_of(THRESHOLD_ARG).map_or_else(
        || Ok(default_threshold),
        |threshold| {
            Byte::from_str(threshold).map_err(|_| {
                io::Error::new(
                    io::ErrorKind::Other,
                    format!("Invalid threshold {}.", threshold.code_str()),
                )
            })
        },
    )?;

    Ok(Settings { threshold })
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
                                format!("Invalid threshold {}.", space_record.size.code_str()),
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

// Delete a Docker image.
fn delete_image(image_id: &str) -> io::Result<()> {
    info!("Deleting image {}\u{2026}", image_id.code_str());

    // Tell Docker to delete the image.
    let mut child = Command::new("docker")
        .args(&["image", "rm", "--force", "--no-prune", image_id])
        .spawn()?;

    // Ensure the command succeeded.
    if !child.wait()?.success() {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            format!("Unable to delete image {}.", image_id.code_str()),
        ));
    }

    Ok(())
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

// The main vacuum logic
fn vacuum(state: &mut State, threshold: &Byte) -> io::Result<()> {
    info!("Vacuuming\u{2026}");

    // Determine all the image IDs.
    let image_ids = image_ids()?;

    // Remove non-existent images from `state`.
    state.images.retain(|image_id, _| {
        debug!(
            "Removing record for non-existent image {}\u{2026}",
            image_id.code_str(),
        );
        image_ids.contains(image_id)
    });

    // Add any missing images to `state`.
    for image_id in &image_ids {
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

    // Sort the image IDs from least recently used to most recently used.
    let mut image_ids_vec = image_ids.iter().collect::<Vec<_>>();
    image_ids_vec.sort_by(|&x, &y| {
        // The two `unwrap`s here are safe by the construction of `image_ids_vec`.
        state
            .images
            .get(x)
            .unwrap()
            .cmp(state.images.get(y).unwrap())
    });

    // Check if we're over threshold.
    let space = space_usage()?;
    if space > *threshold {
        info!(
            "Some images need to be deleted. The images are currently taking up {} but the limit \
             is set to {}.",
            space.get_appropriate_unit(false).to_string().code_str(),
            threshold.get_appropriate_unit(false).to_string().code_str(),
        );

        // Start deleting images, starting with the least recently used.
        for image_id in image_ids_vec {
            // Break if we're under the threshold.
            let new_space = space_usage()?;
            if new_space <= *threshold {
                info!(
                    "The images are now taking up {}, which is under the limit of {}.",
                    new_space.get_appropriate_unit(false).to_string().code_str(),
                    threshold.get_appropriate_unit(false).to_string().code_str(),
                );
                break;
            }

            // Delete the image and continue.
            if let Err(error) = delete_image(image_id) {
                error!("{}", error);
            }
        }
    } else {
        info!(
            "The images are taking up {}, which is under the limit of {}.",
            space.get_appropriate_unit(false).to_string().code_str(),
            threshold.get_appropriate_unit(false).to_string().code_str(),
        );
    }

    // Persist the state.
    state::save(&state)
}

// Stream Docker events and vacuum when necessary.
fn run(settings: &Settings, state: &mut State) -> io::Result<()> {
    // Run the main vacuum logic.
    vacuum(state, &settings.threshold)?;

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
        update_timestamp(state, &image_id)?;

        // Run the main vacuum logic.
        vacuum(state, &settings.threshold)?;
    }

    // The `for` loop above will only terminate if something happened to `docker events`.
    Err(io::Error::new(
        io::ErrorKind::Other,
        format!("{} unexpectedly terminated.", "docker events".code_str()),
    ))
}

// Program entrypoint
fn entry() -> io::Result<()> {
    // Determine whether to print colored output.
    colored::control::set_override(atty::is(Stream::Stderr));

    // Set up the logger.
    set_up_logging();

    // Parse the command-line arguments;
    let settings = settings()?;

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

    // Stream Docker events and vacuum when necessary. Restart if an error occurs.
    loop {
        if let Err(e) = run(&settings, &mut state) {
            error!("{}", e);
            info!("Restarting\u{2026}");
            sleep(Duration::from_secs(1));
        }
    }
}

// Let the fun begin!
fn main() {
    // Jump to the entrypoint and handle any resulting errors.
    if let Err(e) = entry() {
        error!("{}", e);
        exit(1);
    }
}
