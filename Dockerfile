# A base image with glibc
FROM debian:buster-slim

# Install Docuum.
COPY release/docuum-x86_64-unknown-linux-gnu /usr/local/bin/docuum

# Set the entrypoint to Docuum. Note that Docuum is not intended to be run as
# an init process, so we run it indirectly via `sh`.
ENTRYPOINT ["/usr/local/bin/docuum"]
