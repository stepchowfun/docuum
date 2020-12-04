# A base image with glibc
FROM debian:buster-slim

# Install the Docker CLI.
RUN \
  apt-get update && \
  apt-get install --yes \
    apt-transport-https \
    ca-certificates \
    curl \
    gnupg2 \
    software-properties-common \
    && \
  curl -fsSL https://download.docker.com/linux/debian/gpg | apt-key add - && \
  add-apt-repository \
    "deb [arch=amd64] https://download.docker.com/linux/debian buster stable" && \
  apt-get update && \
  apt-get install --yes docker-ce-cli && \
  apt-get remove --yes \
    apt-transport-https \
    ca-certificates \
    curl \
    gnupg2 \
    software-properties-common \
    && \
  apt-get autoremove --yes && \
  rm -rf /var/lib/apt/lists/*

# Install Docuum.
COPY release/docuum-x86_64-unknown-linux-gnu /usr/local/bin/docuum

# Set the entrypoint to Docuum. Note that Docuum is not intended to be run as
# an init process, so be sure to pass `--init` to `docker run`.
ENTRYPOINT ["/usr/local/bin/docuum"]
