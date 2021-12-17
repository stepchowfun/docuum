use {
    crate::format::CodeStr,
    serde::{Deserialize, Serialize},
    std::{
        collections::HashMap,
        env,
        fs::{create_dir_all, read_to_string},
        io::{self, Write},
        path::PathBuf,
        time::Duration,
    },
    tempfile::NamedTempFile,
};

// What we want to remember about an individual image
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Image {
    // The ID of the parent image, if it exists
    pub parent_id: Option<String>,

    // The amount of time that has passed between the UNIX epoch and the moment the image was most
    // recently used
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
    // [tag:state_path_has_parent]
    dirs::data_local_dir()
        .or_else(|| {
            // In the `mcr.microsoft.com/windows/nanoserver` Docker image, `dirs::data_local_dir()`
            // returns `None` (see https://github.com/dirs-dev/dirs-rs/issues/34 for details). So we
            // fall back to the value of the `LOCALAPPDATA` environment variable in that case.
            env::var("LOCALAPPDATA").ok().map(Into::into)
        })
        .map(|path| path.join("docuum/state.yml"))
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
        trace!(
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
        trace!(
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
