use crate::{
    format::CodeStr,
    state::{self, State},
    Settings,
};
use byte_unit::Byte;
use scopeguard::guard;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    io::{self, BufRead, BufReader},
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

// A Docker event (a line of output from `docker events --format '{{json .}}'`)
#[derive(Deserialize, Serialize, Debug)]
struct Event {
    #[serde(rename = "Type")]
    pub r#type: String,

    #[serde(rename = "Action")]
    pub action: String,

    #[serde(rename = "Actor")]
    pub actor: EventActor,

    pub id: String,
}

// A Docker event actor
#[derive(Deserialize, Serialize, Debug)]
struct EventActor {
    #[serde(rename = "Attributes")]
    pub attributes: EventActorAttributes,
}

// Docker event actor attributes
#[derive(Deserialize, Serialize, Debug)]
struct EventActorAttributes {
    pub image: Option<String>,
}

// A line of output from `docker system df --format '{{json .}}'`
#[derive(Deserialize, Serialize, Debug)]
struct SpaceRecord {
    #[serde(rename = "Type")]
    pub r#type: String,

    #[serde(rename = "Size")]
    pub size: String,
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

    // Interpret the output bytes as UTF-8 and trim any leading/trailing whitespace.
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

    // Interpret the output bytes as UTF-8 and collect the lines.
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

// Ask Docker for the IDs of the images currently in use by containers.
pub fn image_ids_in_use() -> io::Result<HashSet<String>> {
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
            "Unable to determine IDs of images currently in use by containers.",
        ));
    }

    // Interpret the output bytes as UTF-8 and collect the lines.
    String::from_utf8(output.stdout)
        .map_err(|error| io::Error::new(io::ErrorKind::Other, error))
        .map(|output| {
            output
                .lines()
                .filter_map(|line| {
                    if line.is_empty() {
                        None
                    } else {
                        match image_id(line.trim()) {
                            Ok(the_image_id) => Some(the_image_id),
                            Err(error) => {
                                // This failure may happen if a container was created from an
                                // image that no longer exists. This is non-fatal, so we just log
                                // the error and continue.
                                error!("{}", error);
                                None
                            }
                        }
                    }
                })
                .collect()
        })
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
            "Unable to determine the disk space used by Docker images.",
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
fn update_timestamp(state: &mut State, image_id: &str, verbose: bool) -> io::Result<()> {
    if verbose {
        info!(
            "Updating last-used timestamp for image {}\u{2026}",
            image_id.code_str(),
        );
    } else {
        debug!(
            "Updating last-used timestamp for image {}\u{2026}",
            image_id.code_str(),
        );
    }

    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => {
            state.images.insert(image_id.to_owned(), duration);
            Ok(())
        }
        Err(error) => Err(io::Error::new(io::ErrorKind::Other, error)),
    }
}

// The main vacuum logic
fn vacuum(state: &mut State, threshold: &Byte) -> io::Result<()> {
    // Inform the user that Docuum is receiving events from Docker.
    info!("Waking up\u{2026}");

    // Determine all the image IDs.
    let image_ids = image_ids()?;

    // Remove non-existent images from `state`.
    state.images.retain(|image_id, _| {
        if image_ids.contains(image_id) {
            true
        } else {
            debug!(
                "Removing record for non-existent image {}\u{2026}",
                image_id.code_str(),
            );
            false
        }
    });

    // In preparation fro the next step, pre-compute the timestamp
    // corresponding to the current time.
    let now_timestamp = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => Ok(duration),
        Err(error) => Err(io::Error::new(io::ErrorKind::Other, error)),
    }?;

    // Add any missing images to `state`.
    for image_id in &image_ids {
        state.images.entry(image_id.clone()).or_insert_with(|| {
            debug!(
                "Adding missing record for image {}\u{2026}",
                &image_id.code_str(),
            );

            now_timestamp
        });
    }

    // Update the timestamps of any images in use.
    for image_id in image_ids_in_use()? {
        update_timestamp(state, &image_id, false)?;
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
            "Docker images are currently using {} but the limit is {}. Some \
             images will be deleted.",
            space.get_appropriate_unit(false).to_string().code_str(),
            threshold.get_appropriate_unit(false).to_string().code_str(),
        );

        // Start deleting images, starting with the least recently used.
        for image_id in image_ids_vec {
            // Break if we're within the threshold.
            let new_space = space_usage()?;
            if new_space <= *threshold {
                info!(
                    "Docker images are now using {}, which is within the limit of {}.",
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
            "Docker images are using {}, which is within the limit of {}.",
            space.get_appropriate_unit(false).to_string().code_str(),
            threshold.get_appropriate_unit(false).to_string().code_str(),
        );
    }

    // Persist the state [tag:vacuum_persists_state].
    state::save(&state)?;

    // Inform the user that we're done for now.
    info!("Going back to sleep\u{2026}");

    Ok(())
}

// Stream Docker events and vacuum when necessary.
pub fn run(settings: &Settings, state: &mut State) -> io::Result<()> {
    // Run the main vacuum logic.
    vacuum(state, &settings.threshold)?;

    // Spawn `docker events --format '{{json .}}'`.
    let mut child = guard(
        Command::new("docker")
            .args(&["events", "--format", "{{json .}}"])
            .stdout(Stdio::piped())
            .spawn()?,
        |mut child| {
            let _ = child.kill();
        },
    );

    // Buffer the data as we read it line-by-line.
    let reader = BufReader::new(child.stdout.as_mut().map_or_else(
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

        // Get the ID of the image.
        let image_id = image_id(
            &if event.r#type == "container" && event.action == "destroy" {
                if let Some(image_name) = event.actor.attributes.image {
                    image_name
                } else {
                    debug!("Invalid Docker event.");
                    continue;
                }
            } else if event.r#type == "image"
                && (event.action == "import"
                    || event.action == "load"
                    || event.action == "pull"
                    || event.action == "push"
                    || event.action == "save"
                    || event.action == "tag")
            {
                event.id
            } else {
                debug!("Skipping due to irrelevance.");
                continue;
            },
        )?;

        // Update the timestamp for this image.
        update_timestamp(state, &image_id, true)?;

        // Run the main vacuum logic. This will also persist the state [ref:vacuum_persists_state].
        vacuum(state, &settings.threshold)?;
    }

    // The `for` loop above will only terminate if something happened to `docker events`.
    Err(io::Error::new(
        io::ErrorKind::Other,
        format!("{} unexpectedly terminated.", "docker events".code_str()),
    ))
}
