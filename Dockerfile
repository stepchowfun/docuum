# The base image for the build stage
FROM --platform=$BUILDPLATFORM alpine:3.23.4 AS build

# Choose the appropriate Docuum binary to install.
ARG TARGETPLATFORM
COPY artifacts/docuum-x86_64-unknown-linux-musl /tmp/linux/amd64
COPY artifacts/docuum-aarch64-unknown-linux-musl /tmp/linux/arm64
RUN cp "/tmp/$TARGETPLATFORM" /usr/local/bin/docuum

# A minimal base image
FROM --platform=$TARGETPLATFORM alpine:3.23.4

# Install the Docker CLI.
RUN apk add --no-cache docker-cli

# Install Podman-Remote without pulling in the misspecified dependencies for local execution.
RUN apk fetch podman-remote && tar -xvf podman-remote-*.apk -C / usr/bin/podman-remote && rm podman-remote-*.apk

# Make Podman-Remote available under its usual name.
RUN ln /usr/bin/podman-remote /usr/bin/podman

# Install Docuum.
COPY --from=build /usr/local/bin/docuum /usr/local/bin/docuum

# Set the entrypoint to Docuum. Note that Docuum is not intended to be run as
# an init process, so be sure to pass `--init` to `docker container run`.
ENTRYPOINT ["/usr/local/bin/docuum"]
