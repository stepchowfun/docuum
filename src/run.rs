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
    cmp::max,
    collections::{HashMap, HashSet},
    convert::TryInto,
    io,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::stream::StreamExt;

// This struct represents the information `docker image list` reports about an image.
#[derive(Clone, Debug)]
struct ImageInfo {
    id: String,
    parent_id: Option<String>,
    created_since_epoch: Duration,
}

// This struct represents a node in the image polyforest.
#[derive(Clone, Debug)]
struct ImageNode {
    image_info: ImageInfo,
    last_used_since_epoch: Duration,
    ancestors: usize, // 0 for images with no parent or missing parent
}

// Ask Docker for the ID of an image.
async fn image_id(docker: &Docker, image: &str) -> io::Result<String> {
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

// Ask Docker for information about all images currently present.
async fn list_images(docker: &Docker) -> io::Result<Vec<ImageInfo>> {
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

    output
        .into_iter()
        .map(|image| match image.created.try_into() {
            Ok(created) => Ok(ImageInfo {
                id: image.id,
                parent_id: if image.parent_id.is_empty() {
                    None
                } else {
                    Some(image.parent_id)
                },
                created_since_epoch: Duration::from_secs(created),
            }),
            Err(error) => Err(io::Error::new(
                io::ErrorKind::Other,
                format!(
                    "Unable to determine the creation timestamp of image {}: {:?}.",
                    image.id.code_str(),
                    error
                ),
            )),
        })
        .collect()
}

// Ask Docker for the IDs of the images currently in use by containers.
async fn image_ids_in_use(docker: &Docker) -> io::Result<HashSet<String>> {
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
            // The `unwrap` is safe assuming that the space usage is non-negative.
            .map_or(0_u64, |size| size.try_into().unwrap())
            .into(),
    ))
}

// Delete a Docker image.
async fn delete_image(docker: &Docker, image_id: &str) -> Result<(), bollard::errors::Error> {
    info!("Deleting image {}\u{2026}", image_id.code_str());

    docker
        .remove_image(
            image_id,
            Some(RemoveImageOptions {
                force: true,
                noprune: true,
            }),
            None,
        )
        .await?;

    Ok(())
}

// Update the timestamp for an image.
fn touch_image(state: &mut State, image_id: &str, verbose: bool) -> io::Result<()> {
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
        Err(error) => Err(io::Error::new(
            io::ErrorKind::Other,
            format!("Unable to compute the current timestamp: {:?}.", error),
        )),
    }
}

