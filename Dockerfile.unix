# A minimal base image
FROM alpine:3.12.1

# Install the Docker CLI.
RUN apk add --no-cache docker-cli

# Install Docuum.
COPY release/docuum-x86_64-unknown-linux-musl /usr/local/bin/docuum

# Set the entrypoint to Docuum. Note that Docuum is not intended to be run as
# an init process, so be sure to pass `--init` to `docker run`.
ENTRYPOINT ["/usr/local/bin/docuum"]
