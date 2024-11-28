use {
    crate::{
        format::CodeStr,
        state::{self, State},
        Settings, Threshold,
    },
    byte_unit::Byte,
    chrono::DateTime,
    regex::RegexSet,
    serde::{Deserialize, Serialize},
    std::{
        cmp::max,
        collections::{hash_map::Entry, HashMap, HashSet},
        io::{self, BufRead, BufReader},
        ops::Deref,
        process::{Command, Stdio},
        sync::{Arc, Mutex},
        time::{Duration, SystemTime, UNIX_EPOCH},
    },
};

#[cfg(target_os = "linux")]
use {
    std::path::{Path, PathBuf},
    sysinfo::{Disk, DiskExt, RefreshKind, System, SystemExt},
};

// When querying Docker for the image IDs corresponding to a list of container IDs, this is the
// maximum number of container IDs to query at once.
const CONTAINER_IDS_CHUNK_SIZE: usize = 100;

// A Docker event (a line of output from `docker events --format '{{json .}}'`)
#[derive(Deserialize, Serialize, Debug)]
struct Event {
    #[serde(rename = "Type")]
    r#type: String,

    #[serde(rename = "Action")]
    action: String,

    #[serde(rename = "Actor")]
    actor: EventActor,

    id: String,
}

// A Docker event actor
#[derive(Deserialize, Serialize, Debug)]
struct EventActor {
    #[serde(rename = "Attributes")]
    attributes: EventActorAttributes,
}

// Docker event actor attributes
#[derive(Deserialize, Serialize, Debug)]
struct EventActorAttributes {
    image: Option<String>,
}

// A line of output from `docker system df --format '{{json .}}'`
#[derive(Deserialize, Serialize, Debug)]
struct SpaceRecord {
    #[serde(rename = "Type")]
    r#type: String,

    #[serde(rename = "Size")]
    size: String,
}

// Each image may be associated with multiple of these repository-tag pairs. Docker will always
// report at least one repository-tag pair for each image. For untagged images, `tag` will be
// `<none>`, and `repository` may also take on that value [tag:at_least_one_repository_tag].
#[derive(Clone, Debug, Eq, PartialEq)]
struct RepositoryTag {
    repository: String,
    tag: String,
}

// This is the information Docker reports about each image when listing images. Note that the image
// ID is not included here because this struct will be used as the value type for a `HashMap` for
// which the key type is the image ID.
#[derive(Clone, Debug, Eq, PartialEq)]
struct ImageRecord {
    parent_id: Option<String>,
    created_since_epoch: Duration,
    repository_tags: Vec<RepositoryTag>, // [ref:at_least_one_repository_tag]
}

// This is a node in the image polyforest. Note that the image ID is not included here because this
// struct will be used as the value type for a `HashMap` for which the key type is the image ID.
#[derive(Clone, Debug, Eq, PartialEq)]
struct ImageNode {
    image_record: ImageRecord,
    last_used_since_epoch: Duration,
    ancestors: usize, // 0 for images with no parent or missing parent
}

// Ask Docker for the ID of an image.
fn image_id(image: &str) -> io::Result<String> {
    // Query Docker for the image ID.
    let output = Command::new("docker")
        .args(["image", "inspect", "--format", "{{.ID}}", image])
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

// Get the ID of the parent of an image (if the parent exists), querying Docker if necessary.
fn parent_id(state: &State, image_id: &str) -> io::Result<Option<String>> {
    // If we already know the parent, just return it.
    if let Some(image) = state.images.get(image_id) {
        return Ok(image.parent_id.clone());
    }

    // Query Docker for the parent image ID.
    let output = Command::new("docker")
        .args(["image", "inspect", "--format", "{{.Parent}}", image_id])
        .stderr(Stdio::inherit())
        .output()?;

    // Ensure the command succeeded.
    if !output.status.success() {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            format!(
                "Unable to determine ID of the parent of image {}.",
                image_id.code_str(),
            ),
        ));
    }

    // Interpret the output bytes as UTF-8 and trim any leading/trailing whitespace.
    String::from_utf8(output.stdout)
        .map(|output| {
            let trimmed_output = output.trim();

            // Does the image even have a parent?
            if trimmed_output.is_empty() {
                None
            } else {
                Some(trimmed_output.to_owned())
            }
        })
        .map_err(|error| io::Error::new(io::ErrorKind::Other, error))
}

