# Docuum: LRU eviction of Docker images

[![Build status](https://github.com/stepchowfun/docuum/workflows/Continuous%20integration/badge.svg?branch=main)](https://github.com/stepchowfun/docuum/actions?query=branch%3Amain)

*Docuum* performs least recently used (LRU) eviction of Docker images to keep the disk usage below a given threshold.

Docker's built-in `docker image prune --all --filter until=…` command serves a similar purpose. However, the built-in solution isn't ideal since it uses the image creation time, rather than the last usage time, to determine which images to remove. That means it can delete frequently used images, which may be expensive to rebuild.

Docuum is ideal for use cases such as continuous integration workers, developer workstations, or any other environment in which Docker images accumulate on disk over time. Docuum works well with tools like [Toast](https://github.com/stepchowfun/toast) and [Docker Compose](https://docs.docker.com/compose/).

Docuum is used by Airbnb on its fleet of 1.5k+ CI workers.

## How it works

[Docker doesn't record when an image was last used.](https://github.com/moby/moby/issues/4237) To work around this, Docuum listens for notifications via `docker events` to learn when images are used. It maintains a small piece of state in a local data directory (see [this](https://docs.rs/dirs/3.0.2/dirs/fn.data_local_dir.html) for details about where this directory is on various platforms). That persisted state allows you to freely restart Docuum (or the whole machine) without losing the image usage timestamp data.

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

Then you can use `Ctrl`+`C` to stop it.

However, you probably want to run Docuum as a [daemon](https://en.wikipedia.org/wiki/Daemon_\(computing\)), e.g., with [launchd](https://www.launchd.info/), [systemd](https://www.freedesktop.org/wiki/Software/systemd/), etc. You may consult your operating system documentation for instructions on how to do that. On macOS, for example, you can create a file (owned by root) called `/Library/LaunchDaemons/local.docuum.plist` with the following:

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

### Running Docuum in a Docker container on macOS or Linux (x86-64)

If you prefer not to install Docuum on your system and you're running macOS or Linux on an x86-64 CPU, you can run it in a container. To run it in the foreground, you can use a command like the following:

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

### Installation on macOS or Linux (x86-64)

If you're running macOS or Linux on an x86-64 CPU, you can install Docuum with this command:

```sh
curl https://raw.githubusercontent.com/stepchowfun/docuum/main/install.sh -LSfs | sh
```

The same command can be used again to update to the latest version.

The installation script supports the following optional environment variables:

- `VERSION=x.y.z` (defaults to the latest version)
- `PREFIX=/path/to/install` (defaults to `/usr/local/bin`)

For example, the following will install Docuum into the working directory:

```sh
curl https://raw.githubusercontent.com/stepchowfun/docuum/main/install.sh -LSfs | PREFIX=. sh
```

If you prefer not to use this installation method, you can download the binary from the [releases page](https://github.com/stepchowfun/docuum/releases), make it executable (e.g., with `chmod`), and place it in some directory in your [`PATH`](https://en.wikipedia.org/wiki/PATH_\(variable\)) (e.g., `/usr/local/bin`).

### Installation on Windows (x86-64)

If you're running Windows on an x86-64 CPU, download the latest binary from the [releases page](https://github.com/stepchowfun/docuum/releases) and rename it to `docuum` (or `docuum.exe` if you have file extensions visible). Create a directory called `Docuum` in your `%PROGRAMFILES%` directory (e.g., `C:\Program Files\Docuum`), and place the renamed binary in there. Then, in the "Advanced" tab of the "System Properties" section of "Control Panel", click on "Environment Variables..." and add the full path to the new `Docuum` directory to the `PATH` variable under "System variables". Note that the `Program Files` directory might have a different name if Windows is configured for language other than English.

To update to an existing installation, simply replace the existing binary.

### Installation with Cargo

If you have [Cargo](https://doc.rust-lang.org/cargo/), you can install Docuum as follows:

```sh
cargo install docuum
```

You can run that command with `--force` to update an existing installation.

## Requirements

- Docuum requires [Docker Engine](https://www.docker.com/products/container-runtime) 17.03.0 or later.
  - If you are using Docker Engine 18.09.0 or later with [BuildKit mode](https://docs.docker.com/develop/develop-images/build_enhancements/) enabled, Docker does not create intermediate images for each build step and instead uses a separate "build cache". Docuum will only clean up images, not the Buildkit build cache. BuildKit's built-in garbage collection feature can be used for the build cache (e.g., `docker builder prune --all --force --keep-storage '30 GB'`). If you are not using BuildKit mode, Docker's caching mechanism uses intermediate images, and Docuum will happily vacuum such images as usual.
