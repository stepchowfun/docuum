mod format;
mod run;
mod state;

use {
    crate::{format::CodeStr, run::run},
    atty::Stream,
    byte_unit::Byte,
    chrono::Local,
    clap::{App, AppSettings, Arg},
    env_logger::{fmt::Color, Builder},
    log::{Level, LevelFilter},
    parse_duration::parse,
    regex::RegexSet,
    std::{
        env,
        io::{self, Write},
        process::exit,
        str::FromStr,
        sync::{Arc, Mutex},
        thread::sleep,
        time::Duration,
    },
};

#[macro_use]
extern crate log;

// The program version
const VERSION: &str = env!("CARGO_PKG_VERSION");

// Defaults
const DEFAULT_DELETION_CHUNK_SIZE: usize = 1;
const DEFAULT_LOG_LEVEL: LevelFilter = LevelFilter::Debug;
const DEFAULT_THRESHOLD: &str = "10 GB";

// Command-line argument and option names
const DELETION_CHUNK_SIZE_OPTION: &str = "deletion-chunk-size";
const KEEP_OPTION: &str = "keep";
const THRESHOLD_OPTION: &str = "threshold";
const MIN_AGE_OPTION: &str = "min-age";

// Size threshold argument, absolute or relative to filesystem size
#[derive(Copy, Clone)]
enum Threshold {
    Absolute(Byte),

    #[cfg(target_os = "linux")]
    Percentage(f64),
}

impl Threshold {
    // Parse a `Threshold`. Relative thresholds are only supported on Linux.
    #[cfg(target_os = "linux")]
    fn from_str(threshold: &str) -> io::Result<Threshold> {
        match threshold.strip_suffix('%') {
            Some(threshold) => {
                if cfg!(target_os = "linux") {
                    threshold
                        .trim()
                        .parse::<f64>()
                        .map_err(|_| {
                            io::Error::new(
                                io::ErrorKind::InvalidInput,
                                format!("Invalid relative threshold {}.", threshold.code_str()),
                            )
                        })
                        .and_then(|f| {
                            if f.is_normal() && (0.0_f64..=100.0_f64).contains(&f) {
                                Ok(f)
                            } else {
                                Err(io::Error::new(
                                    io::ErrorKind::InvalidInput,
                                    format!("Invalid relative threshold {}.", threshold.code_str()),
                                ))
                            }
                        })
                        .map(|f| Threshold::Percentage(f / 100.0))
                } else {
                    Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "Relative thresholds are only supported on Linux.",
                    ))
                }
            }
            None => Byte::from_str(threshold)
                .map_err(|_| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("Invalid absolute threshold {}.", threshold.code_str()),
                    )
                })
                .map(Threshold::Absolute),
        }
    }

    #[cfg(not(target_os = "linux"))]
    fn from_str(threshold: &str) -> io::Result<Threshold> {
        Byte::from_str(threshold)
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("Invalid absolute threshold {}.", threshold.code_str()),
                )
            })
            .map(Threshold::Absolute)
    }
}

// This struct represents the command-line arguments.
pub struct Settings {
    threshold: Threshold,
    keep: Option<RegexSet>,
    deletion_chunk_size: usize,
    min_age: Option<Duration>,
}

// Set up the logger.
fn set_up_logging() {
    Builder::new()
        .filter_module(
            module_path!(),
            LevelFilter::from_str(
                &env::var("LOG_LEVEL").unwrap_or_else(|_| DEFAULT_LOG_LEVEL.to_string()),
            )
            .unwrap_or(DEFAULT_LOG_LEVEL),
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
                Level::Debug => {
                    style.set_color(Color::Blue);
                }
                Level::Trace => {
                    style.set_color(Color::Cyan);
                }
            }

            writeln!(
                buf,
                "{} {}",
                style.value(format!(
                    "[{} {}]",
                    Local::now().format("%Y-%m-%d %H:%M:%S %:z"),
                    record.level(),
                )),
                record.args(),
            )
        })
        .init();
}