// Query Docker for all the images.
fn list_image_records(state: &State) -> io::Result<HashMap<String, ImageRecord>> {
    // Get the IDs and creation timestamps of all the images.
    let output = Command::new("docker")
        .args([
            "image",
            "ls",
            "--all",
            "--no-trunc",
            "--format",
            "{{.ID}}\\t{{.Repository}}\\t{{.Tag}}\\t{{.CreatedAt}}",
        ])
        .stderr(Stdio::inherit())
        .output()?;

    // Ensure the command succeeded.
    if !output.status.success() {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            "Unable to list images.",
        ));
    }

    // Interpret the output bytes as UTF-8 and parse the lines.
    let mut image_records = HashMap::<_, ImageRecord>::new();
    for line in String::from_utf8(output.stdout)
        .map_err(|error| io::Error::new(io::ErrorKind::Other, error))?
        .lines()
    {
        let trimmed_line = line.trim();

        if trimmed_line.is_empty() {
            continue;
        }

        let image_parts = trimmed_line.split('\t').collect::<Vec<_>>();
        if let [id, repository, tag, date_str] = image_parts[..] {
            let repository_tag = RepositoryTag {
                repository: repository.to_owned(),
                tag: tag.to_owned(),
            };

            match image_records.entry(id.to_owned()) {
                Entry::Occupied(mut entry) => {
                    (entry.get_mut()).repository_tags.push(repository_tag);
                }
                Entry::Vacant(entry) => {
                    entry.insert(ImageRecord {
                        parent_id: parent_id(state, id)?,
                        created_since_epoch: parse_docker_date(date_str)?,
                        repository_tags: vec![repository_tag],
                    });
                }
            }
        } else {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "Failed to parse image list output from Docker.",
            ));
        }
    }

    Ok(image_records)
}

// Ask Docker for the IDs of the images currently in use by containers.
fn image_ids_in_use() -> io::Result<HashSet<String>> {
    // Query Docker for all the container IDs.
    let container_ids_output = Command::new("docker")
        .args([
            "container",
            "ls",
            "--all",
            "--no-trunc",
            "--format",
            "{{.ID}}",
        ])
        .stderr(Stdio::inherit())
        .output()?;

    // Ensure the command succeeded.
    if !container_ids_output.status.success() {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            "Unable to determine IDs of images currently in use by containers.",
        ));
    }

    // Interpret the output bytes as UTF-8 and parse the lines.
    let container_ids = String::from_utf8(container_ids_output.stdout)
        .map_err(|error| io::Error::new(io::ErrorKind::Other, error))
        .map(|output| {
            output
                .lines()
                .filter_map(|line| {
                    let trimmed_line = line.trim();

                    if trimmed_line.is_empty() {
                        None
                    } else {
                        Some(trimmed_line.to_owned())
                    }
                })
                .collect::<Vec<_>>()
        })?;

    // Group the container IDs into chunks and query Docker for the image IDs for each chunk.
    let mut image_ids = HashSet::new();
    for chunk in container_ids.chunks(CONTAINER_IDS_CHUNK_SIZE) {
        // Query Docker for the image IDs for this chunk.
        let image_ids_output = Command::new("docker")
            .args(
                ["container", "inspect", "--format", "{{.Image}}"]
                    .iter()
                    .map(Deref::deref)
                    .chain(chunk.iter().map(AsRef::as_ref)),
            )
            .stderr(Stdio::inherit())
            .output()?;

        // Ensure the command succeeded.
        if !image_ids_output.status.success() {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "Unable to determine IDs of images currently in use by containers.",
            ));
        }

        // Interpret the output bytes as UTF-8 and parse the lines.
        image_ids.extend(
            String::from_utf8(image_ids_output.stdout)
                .map_err(|error| io::Error::new(io::ErrorKind::Other, error))
                .map(|output| {
                    output
                        .lines()
                        .filter_map(|line| {
                            let trimmed_line = line.trim();

                            if trimmed_line.is_empty() {
                                None
                            } else {
                                Some(trimmed_line.to_owned())
                            }
                        })
                        .collect::<Vec<_>>()
                })?,
        );
    }

    Ok(image_ids)
}

// Determine Docker's root directory.
#[cfg(target_os = "linux")]
fn docker_root_dir() -> io::Result<PathBuf> {
    // Query Docker for it.
    let output = Command::new("docker")
        .args(["info", "--format", "{{.DockerRootDir}}"])
        .stderr(Stdio::inherit())
        .output()?;

    // Ensure the command succeeded.
    if !output.status.success() {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            "Unable to determine the Docker root directory.",
        ));
    }

    // Trim the output.
    String::from_utf8(output.stdout)
        .map(|s| PathBuf::from(s.trim()))
        .map_err(|error| io::Error::new(io::ErrorKind::Other, error))
}

// Find the disk containing a path.
#[cfg(target_os = "linux")]
fn get_disk_by_file<'a>(disks: &'a [Disk], path: &Path) -> io::Result<&'a Disk> {
    disks
        .iter()
        .filter(|d| path.starts_with(d.mount_point()))
        .max_by_key(|d| d.mount_point().as_os_str().len())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::Other,
                format!(
                    "Unable to find disk for path {}.",
                    path.to_string_lossy().code_str(),
                ),
            )
        })
}

// Find size of filesystem on which docker root directory is stored.
#[cfg(target_os = "linux")]
fn docker_root_dir_filesystem_size() -> io::Result<Byte> {
    let root_dir = docker_root_dir()?;
    let system = System::new_with_specifics(RefreshKind::new().with_disks_list());
    let disks = system.disks();
    let disk = get_disk_by_file(disks, &root_dir)?;
    Ok(Byte::from(disk.total_space()))
}

