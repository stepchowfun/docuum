use crate::format::CodeStr;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::{create_dir_all, read_to_string},
    io::{self, Write},
    path::{Path, PathBuf},
    time::Duration,
    env
};
use tempfile::NamedTempFile;

// What we want to remember about an individual image
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Image {
    // The ID of the parent image, if it exists
    pub parent_id: Option<String>,

    // The amount of time that has passed between the UNIX epoch and the moment the image was
    // most recently used
    pub last_used_since_epoch: Duration,
}

// The program state
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    // Map from image ID to `Image`
    pub images: HashMap<String, Image>,
}

// Where the program state is persisted on disk
fn path() -> Option<PathBuf> {
    let mut base_path = dirs::data_local_dir();

    // Overwrite the directory on Windows because it's empty in nanoserver
    if cfg!(windows) {
        if let Ok(local_dir) = env::var("LOCALAPPDATA") {
            base_path = Option::Some(Path::new(&local_dir).to_path_buf())
        }
    }

    // [tag:state_path_has_parent]
    base_path.map(|path| path.join("docuum/state.yml"))
}

// Return the state in which the program starts, if no state was loaded from disk.
pub fn initial() -> State {
    State {
        images: HashMap::new(),
    }
}

// Load the program state from disk.
pub fn load() -> io::Result<State> {
    // Check if we have a path.
    if let Some(path) = path() {
        // Log what we are trying to do in case an error occurs.
        debug!(
            "Attempting to load the state from {}\u{2026}",
            path.to_string_lossy().code_str(),
        );

        // Read the YAML from disk.
        let yaml = read_to_string(path)?;

        // Deserialize the YAML.
        serde_yaml::from_str(&yaml).map_err(|error| io::Error::new(io::ErrorKind::Other, error))
    } else {
        // Fail if we don't have a path.
        Err(io::Error::new(
            io::ErrorKind::Other,
            "Unable to locate data directory.",
        ))
    }
}

// Save the program state to disk.
pub fn save(state: &State) -> io::Result<()> {
    // Check if we have a path.
    if let Some(path) = path() {
        // Log what we're trying to do in case an error occurs.
        debug!(
            "Persisting the state to {}\u{2026}",
            path.to_string_lossy().code_str(),
        );

        // The `unwrap` is safe due to [ref:state_path_has_parent].
        let parent = path.parent().unwrap().to_owned();

        // The `unwrap` is safe because serialization should never fail.
        let payload = serde_yaml::to_string(state).unwrap();

        // Create the ancestor directories, if needed.
        create_dir_all(parent.clone())?;

        // Persist the state to disk.
        let mut temp_file = NamedTempFile::new_in(parent)?;
        temp_file.write_all(payload.as_bytes())?;
        temp_file.flush()?;
        temp_file.persist(path)?;
    } else {
        // Fail if we don't have a path.
        return Err(io::Error::new(
            io::ErrorKind::Other,
            "Unable to locate data directory.",
        ));
    }

    Ok(())
}
