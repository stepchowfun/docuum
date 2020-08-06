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
    cell::RefCell,
    collections::{HashMap, HashSet},
    convert::TryInto,
    io,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::stream::StreamExt;

// Ask Docker for the ID of an image.
pub async fn image_id(docker: &Docker, image: &str) -> io::Result<String> {
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

pub struct DockerImageInfo {
    id: String,
    parent_id: String,
    created_since_epoch: Duration,
}

// Ask Docker for information about all images currently present.
pub async fn image_info(docker: &Docker) -> io::Result<Vec<DockerImageInfo>> {
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
            Ok(created) => Ok(DockerImageInfo {
                id: image.id,
                parent_id: image.parent_id,
                created_since_epoch: Duration::from_secs(created),
            }),
            Err(error) => Err(io::Error::new(io::ErrorKind::Other, error)),
        })
        .collect()
}

// Ask Docker for the IDs of the images currently in use by containers.
pub async fn image_ids_in_use(docker: &Docker) -> io::Result<HashSet<String>> {
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

struct ImageNode<'a> {
    docker_image: &'a DockerImageInfo,
    last_used_since_epoch: Duration,
    height: Option<u32>,
}

fn populate_node_height(nodes: &HashMap<&str, RefCell<ImageNode>>) {
    let mut node_stack = Vec::<&str>::new();
    'outer: for mut node_cell in nodes.values() {
        loop {
            let mut node = node_cell.borrow_mut();
            let parent_cell = if node.docker_image.parent_id == "" {
                None
            } else {
                nodes.get(node.docker_image.parent_id.as_str())
            };

            if parent_cell.is_none() {
                if node.docker_image.parent_id != "" {
                    warn!(
                        "Image {} has unknown parent {}",
                        node.docker_image.id.code_str(),
                        node.docker_image.parent_id.code_str()
                    );
                }

                node.height = Some(0);
            }

            if let Some(mut height) = node.height {
                loop {
                    match node_stack.pop() {
                        Some(image_id) => {
                            height = height.saturating_add(1);
                            nodes[image_id].borrow_mut().height = Some(height);
                        }
                        None => continue 'outer,
                    }
                }
            }

            node_stack.push(&node.docker_image.id);
            // This `unwrap` is safe because the above logic breaks out of the loop on None.
            node_cell = parent_cell.unwrap();
        }
    }
}

fn fix_node_timestamps(nodes: &HashMap<&str, RefCell<ImageNode>>) {
    for node in nodes.values() {
        let mut node = node.borrow_mut();
        loop {
            let parent = if node.docker_image.parent_id == "" {
                None
            } else {
                nodes.get(node.docker_image.parent_id.as_str())
            };

            if let Some(parent) = parent {
                let mut parent = parent.borrow_mut();
                if node.last_used_since_epoch <= parent.last_used_since_epoch {
                    break;
                }

                parent.last_used_since_epoch = node.last_used_since_epoch;
                node = parent;
            } else {
                break;
            }
        }
    }
}

