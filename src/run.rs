use crate::{
    format::CodeStr,
    state::{self, State},
    Settings,
};
use bollard::{
    container::ListContainersOptions,
    image::{ListImagesOptions, RemoveImageOptions},
    system::EventsOptions,
    Docker,
};
use byte_unit::Byte;
use std::{
    collections::HashSet,
    convert::TryInto,
    io,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::stream::StreamExt;

// Ask Docker for the ID of an image.
pub async fn image_id(docker: &Docker, image: &str) -> io::Result<String> {
    // Query Docker for the image ID.
    let output = docker.inspect_image(image).await.map_err(|error| {
        io::Error::new(
            io::ErrorKind::Other,
            format!(
                "Unable to determine ID of image {}: {:?}.",
                image.code_str(),
                error
            ),
        )
    })?;

    Ok(output.id)
}

// Ask Docker for the IDs of all the images.
pub async fn image_ids(docker: &Docker) -> io::Result<HashSet<String>> {
    // Query Docker for the image IDs.
    let output = docker
        .list_images(Some(ListImagesOptions {
            all: true,
            ..ListImagesOptions::<String>::default()
        }))
        .await
        .map_err(|error| {
            io::Error::new(
                io::ErrorKind::Other,
                format!("Unable to determine IDs of all images: {:?}.", error),
            )
        })?;

    Ok(output.into_iter().map(|image| image.id).collect())
}

// Ask Docker for the IDs of the images currently in use by containers.
pub async fn image_ids_in_use(docker: &Docker) -> io::Result<HashSet<String>> {
    // Query Docker for the image IDs.
    let output = docker
        .list_containers(Some(ListContainersOptions {
            all: true,
            ..ListContainersOptions::<String>::default()
        }))
        .await
        .map_err(|error| {
            io::Error::new(
                io::ErrorKind::Other,
                format!(
                    "Unable to determine IDs of images currently in use by containers: {:?}.",
                    error
                ),
            )
        })?;

    Ok(output
        .into_iter()
        .filter_map(|container| container.image_id)
        .collect())
}

// Get the total space used by Docker images.
async fn space_usage(docker: &Docker) -> io::Result<Byte> {
    // Query Docker for the space usage.
    let output = docker.df().await.map_err(|error| {
        io::Error::new(
            io::ErrorKind::Other,
            format!(
                "Unable to determine the disk space used by Docker images: {:?}.",
                error
            ),
        )
    })?;

    Ok(Byte::from_bytes(
        output
            .layers_size
            // Assumes total size is non-negative.
            .map_or(0_u64, |size| size.try_into().unwrap())
            .into(),
    ))
}

// Delete a Docker image.
async fn delete_image(docker: &Docker, image_id: &str) -> io::Result<()> {
    info!("Deleting image {}\u{2026}", image_id.code_str());

    // Tell Docker to delete the image.
    if let Err(error) = docker
        .remove_image(
            image_id,
            Some(RemoveImageOptions {
                force: true,
                noprune: true,
            }),
            None,
        )
        .await
    {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            format!(
                "Unable to delete image {}: {:?}.",
                image_id.code_str(),
                error
            ),
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
async fn vacuum(docker: &Docker, state: &mut State, threshold: &Byte) -> io::Result<()> {
    // Inform the user that Docuum is receiving events from Docker.
    info!("Waking up\u{2026}");

    // Determine all the image IDs.
    let image_ids = image_ids(docker).await?;

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
    for image_id in image_ids_in_use(docker).await? {
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
    let space = space_usage(docker).await?;
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
            let new_space = space_usage(docker).await?;
            if new_space <= *threshold {
                info!(
                    "Docker images are now using {}, which is within the limit of {}.",
                    new_space.get_appropriate_unit(false).to_string().code_str(),
                    threshold.get_appropriate_unit(false).to_string().code_str(),
                );
                break;
            }

            // Delete the image and continue.
            if let Err(error) = delete_image(docker, image_id).await {
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
pub async fn run(settings: &Settings, state: &mut State) -> io::Result<()> {
    let docker = Docker::connect_with_local_defaults().map_err(|error| {
        io::Error::new(
            io::ErrorKind::Other,
            format!("Unable to connect to Docker: {:?}.", error),
        )
    })?;

    // Run the main vacuum logic.
    vacuum(&docker, state, &settings.threshold).await?;

    let mut events = docker.events(Option::<EventsOptions<String>>::None);
    loop {
        let event = match events.next().await {
            Some(event) => event,
            None => break,
        }
        .map_err(|error| {
            io::Error::new(
                io::ErrorKind::Other,
                format!("Unable to read Docker events: {:?}.", error),
            )
        })?;

        debug!("Incoming event: {:?}", event);

        // Bollard uses _type instead of r#type or type_: fussybeaver/bollard#87
        #[allow(clippy::used_underscore_binding)]
        let (r#type, action) = match (event._type, event.action) {
            (Some(r#type), Some(action)) => (r#type, action),
            _ => continue,
        };

        // Get the ID of the image.
        let image_id = image_id(
            &docker,
            &match (r#type.as_str(), action.as_str()) {
                ("container", "destroy") => {
                    if let Some(image_name) = event
                        .actor
                        .and_then(|actor| actor.attributes)
                        .and_then(|mut attributes| attributes.remove("image"))
                    {
                        image_name
                    } else {
                        debug!("Invalid Docker event.");
                        continue;
                    }
                }
                ("image", "import")
                | ("image", "load")
                | ("image", "pull")
                | ("image", "push")
                | ("image", "save")
                | ("image", "tag") => {
                    if let Some(id) = event.actor.and_then(|actor| actor.id) {
                        id
                    } else {
                        debug!("Invalid Docker event.");
                        continue;
                    }
                }
                _ => {
                    debug!("Skipping due to irrelevance.");
                    continue;
                }
            },
        )
        .await?;

        // Update the timestamp for this image.
        update_timestamp(state, &image_id, true)?;

        // Run the main vacuum logic. This will also persist the state [ref:vacuum_persists_state].
        vacuum(&docker, state, &settings.threshold).await?;
    }

    // The `for` loop above will only terminate if something happened to `docker events`.
    Err(io::Error::new(
        io::ErrorKind::Other,
        format!("{} unexpectedly terminated.", "docker events".code_str()),
    ))
}