// Get the total space used by Docker images.
#[allow(clippy::map_err_ignore)]
fn space_usage() -> io::Result<Byte> {
    // Query Docker for the space usage.
    let output = Command::new("docker")
        .args(["system", "df", "--format", "{{json .}}"])
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
                if let Ok(space_record) = serde_json::from_str::<SpaceRecord>(line) {
                    // Return early if we found the record we're looking for.
                    if space_record.r#type == "Images" {
                        return Byte::from_str(&space_record.size).map_err(|_| {
                            io::Error::new(
                                io::ErrorKind::Other,
                                format!(
                                    "Unable to parse {} from {}.",
                                    space_record.size.code_str(),
                                    "docker system df".code_str(),
                                ),
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
fn delete_image(image: &str) -> io::Result<()> {
    info!("Deleting image {}\u{2026}", image.code_str());

    // Tell Docker to delete the image.
    let mut child = Command::new("docker")
        .args(["image", "rm", "--force", "--no-prune", image])
        .spawn()?;

    // Ensure the command succeeded.
    if !child.wait()?.success() {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            format!("Unable to delete image {}.", image.code_str()),
        ));
    }

    Ok(())
}

// Update the timestamp for an image.
// Returns a boolean indicating if a new entry was created for the image.
fn touch_image(state: &mut State, image_id: &str, verbose: bool) -> io::Result<bool> {
    if verbose {
        debug!(
            "Updating last-used timestamp for image {}\u{2026}",
            image_id.code_str(),
        );
    } else {
        trace!(
            "Updating last-used timestamp for image {}\u{2026}",
            image_id.code_str(),
        );
    }

    // Get the current timestamp.
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => {
            // Store the image metadata in the state.
            Ok(state
                .images
                .insert(
                    image_id.to_owned(),
                    state::Image {
                        parent_id: parent_id(state, image_id)?,
                        last_used_since_epoch: duration,
                    },
                )
                .is_none())
        }
        Err(error) => Err(io::Error::new(
            io::ErrorKind::Other,
            format!("Unable to compute the current timestamp: {error:?}."),
        )),
    }
}

// Parse the non-standard timestamp format Docker uses for `docker image ls`.
// Example input: "2017-12-20 16:30:49 -0500 EST".
fn parse_docker_date(timestamp: &str) -> io::Result<Duration> {
    // Chrono can't read the "EST", so remove it before parsing.
    let timestamp_without_timezone_triad =
        timestamp.trim().rsplitn(2, ' ').last().ok_or_else(|| {
            io::Error::new(io::ErrorKind::Other, "Failed to remove timezone string.")
        })?;

    // Parse the date and convert it into a duration since the UNIX epoch.
    let duration =
        match DateTime::parse_from_str(timestamp_without_timezone_triad, "%Y-%m-%d %H:%M:%S %z") {
            Ok(datetime) => {
                datetime.signed_duration_since::<chrono::offset::Utc>(DateTime::from(UNIX_EPOCH))
            }
            Err(error) => return Err(io::Error::new(io::ErrorKind::Other, error)),
        };

    // Convert the duration into a `std::time::Duration`. If the duration is negative, it will be
    // clamped to zero. This can occur when building images with `kaniko --reproducible`, as the
    // resulting images have `0001-01-01 00:00:00 +0000 UTC` for their creation timestamp.
    Ok(duration.to_std().unwrap_or(Duration::ZERO))
}

// Construct a polyforest of image nodes that reflects their parent-child relationships.
fn construct_polyforest(
    state: &State,
    first_run: bool,
    image_records: &HashMap<String, ImageRecord>,
    image_ids_in_use: &HashSet<String>,
) -> io::Result<HashMap<String, ImageNode>> {
    // Compute the current timestamp.
    let time_since_epoch = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => Ok(duration),
        Err(error) => Err(io::Error::new(
            io::ErrorKind::Other,
            format!("Unable to compute the current timestamp: {error:?}."),
        )),
    }?;

    // Construct the graph. It's a map, just like `image_records`, except the values are
    // `ImageNode`s rather than `ImageRecord`s. The majority of this code exists just to compute
    // the number of ancestors for each node.
    let mut polyforest = HashMap::new();
    for (image_id, image_record) in image_records {
        // Find the ancestors of the current image, including the current image itself.
        let mut image_ids_and_records_to_add = vec![];
        let mut image_id_and_record = (image_id.clone(), image_record.clone());
        loop {
            // Is the image already in the polyforest?
            if polyforest.contains_key(&image_id_and_record.0) {
                // The image has already been added.
                break;
            }

            // Schedule the image for addition in the polyforest.
            image_ids_and_records_to_add.push(image_id_and_record.clone());

            // Does the image have a parent?
            if let Some(parent_id) = &image_id_and_record.1.parent_id {
                // The image has a parent, but does it actually exist?
                if let Some(parent_image_record) = image_records.get(parent_id) {
                    // It does. Advance to the parent and continue.
                    image_id_and_record = (parent_id.clone(), parent_image_record.clone());
                    continue;
                }
            }

            // If we made it this far, the image is a root.
            break;
        }

        // Add the ancestor images gathered above to the polyforest. We add them in order of
        // ancestor before descendant because we need to ensure the number of ancestors of the
        // parent has already been computed when computing that of the child.
        while let Some(image_id_and_record_to_add) = image_ids_and_records_to_add.pop() {
            // Compute the last used date.
            let mut last_used_since_epoch = state.images.get(&image_id_and_record_to_add.0).map_or(
                if first_run {
                    image_id_and_record_to_add.1.created_since_epoch
                } else {
                    time_since_epoch
                },
                |image| image.last_used_since_epoch,
            );

            // If the image is in use by a container, update its timestamp.
            if image_ids_in_use.contains(&image_id_and_record_to_add.0) {
                last_used_since_epoch = max(last_used_since_epoch, time_since_epoch);
            }

            // Compute the number of ancestors.
            let ancestors =
                image_id_and_record_to_add
                    .1
                    .parent_id
                    .as_ref()
                    .map_or(0, |parent_id| {
                        polyforest
                            .get(parent_id)
                            .map_or(0, |parent_image_node: &ImageNode| {
                                parent_image_node.ancestors + 1
                            })
                    });

            // Add the image.
            polyforest.insert(
                image_id_and_record_to_add.0.clone(),
                ImageNode {
                    image_record: image_id_and_record_to_add.1.clone(),
                    last_used_since_epoch,
                    ancestors,
                },
            );
        }
    }

    // We need to ensure that each node's last used timestamp is at least as recent as those of all
    // the transitive descendants. We'll do this via a breadth first traversal, starting with the
    // leaves. Since we're working with a polyforest, we don't need to keep track of a "visited" set
    // like we usually would for BFS.
    let mut frontier = polyforest.keys().cloned().collect::<HashSet<_>>();
    for image_node in polyforest.values() {
        if let Some(parent_id) = &image_node.image_record.parent_id {
            frontier.remove(parent_id);
        }
    }

    // In each iteration of the outer loop, we traverse the frontier and build up a new frontier
    // for the subsequent iteration.
    while !frontier.is_empty() {
        let mut new_frontier = HashSet::new();

        for image_id in frontier {
            if let Some(image_node) = polyforest.get(&image_id).cloned() {
                if let Some(parent_id) = &image_node.image_record.parent_id {
                    if let Some(parent_node) = polyforest.get_mut(parent_id) {
                        parent_node.last_used_since_epoch = max(
                            parent_node.last_used_since_epoch,
                            image_node.last_used_since_epoch,
                        );
                        new_frontier.insert(parent_id.clone());
                    }
                }
            }
        }

        frontier = new_frontier;
    }

    // If we made it this far, we have a polyforest!
    Ok(polyforest)
}

// The main vacuum logic
fn vacuum(
    state: &mut State,
    first_run: bool,
    threshold: Byte,
    keep: Option<&RegexSet>,
    deletion_chunk_size: usize,
    min_age: Option<Duration>,
) -> io::Result<()> {
    // Find all images.
    let image_records = list_image_records(state)?;

    // Find all images in use by containers.
    let image_ids_in_use = image_ids_in_use()?;

    // Construct a polyforest of image nodes that reflects their parent-child relationships.
    let polyforest = construct_polyforest(state, first_run, &image_records, &image_ids_in_use)?;

    // Sort the images from least recently used to most recently used.
    // Break ties using the number of dependency layers.
    let mut sorted_image_nodes = polyforest.iter().collect::<Vec<_>>();
    sorted_image_nodes.sort_by(|x, y| {
        x.1.last_used_since_epoch
            .cmp(&y.1.last_used_since_epoch)
            .then(y.1.ancestors.cmp(&x.1.ancestors))
    });

    // If the user provided the `--keep` argument, we need to filter out images which match the
    // provided regexes.
    if let Some(regex_set) = keep {
        sorted_image_nodes.retain(|(_, image_node)| {
            for repository_tag in &image_node.image_record.repository_tags {
                if regex_set.is_match(&format!(
                    "{}:{}",
                    repository_tag.repository,
                    repository_tag.tag,
                )) {
                    debug!(
                        "Ignored image {} due to the {} flag.",
                        format!("{}:{}", repository_tag.repository, repository_tag.tag).code_str(),
                        "--keep".code_str(),
                    );
                    return false;
                }
            }

            true
        });
    }

    // If the `--min-age` argument is provided, we need to filter out images
    // which are newer than the provided duration.
    if let Some(duration) = min_age {
        match (SystemTime::now() - duration).duration_since(UNIX_EPOCH) {
            Ok(time_stamp) => {
                sorted_image_nodes.retain(|(image_id, image_node)| {
                    if image_node.last_used_since_epoch > time_stamp {
                        debug!(
                            "Ignored image {} due to the {} flag.",
                            image_id.code_str(),
                            "--min-age".code_str(),
                        );

                        return false;
                    }

                    true
                });
            }
            Err(e) => return Err(io::Error::new(io::ErrorKind::InvalidInput, e)),
        };
    }

    // Check if we're over the threshold.
    let mut deleted_image_ids = HashSet::new();
    let space = space_usage()?;
    if space > threshold {
        info!(
            "Docker images are currently using {}, but the limit is {}.",
            space.get_appropriate_unit(false).to_string().code_str(),
            threshold.get_appropriate_unit(false).to_string().code_str(),
        );

        // Start deleting images, beginning with the least recently used.
        for image_ids in sorted_image_nodes.chunks_mut(deletion_chunk_size) {
            for (image_id, _) in image_ids {
                // Delete the image.
                if let Err(error) = delete_image(image_id) {
                    // The deletion failed. Just log the error and proceed.
                    error!("{}", error);
                } else {
                    // Forget about the deleted image.
                    deleted_image_ids.insert(image_id.clone());
                }
            }

            // Break if we're within the threshold.
            let new_space = space_usage()?;
            if new_space <= threshold {
                info!(
                    "Docker images are now using {}, which is within the limit of {}.",
                    new_space.get_appropriate_unit(false).to_string().code_str(),
                    threshold.get_appropriate_unit(false).to_string().code_str(),
                );
                break;
            }
        }
    } else {
        debug!(
            "Docker images are using {}, which is within the limit of {}.",
            space.get_appropriate_unit(false).to_string().code_str(),
            threshold.get_appropriate_unit(false).to_string().code_str(),
        );
    }

    // Update the state.
    state.images.clear();
    for (image_id, image_node) in polyforest {
        if !deleted_image_ids.contains(&image_id) {
            state.images.insert(
                image_id.clone(),
                state::Image {
                    parent_id: image_node.image_record.parent_id.clone(),
                    last_used_since_epoch: image_node.last_used_since_epoch,
                },
            );
        }
    }

    Ok(())
}

// Stream Docker events and vacuum when necessary.
#[allow(clippy::type_complexity)]
pub fn run(
    settings: &Settings,
    state: &mut State,
    first_run: &mut bool,
    destructors: &Arc<Mutex<Vec<Box<dyn FnOnce() + Send>>>>,
) -> io::Result<()> {
    // Determine the threshold in bytes.
    let threshold = match settings.threshold {
        Threshold::Absolute(b) => b,

        #[cfg(target_os = "linux")]
        Threshold::Percentage(p) =>
        {
            #[allow(
                clippy::cast_precision_loss,
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss
            )]
            Byte::from_bytes((p * docker_root_dir_filesystem_size()?.get_bytes() as f64) as u128)
        }
    };

    // NOTE: Don't change this log line, since the test in the Homebrew formula
    // (https://github.com/Homebrew/homebrew-core/blob/HEAD/Formula/d/docuum.rb) relies on it.
    info!("Performing an initial vacuum on startup\u{2026}");

    // Run the main vacuum logic.
    vacuum(
        state,
        *first_run,
        threshold,
        settings.keep.as_ref(),
        settings.deletion_chunk_size,
        settings.min_age,
    )?;
    state::save(state)?;
    *first_run = false;

    // Spawn `docker events --format '{{json .}}'`.
    let mut child = Command::new("docker")
        .args(["events", "--format", "{{json .}}"])
        .stdout(Stdio::piped()) // [tag:stdout]
        .spawn()?;

    // Buffer the data as we read it line-by-line. The `unwrap` is safe due to [ref:stdout].
    let reader = BufReader::new(child.stdout.take().unwrap());

    // When this run is done (e.g., due to an error) or when a termination signal is received, kill
    // the child process.
    destructors.lock().unwrap().push(Box::new(move || {
        if let Err(error) = child.kill() {
            error!("{}", error);
        } else if let Err(error) = child.wait() {
            error!("{}", error);
        }
    }));

    // Handle each incoming event.
    info!("Listening for Docker events\u{2026}");
    for line_option in reader.lines() {
        // Unwrap the line.
        let line = line_option?;
        trace!("Incoming event: {}", line.code_str());

        // Parse the line as an event.
        let event = match serde_json::from_str::<Event>(&line) {
            Ok(event) => {
                trace!("Parsed as: {}", format!("{event:?}").code_str());
                event
            }
            Err(error) => {
                trace!("Skipping due to: {}", error);
                continue;
            }
        };

        // Get the ID of the image.
        let image_id = image_id(&if event.r#type == "container"
            && (event.action == "create" || event.action == "destroy")
        {
            if let Some(image_name) = event.actor.attributes.image {
                image_name
            } else {
                trace!("Invalid Docker event.");
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
            trace!("Skipping due to irrelevance.");
            continue;
        })?;

        // Inform the user that we're about to vacuum.
        debug!("Waking up\u{2026}");

        // Update the timestamp for this image.
        if touch_image(state, &image_id, true)? {
            // Run the main vacuum logic only if a new image came in.
            vacuum(
                state,
                *first_run,
                threshold,
                settings.keep.as_ref(),
                settings.deletion_chunk_size,
                settings.min_age,
            )?;
        }

        // Persist the state.
        state::save(state)?;

        // Inform the user that we're done for now.
        debug!("Going back to sleep\u{2026}");
    }

    // The `for` loop above will only terminate if something happened to `docker events`.
    Err(io::Error::new(
        io::ErrorKind::Other,
        format!("{} terminated.", "docker events".code_str()),
    ))
}