// The main vacuum logic
async fn vacuum(docker: &Docker, state: &mut State, threshold: &Byte) -> io::Result<()> {
    // Inform the user that Docuum is receiving events from Docker.
    info!("Waking up\u{2026}");

    // Find all current images.
    let image_info = image_info(docker).await?;

    // Update the timestamps of any images in use.
    for image_id in image_ids_in_use(docker).await? {
        update_timestamp(state, &image_id, false)?;
    }

    let mut nodes = HashMap::with_capacity(image_info.len());
    for docker_image in &image_info {
        let last_used_since_epoch = if let Some(state_image) = state.images.get(&docker_image.id) {
            *state_image
        } else {
            debug!(
                "Adding missing record for image {}\u{2026}",
                &docker_image.id.code_str(),
            );

            state
                .images
                .insert(docker_image.id.clone(), docker_image.created_since_epoch);
            docker_image.created_since_epoch
        };

        nodes.insert(
            docker_image.id.as_str(),
            RefCell::new(ImageNode {
                docker_image,
                last_used_since_epoch,
                height: None,
            }),
        );
    }

    // Compute the height (total number of image dependencies) for each image.
    populate_node_height(&nodes);
    // Ensure that last usage time of each image is greater than or equal to the last usage time of every dependent image.
    fix_node_timestamps(&nodes);

    // Sort the images from least recently used to most recently used.
    // Break ties using the number of dependency layers.
    let mut image_ids_vec = nodes.iter().collect::<Vec<_>>();
    image_ids_vec.sort_by(|&x, &y| {
        let (x, y) = (x.1.borrow(), y.1.borrow());
        // The two `unwrap`s here are safe because height is set by `populate_node_height`.
        x.last_used_since_epoch
            .cmp(&y.last_used_since_epoch)
            .then(x.height.unwrap().cmp(&y.height.unwrap()).reverse())
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
        for (image_id, _) in image_ids_vec {
            // Delete the image.
            if let Err(error) = delete_image(docker, image_id).await {
                match error.kind() {
                    bollard::errors::ErrorKind::DockerResponseNotFoundError { .. }
                    | bollard::errors::ErrorKind::DockerResponseServerError { .. }
                    | bollard::errors::ErrorKind::DockerResponseConflictError { .. }
                    | bollard::errors::ErrorKind::DockerResponseNotModifiedError { .. } => {
                        error!("Unable to delete image {}: {}.", image_id.code_str(), error);

                        // We couldn't delete, so don't recompute space before attempting next image.
                        continue;
                    }
                    _ => {
                        // Docuum error? Docker is gone?
                        return Err(io::Error::new(
                            io::ErrorKind::Other,
                            format!(
                                "Unexpected error when deleting {}: {}.",
                                image_id.code_str(),
                                error
                            ),
                        ));
                    }
                }
            }

            // Forget about the deleted image.
            state.images.remove(image_id.to_owned());

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

    // Synchronize data back to state.
    state.images.retain(|image_id, last_used_since_epoch| {
        if let Some(node) = nodes.get(image_id.as_str()) {
            // Consider a image used if a dependent image was used.
            *last_used_since_epoch = node.borrow().last_used_since_epoch;

            true
        } else {
            debug!(
                "Removing record for non-existent image {}\u{2026}",
                image_id.code_str(),
            );
            false
        }
    });

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

        // Extract the event type and action. The Clippy directive is needed because Bollard uses
        // `_type` instead of `r#type` or `type_`. See
        // https://github.com/fussybeaver/bollard/issues/87 for details.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_timestamp_for_new_image() {
        const IMAGE_ID: &str = "3e7dc008597313a826cd0533ae7ead7af49a8a9a35711a239d5b7924bec4db1c";

        let mut state = State {
            images: HashMap::new(),
        };

        update_timestamp(&mut state, IMAGE_ID, true).expect("Update failed");

        let time = state.images.get(IMAGE_ID);
        assert!(time.is_some(), "Image was not added to state");
    }

    #[test]
    fn update_timestamp_for_existing_image() {
        const IMAGE_ID: &str = "3e7dc008597313a826cd0533ae7ead7af49a8a9a35711a239d5b7924bec4db1c";

        let mut state = State {
            images: vec![(IMAGE_ID.to_owned(), Duration::default())]
                .into_iter()
                .collect(),
        };

        update_timestamp(&mut state, IMAGE_ID, true).expect("Update failed");

        let time = state.images.get(IMAGE_ID);
        assert!(time.is_some(), "Image was not added to state");
        assert_ne!(&Duration::default(), time.unwrap());
    }

    #[test]
    fn populate_node_height_with_complete_data() {
        const NODE_1: &str = "488f2572ca8f6d9ab06284f1bdbe222323fae563d748ff71688480fc2d1a3c0b";
        const NODE_1_1: &str = "5efe99e2856f3be4fbc8bd7df7b243cc34972dee5251467b454ef91c40ee85f4";
        const NODE_1_1_1: &str = "1d1134b3c8b022eec5df7feb1a753e7a9084103fbdccc57726dcf53cd76e832d";
        const NODE_1_2: &str = "f9179b875976bbb530602cd3e46dfce53cab405feff828a79df18e1ceda4a89e";
        const NODE_2: &str = "67a1482a87761ac983be23f342c48a721a6a836203f8ce259dd7285bcfe29470";

        let infos = vec![
            DockerImageInfo {
                id: NODE_1.to_owned(),
                parent_id: String::default(),
                created_since_epoch: Duration::default(),
            },
            DockerImageInfo {
                id: NODE_1_1.to_owned(),
                parent_id: NODE_1.to_owned(),
                created_since_epoch: Duration::default(),
            },
            DockerImageInfo {
                id: NODE_1_1_1.to_owned(),
                parent_id: NODE_1_1.to_owned(),
                created_since_epoch: Duration::default(),
            },
            DockerImageInfo {
                id: NODE_1_2.to_owned(),
                parent_id: NODE_1.to_owned(),
                created_since_epoch: Duration::default(),
            },
            DockerImageInfo {
                id: NODE_2.to_owned(),
                parent_id: String::default(),
                created_since_epoch: Duration::default(),
            },
        ];

        let nodes = infos
            .iter()
            .map(|docker_image| {
                (
                    docker_image.id.as_str(),
                    RefCell::new(ImageNode {
                        docker_image,
                        last_used_since_epoch: Duration::default(),
                        height: None,
                    }),
                )
            })
            .collect();

        populate_node_height(&nodes);

        assert_eq!(Some(0), nodes[NODE_1].borrow().height);
        assert_eq!(Some(1), nodes[NODE_1_1].borrow().height);
        assert_eq!(Some(2), nodes[NODE_1_1_1].borrow().height);
        assert_eq!(Some(1), nodes[NODE_1_2].borrow().height);
        assert_eq!(Some(0), nodes[NODE_2].borrow().height);
    }

    #[test]
    fn populate_node_height_with_missing_parent() {
        const PARENT: &str = "94438846870b604603c351458f13a50afc3e70ce70d94f3953cc641240f628ef";
        const CHILD: &str = "af480962dde156510ae4234a4479c40d83db562ec9026f185be0c2432a9aa2ae";

        let infos = vec![DockerImageInfo {
            id: CHILD.to_owned(),
            parent_id: PARENT.to_owned(),
            created_since_epoch: Duration::default(),
        }];

        let nodes = infos
            .iter()
            .map(|docker_image| {
                (
                    docker_image.id.as_str(),
                    RefCell::new(ImageNode {
                        docker_image,
                        last_used_since_epoch: Duration::default(),
                        height: None,
                    }),
                )
            })
            .collect();

        populate_node_height(&nodes);

        assert_eq!(Some(0), nodes[CHILD].borrow().height);
    }

    #[test]
    fn fix_node_timestamps_with_complete_data() {
        const NODE_1: &str = "488f2572ca8f6d9ab06284f1bdbe222323fae563d748ff71688480fc2d1a3c0b";
        const NODE_1_1: &str = "5efe99e2856f3be4fbc8bd7df7b243cc34972dee5251467b454ef91c40ee85f4";
        const NODE_1_1_1: &str = "1d1134b3c8b022eec5df7feb1a753e7a9084103fbdccc57726dcf53cd76e832d";
        const NODE_1_2: &str = "f9179b875976bbb530602cd3e46dfce53cab405feff828a79df18e1ceda4a89e";
        const NODE_2: &str = "67a1482a87761ac983be23f342c48a721a6a836203f8ce259dd7285bcfe29470";

        let infos = vec![
            DockerImageInfo {
                id: NODE_1.to_owned(),
                parent_id: String::default(),
                created_since_epoch: Duration::from_secs(3),
            },
            DockerImageInfo {
                id: NODE_1_1.to_owned(),
                parent_id: NODE_1.to_owned(),
                created_since_epoch: Duration::from_secs(1),
            },
            DockerImageInfo {
                id: NODE_1_1_1.to_owned(),
                parent_id: NODE_1_1.to_owned(),
                created_since_epoch: Duration::from_secs(2),
            },
            DockerImageInfo {
                id: NODE_1_2.to_owned(),
                parent_id: NODE_1.to_owned(),
                created_since_epoch: Duration::from_secs(3),
            },
            DockerImageInfo {
                id: NODE_2.to_owned(),
                parent_id: String::default(),
                created_since_epoch: Duration::from_secs(1),
            },
        ];

        let nodes = infos
            .iter()
            .map(|docker_image| {
                (
                    docker_image.id.as_str(),
                    RefCell::new(ImageNode {
                        docker_image,
                        last_used_since_epoch: docker_image.created_since_epoch,
                        height: None,
                    }),
                )
            })
            .collect();

        fix_node_timestamps(&nodes);

        assert_eq!(
            Duration::from_secs(3),
            nodes[NODE_1].borrow().last_used_since_epoch
        );
        assert_eq!(
            Duration::from_secs(2),
            nodes[NODE_1_1].borrow().last_used_since_epoch
        );
        assert_eq!(
            Duration::from_secs(2),
            nodes[NODE_1_1_1].borrow().last_used_since_epoch
        );
        assert_eq!(
            Duration::from_secs(3),
            nodes[NODE_1_2].borrow().last_used_since_epoch
        );
        assert_eq!(
            Duration::from_secs(1),
            nodes[NODE_2].borrow().last_used_since_epoch
        );
    }

    #[test]
    fn fix_node_timestamps_with_missing_parent() {
        const PARENT: &str = "94438846870b604603c351458f13a50afc3e70ce70d94f3953cc641240f628ef";
        const CHILD: &str = "af480962dde156510ae4234a4479c40d83db562ec9026f185be0c2432a9aa2ae";

        let infos = vec![DockerImageInfo {
            id: CHILD.to_owned(),
            parent_id: PARENT.to_owned(),
            created_since_epoch: Duration::from_secs(1),
        }];

        let nodes = infos
            .iter()
            .map(|docker_image| {
                (
                    docker_image.id.as_str(),
                    RefCell::new(ImageNode {
                        docker_image,
                        last_used_since_epoch: docker_image.created_since_epoch,
                        height: None,
                    }),
                )
            })
            .collect();

        fix_node_timestamps(&nodes);

        assert_eq!(
            Duration::from_secs(1),
            nodes[CHILD].borrow().last_used_since_epoch
        );
    }
}