// The main vacuum logic
#[allow(clippy::too_many_lines)]
async fn vacuum(docker: &Docker, state: &mut State, threshold: &Byte) -> io::Result<()> {
    // Find all current images.
    let image_infos = list_images(docker).await?;

    // Find all images in use by containers.
    let image_ids_in_use = image_ids_in_use(docker).await?;

    // Construct a map from image ID to image info.
    let mut image_map = HashMap::new();
    for image_info in &image_infos {
        image_map.insert(image_info.id.clone(), image_info.clone());
    }

    // Construct a polyforest of image nodes that reflects the parent-child relationships.
    let mut image_graph = HashMap::new();
    for image_info in &image_infos {
        // Find the ancestors of the current image, including the current image itself, excluding
        // the root ancestor. The root will be added to the polyforest directly, and later we'll
        // add the other ancestors as well. The first postcondition is that, for every image in
        // this list except the final image, the parent is the subsequent image. The second
        // postcondition is that the parent of the final image has already been added to the
        // polyforest.
        let mut image_infos_to_add = vec![];
        let mut image_info = image_info.clone();
        loop {
            // Is the image already in the polyforest?
            if image_graph.contains_key(&image_info.id) {
                // The image has already been added.
                break;
            }

            // Does the image have a parent?
            if let Some(parent_id) = &image_info.parent_id {
                // The image has a parent, but does it actually exist?
                if let Some(parent_info) = image_map.get(parent_id) {
                    // It does. Add it to the list of images to add and continue.
                    image_infos_to_add.push(image_info.clone());
                    image_info = parent_info.clone();
                    continue;
                }
            }

            // The image is a root because either it has no parent or the parent doesn't exist.
            // Compute the last used date.
            let last_used_since_epoch =
                if let Some(last_used_since_epoch) = state.images.get(&image_info.id) {
                    *last_used_since_epoch
                } else {
                    image_info.created_since_epoch
                };

            // Add the image to the polyforest and break.
            image_graph.insert(
                image_info.id.clone(),
                ImageNode {
                    image_info: image_info.clone(),
                    last_used_since_epoch,
                    ancestors: 0,
                },
            );
            break;
        }

        // Add the ancestor images gathered above to the polyforest. We add them in order of
        // ancestor before descendant because we need to ensure the number of ancestors of the
        // parent has already been computed when computing that of the child.
        while let Some(image_info) = image_infos_to_add.pop() {
            // Look up the parent info. The first `unwrap` is safe because every image in the list
            // of images to add has a parent. The second `unwrap` is safe because the parent of the
            // last image to update is guaranteed to be in the polyforest.
            let parent_node = image_graph
                .get(&image_info.parent_id.clone().unwrap())
                .unwrap()
                .clone();

            // Compute the last used date. Ensure that it's at least as recent as that of the
            // parent.
            let mut last_used_since_epoch =
                if let Some(last_used_since_epoch) = state.images.get(&image_info.id) {
                    *last_used_since_epoch
                } else {
                    image_info.created_since_epoch
                };

            // If the image is in use by a container, update its timestamp.
            if image_ids_in_use.contains(&image_info.id) {
                last_used_since_epoch = max(
                    last_used_since_epoch,
                    match SystemTime::now().duration_since(UNIX_EPOCH) {
                        Ok(duration) => Ok(duration),
                        Err(error) => Err(io::Error::new(
                            io::ErrorKind::Other,
                            format!("Unable to compute the current timestamp: {:?}.", error),
                        )),
                    }?,
                );
            }

            // Add the image.
            image_graph.insert(
                image_info.id.clone(),
                ImageNode {
                    image_info: image_info.clone(),
                    last_used_since_epoch,
                    ancestors: parent_node.ancestors + 1,
                },
            );
        }
    }

    // We need to ensure that each node's last used timestamp is at least as recent as those of all
    // the transitive descendants. We'll do this via a breadth first traversal, starting with the
    // leaves. Since we're working with an acyclic graph, we don't need to keep track of a
    // "visited" set like we usually would for BFS.
    let mut frontier = image_graph.keys().cloned().collect::<HashSet<_>>();
    for image_node in image_graph.values() {
        if let Some(parent_id) = &image_node.image_info.parent_id {
            frontier.remove(parent_id);
        }
    }

    // In each iteration of the outer loop, we traverse the frontier and build up a new frontier
    // for the subsequent iteration.
    while !frontier.is_empty() {
        let mut new_frontier = HashSet::new();

        for image_id in frontier {
            if let Some(image_node) = image_graph.get(&image_id).cloned() {
                if let Some(parent_id) = &image_node.image_info.parent_id {
                    if let Some(parent_node) = image_graph.get_mut(parent_id) {
                        parent_node.last_used_since_epoch = max(
                            parent_node.last_used_since_epoch,
                            image_node.last_used_since_epoch,
                        );
                        new_frontier.insert(parent_node.image_info.id.clone());
                    }
                }
            }
        }

        frontier = new_frontier;
    }

    // Sort the images from least recently used to most recently used.
    // Break ties using the number of dependency layers.
    let mut sorted_image_nodes = image_graph.values().collect::<Vec<_>>();
    sorted_image_nodes.sort_by(|x, y| {
        x.last_used_since_epoch
            .cmp(&y.last_used_since_epoch)
            .then(y.ancestors.cmp(&x.ancestors))
    });

    // Check if we're over the threshold.
    let mut deleted_image_ids = HashSet::new();
    let space = space_usage(docker).await?;
    if space > *threshold {
        info!(
            "Docker images are currently using {} but the limit is {}. Some \
             images will be deleted.",
            space.get_appropriate_unit(false).to_string().code_str(),
            threshold.get_appropriate_unit(false).to_string().code_str(),
        );

        // Start deleting images, beginning with the least recently used.
        for image_node in sorted_image_nodes {
            // Delete the image.
            if let Err(error) = delete_image(docker, &image_node.image_info.id).await {
                // We distinguish between two types of errors.
                // 1. For some known types of errors, we can be certain that the image was not
                //    deleted, so we skip recomputing the space usage (which is expensive).
                // 2. For all other types of errors, we err on the safe side and recompute the
                //    space usage.
                match error {
                    bollard::errors::Error::DockerResponseNotFoundError { .. }
                    | bollard::errors::Error::DockerResponseConflictError { .. } => {
                        // We know with certainty the image wasn't deleted, so we skip recomputing
                        // the space usage.
                        error!("{}", error);
                        continue;
                    }
                    _ => {
                        // There was some unknown error, but we can't assume the deletion didn't
                        // actually happen.
                        return Err(io::Error::new(
                            io::ErrorKind::Other,
                            format!(
                                "Unexpected error when deleting image {}: {}.",
                                image_node.image_info.id.code_str(),
                                error
                            ),
                        ));
                    }
                }
            }

            // Forget about the deleted image.
            deleted_image_ids.insert(image_node.image_info.id.clone());

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
        }
    } else {
        info!(
            "Docker images are using {}, which is within the limit of {}.",
            space.get_appropriate_unit(false).to_string().code_str(),
            threshold.get_appropriate_unit(false).to_string().code_str(),
        );
    }

    // Update the state.
    state.images.clear();
    for image_node in image_graph.values() {
        if !deleted_image_ids.contains(&image_node.image_info.id) {
            state.images.insert(
                image_node.image_info.id.clone(),
                image_node.last_used_since_epoch,
            );
        }
    }

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

    info!("Performing an initial vacuum on startup\u{2026}");
    vacuum(&docker, state, &settings.threshold).await?;
    state::save(&state)?;

    info!("Listening for Docker events\u{2026}");
    let mut events = docker.events(Option::<EventsOptions<String>>::None);
    loop {
        // Wait until there's an event to handle.
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

        // Extract the event type and action.
        let (r#type, action) = match (event.typ, event.action) {
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

        // Inform the user that we're about to vacuum.
        info!("Waking up\u{2026}");

        // Update the timestamp for this image.
        touch_image(state, &image_id, true)?;

        // Run the main vacuum logic.
        vacuum(&docker, state, &settings.threshold).await?;

        // Persist the state.
        state::save(&state)?;

        // Inform the user that we're done for now.
        info!("Going back to sleep\u{2026}");
    }

    // The `for` loop above will only terminate if something happened to `docker events`.
    Err(io::Error::new(
        io::ErrorKind::Other,
        format!("{} unexpectedly terminated.", "docker events".code_str()),
    ))
}
