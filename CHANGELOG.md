# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.15.1] - 2020-10-09

### Fixed
- Fixed a bug which would cause Docuum to crash if the system temporary directory (e.g., `/tmp` is in a different filesystem than the user's local data directory. Thanks Mac Chaffee for the fix!

## [0.15.0] - 2020-10-09

### Changed
- Docuum now persists its state atomically. This is to avoid the possibility of having the state only partially written due to abnormal termination, such as from a power failure.

## [0.14.0] - 2020-10-08

### Changed
- Docuum now uses the Docker CLI rather than the Docker API to communicate with the Docker daemon. This was motivated by a recent issue in which Docuum mysteriously stopped being able to stream events from Docker. At the time of this writing, it's not clear whether the issue is with Bollard (our Docker API library) or with the Docker API itself, but we know that the Docker CLI continues to work. See https://github.com/fussybeaver/bollard/issues/113 for details. Unfortunately, this is a **breaking change** because the schema for the state has changed.

## [0.13.1] - 2020-10-08

### Fixed
- Docuum now persists its state after the initial run on startup, and not just on subsequent runs triggered by Docker events. The bug was introduced in the previous version, v0.13.0.

## [0.13.0] - 2020-10-08

### Changed
- Thanks to Matthew Donoughe, Docuum now uses a better algorithm for determining the order in which to delete images. This should make Docuum more efficient and less noisy.

## [0.12.0] - 2020-08-06

### Changed
- Docuum now uses the Docker API to communicate with the Docker daemon directly and no longer depends on the Docker CLI.

## [0.11.0] - 2020-07-27

### Changed
- When Docuum discovers an image it hasn't seen before, it now bootstraps the "last used" timestamp from the image creation timestamp rather than the current timestamp.

## [0.10.1] - 2020-07-26

### Changed
- Reverted the change from v0.10.0 due to a bug in a dependency (see https://github.com/stepchowfun/docuum/issues/78).

## [0.10.0] - 2020-07-26

### Changed
- Docuum now uses the Docker API to communicate with the Docker daemon directly and no longer depends on the Docker CLI.

## [0.9.5] - 2020-07-14

### Fixed
- Fixed a bug which caused zombie `docker events ...` processes to accumulate over time.

## [0.9.4] - 2020-02-12

### Changed
- Docuum no longer considers the `delete` or `untag` image events to be "uses" of the image.

## [0.9.3] - 2020-02-12

### Fixed
- Fixed a bug in which Docuum would not consider certain image events as "uses" of the image.

## [0.9.2] - 2020-02-01

### Fixed
- Fixed a bug in which Docuum could enter a crash loop by repeatedly trying to query the ID of an image that no longer exists. This would happen when there is a container that points to such an image.

## [0.9.1] - 2020-01-29

### Changed
- In logs, timestamps are now displayed in the local time zone (along with the UTC offset).

## [0.9.0] - 2020-01-17

### Fixed
- Unrecognized images are considered to be brand new, rather than as old as the UNIX epoch.

## [0.8.0] - 2020-01-15

### Fixed
- Docuum now listens for the `container destroy` event rather than the `container die` event.

## [0.7.0] - 2020-01-08

### Fixed
- Docuum now wakes up when an image is imported or loaded, not just when an image is built or pulled.

## [0.6.0] - 2020-01-08

### Fixed
- Docuum now wakes up when an image is pulled, not just when an image is built.

## [0.5.0] - 2020-01-07

### Fixed
- Fixed some incorrect error messages and log lines.
- Optimized the number of disk writes.

## [0.4.0] - 2020-01-07

### Added
- Added timestamps to the log format.

### Changed
- Renamed the `-c` ("capacity") option to `-t` ("threshold") as a follow-up from the change introduced in v0.3.0.
- Docuum now automatically restarts itself when an error occurs.

### Fixed
- Docuum now cleans up the `docker events` child process when an error occurs.

## [0.3.0] - 2020-01-06

### Changed
- Renamed the `--capacity` option to `--threshold`.

## [0.2.0] - 2020-01-06

### Added
- Initial release.