#[cfg(test)]
mod tests {
    use {
        super::{construct_polyforest, parse_docker_date, ImageNode, ImageRecord, RepositoryTag},
        crate::state::{self, State},
        std::{
            collections::{HashMap, HashSet},
            io,
            time::Duration,
        },
    };

    #[test]
    fn parse_docker_date_valid() {
        assert_eq!(
            parse_docker_date("2022-02-25 12:53:30 -0800 PST").unwrap(),
            Duration::from_secs(1_645_822_410),
        );
    }

    #[test]
    fn parse_docker_date_before_unix_epoch() {
        assert_eq!(
            parse_docker_date("0001-01-01 00:00:00 +0000 UTC").unwrap(),
            Duration::ZERO,
        );
    }

    #[test]
    fn parse_docker_date_invalid() {
        assert!(parse_docker_date("invalid").is_err());
    }

    #[test]
    fn construct_polyforest_empty() -> io::Result<()> {
        let state = State {
            images: HashMap::new(),
        };

        let image_records = HashMap::new();
        let image_ids_in_use = HashSet::new();
        let image_graph = construct_polyforest(&state, true, &image_records, &image_ids_in_use)?;

        assert_eq!(0, image_graph.len());

        Ok(())
    }

    #[test]
    fn construct_polyforest_single_image() -> io::Result<()> {
        let image_id = "id-0";

        let mut images = HashMap::new();
        images.insert(
            image_id.to_owned(),
            state::Image {
                parent_id: None,
                last_used_since_epoch: Duration::from_secs(42),
            },
        );

        let state = State { images };

        let image_record = ImageRecord {
            parent_id: None,
            created_since_epoch: Duration::from_secs(100),
            repository_tags: vec![RepositoryTag {
                repository: String::from("alpine"),
                tag: String::from("latest"),
            }],
        };

        let mut image_records = HashMap::new();
        image_records.insert(image_id.to_owned(), image_record.clone());
        let image_ids_in_use = HashSet::new();
        let image_graph = construct_polyforest(&state, true, &image_records, &image_ids_in_use)?;

        assert_eq!(1, image_graph.len());

        assert_eq!(
            Some(&ImageNode {
                image_record,
                last_used_since_epoch: Duration::from_secs(42),
                ancestors: 0,
            }),
            image_graph.get(image_id),
        );

        Ok(())
    }

