# *Docuum:* LRU eviction of Docker images

[![Build status](https://github.com/stepchowfun/docuum/workflows/Continuous%20integration/badge.svg?branch=master)](https://github.com/stepchowfun/docuum/actions?query=branch%3Amaster)

*Docuum* performs least recently used (LRU) eviction of Docker images to keep the disk usage below a given threshold.

Docker's built-in `docker image prune --all --filter until=…` command serves a similar purpose. However, the built-in solution isn't ideal since it uses the image creation time, rather than the last usage time, to determine which images to remove. That means it can delete frequently used images, and these may take a long time to build.

Docuum is ideal for use cases such as continuous integration workers, developer workstations, or any other environment in which Docker images accumulate on disk over time. Docuum works well with tools like [Toast](https://github.com/stepchowfun/toast) and [Docker Compose](https://docs.docker.com/compose/).

Docuum is used by Airbnb on its fleet of 1.5k+ CI workers.

## How it works

[Docker doesn't record when an image was last used.](https://github.com/moby/moby/issues/4237) To work around this, Docuum listens for notifications via `docker events` to learn when images are used. It maintains a small piece of state in a local data directory (see [this](https://docs.rs/dirs/2.0.2/dirs/fn.data_local_dir.html) for details about where this directory is on various platforms). That persisted state allows you to freely restart Docuum (or the whole machine) without losing the image usage timestamp data.

When Docuum first starts and subsequently whenever a new Docker event comes in, LRU eviction is performed until the total disk usage due to Docker images is below the given threshold. This design has a few advantages over evicting images based on a fixed [time to live](https://en.wikipedia.org/wiki/Time_to_live) (TTL), which is what various other tools in the Docker ecosystem do:

1. There is no need to configure and tune an interval to run on. Docuum evicts images immediately whenever the disk usage exceeds the threshold without waiting for any timers.
2. Docuum uses no CPU resources when there is no Docker activity. You can run it on your laptop without worrying about draining your battery.
3. In order to prevent your disk from filling up, it's more straightforward to set a threshold based on disk usage rather than guessing an appropriate maximum image age.

Docuum also respects the parent-child relationships between images. In particular, it will delete children of a parent before deleting the parent (even if the children were used more recently than the parent), because Docker doesn't allow images with children to be deleted.

## Usage

Docuum is meant to be started once and run forever, rather than as a cron job. Once Docuum is [installed](#installation), you can run it from the command line as follows:

```sh
$ docuum --threshold '30 GB'
```

You probably want to run Docuum as a daemon, e.g., with [launchd](https://www.launchd.info/), [systemd](https://www.freedesktop.org/wiki/Software/systemd/), etc. You may consult your operating system documentation for instructions on how to do that. For macOS, for example, you can create a file (owned by root) called `/Library/LaunchDaemons/local.docuum.plist` with the following:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "https://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
    <dict>
        <key>Label</key>
        <string>local.docuum</string>
        <key>Program</key>
        <string>/usr/local/bin/docuum</string>
        <key>ProgramArguments</key>
        <array>
            <string>/usr/local/bin/docuum</string>
            <string>--threshold</string>
            <string>10 GB</string>
        </array>
        <key>StandardOutPath</key>
        <string>/var/log/docuum.log</string>
        <key>StandardErrorPath</key>
        <string>/var/log/docuum.log</string>
        <key>EnvironmentVariables</key>
        <dict>
            <key>PATH</key>
            <string>/bin:/usr/bin:/usr/local/bin</string>
        </dict>
        <key>KeepAlive</key>
        <true/>
    </dict>
</plist>
```

Now Docuum will start automatically when you restart your machine, and the logs can be found at `/var/log/docuum.log`. If you do not wish to restart your machine, you can run `sudo launchctl load /Library/LaunchDaemons/local.docuum.plist` to start the daemon.

Here are the supported command-line options:

```
USAGE:
    docuum

OPTIONS:
    -h, --help
            Prints help information

    -t, --threshold <THRESHOLD>
            Sets the maximum amount of space to be used for Docker images (default: 10 GB)

    -v, --version
            Prints version information
```

## Installation

### Running Docuum in a Docker container

If you prefer not to install Docuum on your system, you can run it in a container. To run it in the foreground, you can use a command like the following:

```sh
docker run \
  --init \
  --rm \
  --tty \
  --name docuum \
  --volume /var/run/docker.sock:/var/run/docker.sock \
  --volume docuum:/root \
  stephanmisc/docuum --threshold '15 GB'
```

To run it in the background:

```sh
docker run \
  --detach \
  --init \
  --rm \
  --name docuum \
  --volume /var/run/docker.sock:/var/run/docker.sock \
  --volume docuum:/root \
  stephanmisc/docuum --threshold '15 GB'
```

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
  - If you are using Docker Engine 18.09.0 or later with [BuildKit mode](https://docs.docker.com/develop/develop-images/build_enhancements/) enabled, Docker does not create intermediate images for each build step and instead uses a separate "build cache". Docuum will only clean up images, not the Buildkit build cache. BuildKit's built-in garbage collection feature can be used for the build cache (e.g., `docker builder prune --all --force --keep-storage '30 GB'`). If you are not using BuildKit mode, Docker's caching mechanism uses intermediate images, and Docuum will happily vacuum such images as usual.
