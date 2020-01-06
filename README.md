# Docuum

[![Build Status](https://travis-ci.org/stepchowfun/docuum.svg?branch=master)](https://travis-ci.org/stepchowfun/docuum)

*Docuum* performs least recently used (LRU) eviction of Docker images to keep the total disk usage below a given threshold. Docker's build in `docker image prune` command is not ideal because it uses the image creation time, rather than the last usage time, to determine which images to remove.

Docuum is ideal for use cases such as CI workers, development environments, or any other situation in which Docker images accumulate on disk over time.

## Usage

Docuum is meant to be run forever (rather than as a periodic cron job, for example). You can run it like this:

```sh
$ docuum --capacity '30 GiB'
```

Here are the supported command-line options:

```
USAGE:
    docuum

OPTIONS:
    -c, --capacity <CAPACITY>
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