    #[test]
    fn construct_polyforest_single_image_missing_state() -> io::Result<()> {
        let image_id = "id-0";
        let images = HashMap::new();
        let state = State { images };

        let image_record = ImageRecord {
            parent_id: None,
            created_since_epoch: Duration::from_secs(100),
            repository_tags: vec![RepositoryTag {
                repository: String::from("alpine"),
                tag: String::from("latest"),
            }],
        };

        let mut image_records = HashMap::new();
        image_records.insert(image_id.to_owned(), image_record.clone());
        let image_ids_in_use = HashSet::new();
        let image_graph = construct_polyforest(&state, true, &image_records, &image_ids_in_use)?;

        assert_eq!(1, image_graph.len());

        assert_eq!(
            Some(&ImageNode {
                image_record,
                last_used_since_epoch: Duration::from_secs(100),
                ancestors: 0,
            }),
            image_graph.get(image_id),
        );

        Ok(())
    }

    #[test]
    fn construct_polyforest_parent_child_increasing_timestamps() -> io::Result<()> {
        let image_id_0 = "id-0";
        let image_id_1 = "id-1";

        let mut images = HashMap::new();
        images.insert(
            image_id_0.to_owned(),
            state::Image {
                parent_id: None,
                last_used_since_epoch: Duration::from_secs(42),
            },
        );
        images.insert(
            image_id_1.to_owned(),
            state::Image {
                parent_id: Some(image_id_0.to_owned()),
                last_used_since_epoch: Duration::from_secs(43),
            },
        );

        let state = State { images };

        let image_record_0 = ImageRecord {
            parent_id: None,
            created_since_epoch: Duration::from_secs(100),
            repository_tags: vec![RepositoryTag {
                repository: String::from("alpine"),
                tag: String::from("latest"),
            }],
        };

        let image_record_1 = ImageRecord {
            parent_id: Some(image_id_0.to_owned()),
            created_since_epoch: Duration::from_secs(101),
            repository_tags: vec![RepositoryTag {
                repository: String::from("debian"),
                tag: String::from("latest"),
            }],
        };

        let mut image_records = HashMap::new();
        image_records.insert(image_id_0.to_owned(), image_record_0.clone());
        image_records.insert(image_id_1.to_owned(), image_record_1.clone());
        let image_ids_in_use = HashSet::new();
        let image_graph = construct_polyforest(&state, true, &image_records, &image_ids_in_use)?;

        assert_eq!(2, image_graph.len());

        assert_eq!(
            Some(&ImageNode {
                image_record: image_record_0,
                last_used_since_epoch: Duration::from_secs(43),
                ancestors: 0,
            }),
            image_graph.get(image_id_0),
        );

        assert_eq!(
            Some(&ImageNode {
                image_record: image_record_1,
                last_used_since_epoch: Duration::from_secs(43),
                ancestors: 1,
            }),
            image_graph.get(image_id_1),
        );

        Ok(())
    }

