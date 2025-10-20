# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.25.1] - 2025-10-19

### Fixed
- Docuum no longer enters a crash loop when there are containers stuck in the `removing` state. Thanks to Addison Kimball for reporting this issue, and thanks to Farrukh Taqveem for investigating and fixing it.

## [0.25.0] - 2024-05-02

### Added
- Added `--min-age` argument.

## [0.24.0] - 2024-04-05

### Fixed
- Docuum now cleans up child processes when exiting due to a signal (`SIGHUP`, `SIGINT`, or `SIGTERM`).

## [0.23.1] - 2023-10-02

### Added
- The Docuum Docker image now supports AArch64.

## [0.23.0] - 2023-08-17

### Changed
- Docuum now only runs its vacuuming logic when it learns about a new image for the first time.

## [0.22.4] - 2023-06-18

### Added
- Docuum supports a new platform: Windows on AArch64.

## [0.22.3] - 2023-06-02

### Added
- Docuum supports a new platform: musl Linux on AArch64.

## [0.22.2] - 2023-05-23

### Added
- Docuum supports a new platform: GNU Linux on AArch64.

## [0.22.1] - 2023-05-13

### Added
- Docuum supports a new platform: macOS on Apple silicon.

## [0.22.0] - 2023-04-09

### Added
- Added `--deletion-chunk-size`, thanks to Kulek Alexandr.

## [0.21.1] - 2022-04-12

### Fixed
- Fixed an issue introduced in v0.21.0 which prevented Docuum from working on Windows.

## [0.21.0] - 2022-04-05

### Added
- On Linux, Docuum now supports percentage-based thresholds such as `--threshold '50%'` in addition to absolute thresholds like `--threshold '10 GB'`.

## [0.20.5] - 2022-03-06

### Fixed
- Docuum now works with images produced by `kaniko --reproducible`, which produces images with timestamps earlier than the UNIX epoch.

## [0.20.4] - 2021-12-17

### Changed
- Log levels have been adjusted to give more control over the verbosity of log messages. All `debug` messages were moved to `trace`, some `info` messages were moved to `debug`, and the default log level has been changed to `debug`. This means that the same messages should be logged as before, unless a custom log level has been specified.

## [0.20.3] - 2021-08-08

### Changed
- When Docker is not running, Docuum now restarts every 5 seconds instead of every second.

## [0.20.2] - 2021-08-02

### Fixed
- Docuum now uses a smarter strategy for populating "last used" timestamps for unrecognized images, thanks to a suggestion by Mac Chaffee. Previously, Docuum would default to the image build timestamp when no last used timestamp for that image is known. However, that strategy can occasionally lead to situations in which Docuum would delete an image that was built a long time ago but only recently pulled locally, because Docuum could discover and delete the image before consuming the image pull event. Now, Docuum only defaults to the image build time the first time it runs on a machine, and thereafter it defaults to the current time.

## [0.20.1] - 2021-08-02

### Changed
- Docuum now uses a more robust way to determine the images that are currently in use by containers.

## [0.20.0] - 2021-08-02

### Added
- Added the `--keep` flag to prevent Docuum from deleting certain images.

## [0.19.2] - 2021-07-14

### Fixed
- Added a workaround for a bug in the `mcr.microsoft.com/windows/nanoserver` Docker image so that Docuum can run in that environment.

## [0.19.1] - 2021-07-14

### Fixed
- Fixed an incorrect error message.

## [0.19.0] - 2021-07-13

### Changed
- The Windows binary is now statically linked, which makes it portable enough to run with the `mcr.microsoft.com/windows/nanoserver` Docker image.

## [0.18.1] - 2021-07-13

### Fixed
- Fixed an issue with the Docker image which was caused by incorrect file permissions.

## [0.18.0] - 2021-07-12

### Added
- Docuum now supports Windows.

## [0.17.0] - 2021-07-12

### Changed
- Docuum's dependencies have been updated to their latest versions. There should be no changes in behavior.

## [0.16.1] - 2020-12-04

### Changed
- This version is the same as 0.16.0, except for Linux we now distribute musl binaries in addition to the glibc binaries. The `stephanmisc/docuum` Docker image is also much smaller now thanks to switching to Alpine Linux (with the new musl release) over Debian Slim.

## [0.16.0] - 2020-10-12

### Changed
- Docuum now considers an image to be used when a container is created from it, rather than just when a container is destroyed.

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
