FROM --platform=$BUILDPLATFORM ubuntu AS builder
ENV HOME="/root"
WORKDIR $HOME

RUN apt update \
  && apt install -y --no-install-recommends \
  build-essential \
  curl \
  python3-venv \
  && apt clean \
  && rm -rf /var/lib/apt/lists/*

# Setup zig as cross compiling linker
RUN python3 -m venv $HOME/.venv
RUN .venv/bin/pip install cargo-zigbuild
ENV PATH="$HOME/.venv/bin:$PATH"

# Install rust
ARG TARGETPLATFORM
RUN case "$TARGETPLATFORM" in \
  "linux/arm64") echo "aarch64-unknown-linux-musl" > rust_target.txt ;; \
  "linux/amd64") echo "x86_64-unknown-linux-musl" > rust_target.txt ;; \
  *) exit 1 ;; \
  esac

# Update rustup whenever we bump the rust version
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --target $(cat rust_target.txt) --profile minimal --default-toolchain none
ENV PATH="$HOME/.cargo/bin:$PATH"
# Install the toolchain then the musl target
RUN rustup toolchain install stable
RUN rustup target add $(cat rust_target.txt)

# Build
COPY . .
RUN cargo zigbuild --bin docuum --target $(cat rust_target.txt) --release
RUN cp target/$(cat rust_target.txt)/release/docuum /docuum

# A distroless base image
FROM scratch
WORKDIR /app

# Install Docuum.
COPY --from=builder /docuum .

# Set the entrypoint to Docuum. Note that Docuum is not intended to be run as
# an init process, so be sure to pass `--init` to `docker run`.
ENTRYPOINT ["/app/docuum"]