    #[test]
    fn construct_polyforest_parent_child_decreasing_timestamps() -> io::Result<()> {
        let image_id_0 = "id-0";
        let image_id_1 = "id-1";

        let mut images = HashMap::new();
        images.insert(
            image_id_0.to_owned(),
            state::Image {
                parent_id: None,
                last_used_since_epoch: Duration::from_secs(43),
            },
        );
        images.insert(
            image_id_1.to_owned(),
            state::Image {
                parent_id: Some(image_id_0.to_owned()),
                last_used_since_epoch: Duration::from_secs(42),
            },
        );

        let state = State { images };

        let image_record_0 = ImageRecord {
            parent_id: None,
            created_since_epoch: Duration::from_secs(100),
            repository_tags: vec![RepositoryTag {
                repository: String::from("alpine"),
                tag: String::from("latest"),
            }],
        };

        let image_record_1 = ImageRecord {
            parent_id: Some(image_id_0.to_owned()),
            created_since_epoch: Duration::from_secs(101),
            repository_tags: vec![RepositoryTag {
                repository: String::from("debian"),
                tag: String::from("latest"),
            }],
        };

        let mut image_records = HashMap::new();
        image_records.insert(image_id_0.to_owned(), image_record_0.clone());
        image_records.insert(image_id_1.to_owned(), image_record_1.clone());
        let image_ids_in_use = HashSet::new();
        let image_graph = construct_polyforest(&state, true, &image_records, &image_ids_in_use)?;

        assert_eq!(2, image_graph.len());

        assert_eq!(
            Some(&ImageNode {
                image_record: image_record_0,
                last_used_since_epoch: Duration::from_secs(43),
                ancestors: 0,
            }),
            image_graph.get(image_id_0),
        );

        assert_eq!(
            Some(&ImageNode {
                image_record: image_record_1,
                last_used_since_epoch: Duration::from_secs(42),
                ancestors: 1,
            }),
            image_graph.get(image_id_1),
        );

        Ok(())
    }

