# Docuum: LRU eviction of Docker images

[![Build status](https://github.com/stepchowfun/docuum/workflows/Continuous%20integration/badge.svg?branch=main)](https://github.com/stepchowfun/docuum/actions?query=branch%3Amain)

*Docuum* performs least recently used (LRU) eviction of Docker images to keep the disk usage below a given threshold.

Docker's built-in `docker image prune --all --filter until=…` command serves a similar purpose. However, the built-in solution isn't ideal since it uses the image creation time, rather than the last usage time, to determine which images to remove. That means it can delete frequently used images, which may be expensive to rebuild or time-consuming to pull.

Docuum is ideal for use cases such as continuous integration (CI) workers, developer workstations, or any other environment in which Docker images accumulate on disk over time. Docuum works well with tools like [Toast](https://github.com/stepchowfun/toast) and [Docker Compose](https://docs.docker.com/compose/).

Docuum is used by Netflix (on its production Kubernetes nodes) and Airbnb (on its CI fleet of 1.5k+ CI workers).

## How it works

[Docker doesn't record when an image was last used.](https://github.com/moby/moby/issues/4237) To work around this, Docuum listens for notifications via `docker events` to learn when images are used. It maintains a small piece of state in a local data directory (see [this](https://docs.rs/dirs/3.0.2/dirs/fn.data_local_dir.html) for details about where this directory is on various platforms). That persisted state allows you to freely restart Docuum (or the whole machine) without losing the image usage timestamp data.

When Docuum first starts and subsequently whenever a new Docker event comes in, LRU eviction is performed until the total disk usage due to Docker images is below the given threshold. This design has a few advantages over evicting images based on a fixed [time to live](https://en.wikipedia.org/wiki/Time_to_live) (TTL), which is what various other tools in the Docker ecosystem do:

1. There is no need to configure and tune an interval to run on. Docuum evicts images immediately whenever the disk usage exceeds the threshold without waiting for any timers.
2. Docuum uses no CPU resources when there is no Docker activity. You can run it on your laptop without worrying about draining your battery.
3. In order to prevent your disk from filling up, it's more straightforward to set a threshold based on disk usage rather than guessing an appropriate maximum image age.

Docuum also respects the parent-child relationships between images. In particular, it will delete children of a parent before deleting the parent (even if the children were used more recently than the parent), because Docker doesn't allow images with children to be deleted.

## Usage

Once Docuum is [installed](#installation-instructions), you can run it manually from the command line as follows:

```sh
docuum --threshold '10 GB'
```

Docuum will then start listening for Docker events. You can use `Ctrl`+`C` to stop it.

You probably want to run Docuum as a [daemon](https://en.wikipedia.org/wiki/Daemon_\(computing\)), e.g., with [launchd](https://www.launchd.info/), [systemd](https://www.freedesktop.org/wiki/Software/systemd/), etc. See the [Configuring your operating system to run the binary as a daemon](#configuring-your-operating-system-to-run-the-binary-as-a-daemon) section below for instructions.

Here are the supported command-line options:

```
USAGE:
    docuum

OPTIONS:
    -d, --deletion-chunk-size <DELETION CHUNK SIZE>
            Removes specified quantity of images at a time (default: 1)

    -h, --help
            Prints help information

    -k, --keep <REGEX>...
            Prevents deletion of images for which repository:tag matches <REGEX>

    -t, --threshold <THRESHOLD>
            Sets the maximum amount of space to be used for Docker images (default: 10 GB)

    -m, --min-age <Duration>
            Sets the minimum age of images to be considered for deletion

    -v, --version
            Prints version information
```

The `--threshold` flag accepts [multiple representations](https://docs.rs/byte-unit/4.0.12/byte_unit/struct.Byte.html#examples-2), like `10 GB`, `10 GiB`, or `10GB`. On Linux, percentage-based thresholds like `50%` are also supported.

You can change the log verbosity by setting an environment variable named `LOG_LEVEL` to one of `trace`, `debug`, `info`, `warning`, or `error`. The default is `debug`.

## Docker's build cache

Old versions of Docker would create an intermediate image for each step in your `Dockerfile`, and Docuum would happily vacuum them when needed. Since the introduction of [BuildKit](https://docs.docker.com/build/buildkit/), Docker no longer produces those intermediate images, and a separate "build cache" is used instead. BuildKit has its own [garbage collector](https://docs.docker.com/build/cache/garbage-collection/) for its build cache with a default threshold of 10% of the total disk capacity.

Docuum does not vacuum BuildKit's build cache, and BuildKit's garbage collector doesn't vacuum images. Both can be used together.

## Installation instructions

Installation consists of two steps:

1. Installing the binary
2. Configuring your operating system to run the binary as a daemon

### Installing the binary

#### Installation on macOS or Linux (AArch64 or x86-64)

If you're running macOS or Linux (AArch64 or x86-64), you can install Docuum with this command:

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

#### Installation on Windows (AArch64 or x86-64)

If you're running Windows (AArch64 or x86-64), download the latest binary from the [releases page](https://github.com/stepchowfun/docuum/releases) and rename it to `docuum` (or `docuum.exe` if you have file extensions visible). Create a directory called `Docuum` in your `%PROGRAMFILES%` directory (e.g., `C:\Program Files\Docuum`), and place the renamed binary in there. Then, in the "Advanced" tab of the "System Properties" section of Control Panel, click on "Environment Variables..." and add the full path to the new `Docuum` directory to the `PATH` variable under "System variables". Note that the `Program Files` directory might have a different name if Windows is configured for a language other than English.

To update an existing installation, simply replace the existing binary.

#### Installation with Homebrew

If you have [Homebrew](https://brew.sh/), you can install Docuum as follows:

```sh
brew install docuum
```

You can update an existing installation with `brew upgrade docuum`.

#### Installation with Cargo

If you have [Cargo](https://doc.rust-lang.org/cargo/), you can install Docuum as follows:

```sh
cargo install docuum
```

You can run that command with `--force` to update an existing installation.

#### Running Docuum in a Docker container on a host capable of running Linux containers

If you prefer not to install Docuum on your system and you're running macOS or Linux (AArch64 or x86-64), you can run it in a container:

```sh
docker run \
  --init \
  --rm \
  --tty \
  --name docuum \
  --mount type=bind,src=/var/run/docker.sock,dst=/var/run/docker.sock \
  --mount type=volume,source=docuum,target=/root \
  stephanmisc/docuum --threshold '10 GB'
```

If you're on a Windows system configured to run Linux containers, use this command:

```powershell
docker run `
  --init `
  --rm `
  --tty `
  --name docuum `
  --mount type=bind,src=//var/run/docker.sock,dst=/var/run/docker.sock `
  --mount type=volume,source=docuum,target=/root `
  stephanmisc/docuum --threshold '10 GB'
```

We don't currently publish a Windows-based image, because some Windows machines (namely, those which run containers with process isolation rather than Hyper-V) can only run Windows containers that were built for the exact build of Windows (e.g., 1809) which is running on the host. This makes Windows-based images less portable, and as a result we'd need to publish a separate Windows-based image for each build of Windows we want to support. At this time, we don't have the infrastructure to do that.

The instructions below for configuring your operating system to run Docuum as a daemon assume it's installed as an executable binary. If you prefer to run it as a Docker container, change the relevant service definition to run a Docker command like the relevant one above, with the following adjustments:

- Omit the `--tty` flag. This prevents Docuum from printing colored logs, which you probably don't want for a daemon.
- Configure Docker as a hard dependency. Ordinarily, Docuum and Docker can be started in any order, and Docuum will patiently wait for Docker to start if needed. However, when running Docuum as a Docker container, then of course Docker must be started first.

### Configuring your operating system to run the binary as a daemon

#### Creating a launchd service on macOS

On macOS, [launchd](https://www.launchd.info/) can be used to run Docuum as a daemon. Create a file (owned by root) called `/Library/LaunchDaemons/local.docuum.plist` with the following contents, adjusting the arguments as needed:

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

Run `sudo launchctl load /Library/LaunchDaemons/local.docuum.plist` to start the service. You can view the logs with `tail -F /var/log/docuum.log`.

#### Creating a systemd service on Linux

On most Linux distributions, [systemd](https://www.freedesktop.org/wiki/Software/systemd/) can be used to run Docuum as a daemon. Create a file (owned by root) called `/etc/systemd/system/docuum.service` with the following contents, adjusting the arguments as needed:

```ini
[Unit]
Description=Docuum
After=docker.service
Wants=docker.service

[Service]
Environment='THRESHOLD=10 GB'
ExecStart=/usr/local/bin/docuum --threshold ${THRESHOLD}
Restart=on-failure

[Install]
WantedBy=multi-user.target
```

Run `sudo systemctl enable docuum --now` to enable and start the service. You can view the logs with `sudo journalctl --follow --unit docuum`.

#### Creating an NSSM service on Windows

On Windows, [NSSM](https://nssm.cc/), the "Non-Sucking Service Manager", can be used to run Docuum as a daemon. [Install NSSM](https://nssm.cc/download) by downloading the binary and adding it to your `PATH` (see the [Installation on Windows (x86-64)](#installation-on-windows-x86-64) section for instructions on how to configure this environment variable), then run Windows Terminal _as Administrator_ and enter the following command:

```powershell
nssm install Docuum
```

NSSM will then open a configuration window. Configure the following:

- In the `Application` tab, select the path to the Docuum binary. You can optionally add arguments like `--threshold "10 GB"`.
- Optionally, in the `I/O` tab, choose where you want the logs to be written.

Then click the `Install service` button. Back in Windows Terminal, run the following to start the service:

```powershell
nssm start Docuum
```

If you configured a path for the log file in the `I/O` tab of the installation window, you can view those logs with `Get-Content -Wait docuum.log` (adjusting the file path as needed).

## Requirements

- Docuum requires [Docker Engine](https://www.docker.com/products/container-runtime) 17.03.0 or later.
