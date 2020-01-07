# *Docuum:* LRU eviction of Docker images

[![Build Status](https://travis-ci.org/stepchowfun/docuum.svg?branch=master)](https://travis-ci.org/stepchowfun/docuum)

*Docuum* performs least recently used (LRU) eviction of Docker images to keep the total disk usage below a given threshold.

Docker's built-in `docker image prune --filter until=...` command serves a similar purpose. However, the built-in solution isn't ideal since it uses the image creation time, rather than the last usage time, to determine which images to remove. That means it can delete frequently used images, and these may take a long time to build.

Docuum is ideal for use cases such as continuous integration workers, development environments, or any other situation in which Docker images accumulate on disk over time. Docuum works well with [Toast](https://github.com/stepchowfun/toast) or [Docker Compose](https://docs.docker.com/compose/).

## How it works

[Docker doesn't record when an image is last used.](https://github.com/moby/moby/issues/4237) To work around this, Docuum listens for notifications via `docker events` to learn when images are used. It maintains a small piece of state in a local data directory (see [this](https://docs.rs/dirs/2.0.2/dirs/fn.data_local_dir.html) for details about where this directory is on various platforms). That persisted state allows you to restart Docuum without losing the image usage timestamp data.

When Docuum starts and whenever a new Docker event comes in, LRU eviction is performed until the total disk usage due to Docker images is below the given threshold. This design has two advantages:

1. There is no need to configure an interval to run on. Docuum evicts images whenever the disk usage exceeds the threshold—no more, no less.
2. Docuum uses no CPU resources when there is no Docker activity. You can run it on your laptop without worrying about draining your battery.

## Usage

Docuum is meant to be started once and run forever, rather than as a cron job. You can run it like this:

```sh
$ docuum --threshold '30 GiB'
```

Here are the supported command-line options:

```
USAGE:
    docuum

OPTIONS:
    -c, --threshold <THRESHOLD>
            Sets the maximum amount of space to be used for Docker images (default: 10 GiB)

    -h, --help
            Prints help information

    -v, --version
            Prints version information
```

## Installation

### Easy installation

If you are running macOS or a GNU-based Linux on an x86-64 CPU, you can install Docuum with this command:

```sh
curl https://raw.githubusercontent.com/stepchowfun/docuum/master/install.sh -LSfs | sh
```

The same command can be used again to update Docuum to the latest version.

**NOTE:** Piping `curl` to `sh` is dangerous since the server might be compromised. If you're concerned about this, you can download and inspect the installation script or choose one of the other installation methods.

#### Customizing the installation

The installation script supports the following environment variables:

- `VERSION=x.y.z` (defaults to the latest version)
- `PREFIX=/path/to/install` (defaults to `/usr/local/bin`)

For example, the following will install Docuum into the working directory:

```sh
curl https://raw.githubusercontent.com/stepchowfun/docuum/master/install.sh -LSfs | PREFIX=. sh
```

### Manual installation

The [releases page](https://github.com/stepchowfun/docuum/releases) has precompiled binaries for macOS or Linux systems running on an x86-64 CPU. You can download one of them and place it in a directory listed in your [`PATH`](https://en.wikipedia.org/wiki/PATH_\(variable\)).

### Installation with Cargo

If you have [Cargo](https://doc.rust-lang.org/cargo/), you can install Docuum as follows:

```sh
cargo install docuum
```

You can run that command with `--force` to update an existing installation.

## Requirements

- Docuum requires [Docker Engine](https://www.docker.com/products/container-runtime) 17.03.0 or later.