    #[test]
    fn construct_polyforest_grandparent_parent_child_increasing_timestamps() -> io::Result<()> {
        let image_id_0 = "id-0";
        let image_id_1 = "id-1";
        let image_id_2 = "id-2";

        let mut images = HashMap::new();
        images.insert(
            image_id_0.to_owned(),
            state::Image {
                parent_id: None,
                last_used_since_epoch: Duration::from_secs(42),
            },
        );
        images.insert(
            image_id_1.to_owned(),
            state::Image {
                parent_id: Some(image_id_0.to_owned()),
                last_used_since_epoch: Duration::from_secs(43),
            },
        );
        images.insert(
            image_id_2.to_owned(),
            state::Image {
                parent_id: Some(image_id_1.to_owned()),
                last_used_since_epoch: Duration::from_secs(44),
            },
        );

        let state = State { images };

        let image_record_0 = ImageRecord {
            parent_id: None,
            created_since_epoch: Duration::from_secs(100),
            repository_tags: vec![RepositoryTag {
                repository: String::from("alpine"),
                tag: String::from("latest"),
            }],
        };

        let image_record_1 = ImageRecord {
            parent_id: Some(image_id_0.to_owned()),
            created_since_epoch: Duration::from_secs(101),
            repository_tags: vec![RepositoryTag {
                repository: String::from("debian"),
                tag: String::from("latest"),
            }],
        };

        let image_record_2 = ImageRecord {
            parent_id: Some(image_id_1.to_owned()),
            created_since_epoch: Duration::from_secs(102),
            repository_tags: vec![RepositoryTag {
                repository: String::from("ubuntu"),
                tag: String::from("latest"),
            }],
        };

        let mut image_records = HashMap::new();
        image_records.insert(image_id_0.to_owned(), image_record_0.clone());
        image_records.insert(image_id_1.to_owned(), image_record_1.clone());
        image_records.insert(image_id_2.to_owned(), image_record_2.clone());
        let image_ids_in_use = HashSet::new();
        let image_graph = construct_polyforest(&state, true, &image_records, &image_ids_in_use)?;

        assert_eq!(3, image_graph.len());

        assert_eq!(
            Some(&ImageNode {
                image_record: image_record_0,
                last_used_since_epoch: Duration::from_secs(44),
                ancestors: 0,
            }),
            image_graph.get(image_id_0),
        );

        assert_eq!(
            Some(&ImageNode {
                image_record: image_record_1,
                last_used_since_epoch: Duration::from_secs(44),
                ancestors: 1,
            }),
            image_graph.get(image_id_1),
        );

        assert_eq!(
            Some(&ImageNode {
                image_record: image_record_2,
                last_used_since_epoch: Duration::from_secs(44),
                ancestors: 2,
            }),
            image_graph.get(image_id_2),
        );

        Ok(())
    }