// Parse the command-line arguments.
fn settings() -> io::Result<Settings> {
    // Set up the command-line interface.
    let matches = App::new("Docuum")
        .version(VERSION)
        .version_short("v")
        .author("Stephan Boyer <stephan@stephanboyer.com>")
        .about("Docuum performs LRU cache eviction for Docker images.")
        .setting(AppSettings::ColoredHelp)
        .setting(AppSettings::NextLineHelp)
        .setting(AppSettings::UnifiedHelpMessage)
        .arg(
            Arg::with_name(THRESHOLD_OPTION)
                .value_name("THRESHOLD")
                .short("t")
                .long(THRESHOLD_OPTION)
                .help(&format!(
                    "Sets the maximum amount of space to be used for Docker images (default: {})",
                    DEFAULT_THRESHOLD.code_str(),
                )),
        )
        .arg(
            Arg::with_name(KEEP_OPTION)
                .value_name("REGEX")
                .short("k")
                .long(KEEP_OPTION)
                .multiple(true)
                .number_of_values(1)
                .help("Prevents deletion of images for which repository:tag matches <REGEX>"),
        )
        .arg(
            Arg::with_name(DELETION_CHUNK_SIZE_OPTION)
                .value_name("DELETION CHUNK SIZE")
                .short("d")
                .long(DELETION_CHUNK_SIZE_OPTION)
                .help(&format!(
                    "Removes specified quantity of images at a time \
                        (default: {DEFAULT_DELETION_CHUNK_SIZE})",
                )),
        )
        .arg(
            Arg::with_name(MIN_AGE_OPTION)
                .value_name("MIN AGE")
                .short("m")
                .long(MIN_AGE_OPTION)
                .help("Specifies which images to delete based image creation time"),
        )
        .get_matches();

    // Read the threshold.
    let default_threshold = Threshold::Absolute(
        Byte::from_str(DEFAULT_THRESHOLD).unwrap(), // Manually verified safe
    );
    let threshold = matches
        .value_of(THRESHOLD_OPTION)
        .map_or_else(|| Ok(default_threshold), Threshold::from_str)?;

    // Determine what images need to be preserved at all costs.
    let keep = match matches.values_of(KEEP_OPTION) {
        Some(values) => match RegexSet::new(values) {
            Ok(set) => Some(set),
            Err(e) => return Err(io::Error::new(io::ErrorKind::InvalidInput, e)),
        },
        None => None,
    };

    // Determine how many images to delete at once.
    let deletion_chunk_size = match matches.value_of(DELETION_CHUNK_SIZE_OPTION) {
        Some(v) => match v.parse::<usize>() {
            Ok(chunk_size) => chunk_size,
            Err(e) => return Err(io::Error::new(io::ErrorKind::InvalidInput, e)),
        },
        None => DEFAULT_DELETION_CHUNK_SIZE,
    };

    // Determine the minimum age for images to be considered for deletion.
    let min_age = match matches.value_of(MIN_AGE_OPTION) {
        Some(value) => match parse(value) {
            Ok(duration) => Some(duration),
            Err(e) => return Err(io::Error::new(io::ErrorKind::InvalidInput, e)),
        },
        None => None,
    };

    Ok(Settings {
        threshold,
        keep,
        deletion_chunk_size,
        min_age,
    })
}

// This function consumes and runs all the registered destructors. We use this mechanism instead of
// RAII for things that need to be cleaned up even when the process is killed due to a signal.
#[allow(clippy::type_complexity)]
fn run_destructors(destructors: &Arc<Mutex<Vec<Box<dyn FnOnce() + Send>>>>) {
    let mut mutex_guard = destructors.lock().unwrap();
    let destructor_fns = std::mem::take(&mut *mutex_guard);
    for destructor in destructor_fns {
        destructor();
    }
}

// Let the fun begin!
fn main() {
    // If Docuum is in the foreground process group for some TTY, the process will receive a SIGINT
    // when the user types CTRL+C at the terminal. The default behavior is to crash when this signal
    // is received. However, we would rather clean up resources before terminating, so we trap the
    // signal here. This code also traps SIGHUP and SIGTERM, since we compile the `ctrlc` crate with
    // the `termination` feature [ref:ctrlc_term].
    let destructors = Arc::new(Mutex::new(Vec::<Box<dyn FnOnce() + Send>>::new()));
    let destructors_clone = destructors.clone();
    if let Err(error) = ctrlc::set_handler(move || {
        run_destructors(&destructors_clone);
        exit(1);
    }) {
        // Log the error and proceed anyway.
        error!("{}", error);
    }

    // Determine whether to print colored output.
    colored::control::set_override(atty::is(Stream::Stderr));

    // Set up the logger.
    set_up_logging();

    // Parse the command-line arguments.
    let settings = match settings() {
        Ok(settings) => settings,
        Err(error) => {
            error!("{}", error);
            exit(1);
        }
    };

    // Try to load the state from disk.
    let (mut state, mut first_run) = state::load().map_or_else(
        |error| {
            // We couldn't load any state from disk. Log the error.
            warn!(
                "Unable to load state from disk. Proceeding with initial state. Details: {}",
                error.to_string().code_str(),
            );

            // Start with the initial state.
            (state::initial(), true)
        },
        |state| (state, false),
    );

    // Stream Docker events and vacuum when necessary. Restart if an error occurs.
    loop {
        // This will run until an error occurs (it never returns `Ok`).
        if let Err(error) = run(&settings, &mut state, &mut first_run, &destructors) {
            error!("{}", error);
        }

        // Clean up any resources left over from that run.
        run_destructors(&destructors);

        // Wait a moment and then retry.
        info!("Retrying in 5 seconds\u{2026}");
        sleep(Duration::from_secs(5));
    }
}
