# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