    #[test]
    fn construct_polyforest_grandparent_parent_child_decreasing_timestamps() -> io::Result<()> {
        let image_id_0 = "id-0";
        let image_id_1 = "id-1";
        let image_id_2 = "id-2";

        let mut images = HashMap::new();
        images.insert(
            image_id_0.to_owned(),
            state::Image {
                parent_id: None,
                last_used_since_epoch: Duration::from_secs(44),
            },
        );
        images.insert(
            image_id_1.to_owned(),
            state::Image {
                parent_id: Some(image_id_0.to_owned()),
                last_used_since_epoch: Duration::from_secs(43),
            },
        );
        images.insert(
            image_id_2.to_owned(),
            state::Image {
                parent_id: Some(image_id_1.to_owned()),
                last_used_since_epoch: Duration::from_secs(42),
            },
        );

        let state = State { images };

        let image_record_0 = ImageRecord {
            parent_id: None,
            created_since_epoch: Duration::from_secs(100),
            repository_tags: vec![RepositoryTag {
                repository: String::from("alpine"),
                tag: String::from("latest"),
            }],
        };

        let image_record_1 = ImageRecord {
            parent_id: Some(image_id_0.to_owned()),
            created_since_epoch: Duration::from_secs(101),
            repository_tags: vec![RepositoryTag {
                repository: String::from("debian"),
                tag: String::from("latest"),
            }],
        };

        let image_record_2 = ImageRecord {
            parent_id: Some(image_id_1.to_owned()),
            created_since_epoch: Duration::from_secs(102),
            repository_tags: vec![RepositoryTag {
                repository: String::from("ubuntu"),
                tag: String::from("latest"),
            }],
        };

        let mut image_records = HashMap::new();
        image_records.insert(image_id_0.to_owned(), image_record_0.clone());
        image_records.insert(image_id_1.to_owned(), image_record_1.clone());
        image_records.insert(image_id_2.to_owned(), image_record_2.clone());
        let image_ids_in_use = HashSet::new();
        let image_graph = construct_polyforest(&state, true, &image_records, &image_ids_in_use)?;

        assert_eq!(3, image_graph.len());

        assert_eq!(
            Some(&ImageNode {
                image_record: image_record_0,
                last_used_since_epoch: Duration::from_secs(44),
                ancestors: 0,
            }),
            image_graph.get(image_id_0),
        );

        assert_eq!(
            Some(&ImageNode {
                image_record: image_record_1,
                last_used_since_epoch: Duration::from_secs(43),
                ancestors: 1,
            }),
            image_graph.get(image_id_1),
        );

        assert_eq!(
            Some(&ImageNode {
                image_record: image_record_2,
                last_used_since_epoch: Duration::from_secs(42),
                ancestors: 2,
            }),
            image_graph.get(image_id_2),
        );

        Ok(())
    }

    #[test]
    fn construct_polyforest_multiple_children() -> io::Result<()> {
        let image_id_0 = "id-0";
        let image_id_1 = "id-1";
        let image_id_2 = "id-2";

        let mut images = HashMap::new();
        images.insert(
            image_id_0.to_owned(),
            state::Image {
                parent_id: None,
                last_used_since_epoch: Duration::from_secs(43),
            },
        );
        images.insert(
            image_id_1.to_owned(),
            state::Image {
                parent_id: Some(image_id_0.to_owned()),
                last_used_since_epoch: Duration::from_secs(42),
            },
        );
        images.insert(
            image_id_2.to_owned(),
            state::Image {
                parent_id: Some(image_id_0.to_owned()),
                last_used_since_epoch: Duration::from_secs(44),
            },
        );

        let state = State { images };

        let image_record_0 = ImageRecord {
            parent_id: None,
            created_since_epoch: Duration::from_secs(100),
            repository_tags: vec![RepositoryTag {
                repository: String::from("alpine"),
                tag: String::from("latest"),
            }],
        };

        let image_record_1 = ImageRecord {
            parent_id: Some(image_id_0.to_owned()),
            created_since_epoch: Duration::from_secs(101),
            repository_tags: vec![RepositoryTag {
                repository: String::from("debian"),
                tag: String::from("latest"),
            }],
        };

        let image_record_2 = ImageRecord {
            parent_id: Some(image_id_0.to_owned()),
            created_since_epoch: Duration::from_secs(102),
            repository_tags: vec![RepositoryTag {
                repository: String::from("ubuntu"),
                tag: String::from("latest"),
            }],
        };

        let mut image_records = HashMap::new();
        image_records.insert(image_id_0.to_owned(), image_record_0.clone());
        image_records.insert(image_id_1.to_owned(), image_record_1.clone());
        image_records.insert(image_id_2.to_owned(), image_record_2.clone());
        let image_ids_in_use = HashSet::new();
        let image_graph = construct_polyforest(&state, true, &image_records, &image_ids_in_use)?;

        assert_eq!(3, image_graph.len());

        assert_eq!(
            Some(&ImageNode {
                image_record: image_record_0,
                last_used_since_epoch: Duration::from_secs(44),
                ancestors: 0,
            }),
            image_graph.get(image_id_0),
        );

        assert_eq!(
            Some(&ImageNode {
                image_record: image_record_1,
                last_used_since_epoch: Duration::from_secs(42),
                ancestors: 1,
            }),
            image_graph.get(image_id_1),
        );

        assert_eq!(
            Some(&ImageNode {
                image_record: image_record_2,
                last_used_since_epoch: Duration::from_secs(44),
                ancestors: 1,
            }),
            image_graph.get(image_id_2),
        );

        Ok(())
    }
}
