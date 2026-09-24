FROM lukemathwalker/cargo-chef:latest-rust-1.95-trixie AS chef
WORKDIR /app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
RUN apt-get update && \
    apt-get install -y --no-install-recommends libclang-dev m4 pkg-config && \
    rm -rf /var/lib/apt/lists/*

# Build profile, maxperf by default
ARG BUILD_PROFILE=maxperf
ENV BUILD_PROFILE=$BUILD_PROFILE

# Extra Cargo features. The defaults already include jemalloc, asm-keccak and keccak-cache-global.
ARG FEATURES=""
ENV FEATURES=$FEATURES

# Rust compiler flags. When left empty, linux/amd64 builds target x86-64-v3 (Haswell and newer)
# plus pclmulqdq for rocksdb, and other platforms use the baseline ISA. To build a baseline amd64
# image pass a non-empty value, e.g. --build-arg RUSTFLAGS="-C target-cpu=x86-64".
# TARGETPLATFORM is set by BuildKit: https://docs.docker.com/reference/dockerfile/#automatic-platform-args-in-the-global-scope
ARG RUSTFLAGS=""
ARG TARGETPLATFORM
RUN if [ -z "$RUSTFLAGS" ] && [ "$TARGETPLATFORM" = "linux/amd64" ]; then \
        RUSTFLAGS="-C target-cpu=x86-64-v3 -C target-feature=+pclmulqdq"; \
    fi && \
    echo "export RUSTFLAGS=\"$RUSTFLAGS\"" > /rustflags.sh

# Build dependencies with the same flags as the final binary, otherwise the cached layer is
# discarded: any RUSTFLAGS change invalidates every compiled artifact.
COPY --from=planner /app/recipe.json recipe.json
RUN . /rustflags.sh && \
    cargo chef cook --profile "$BUILD_PROFILE" --features "$FEATURES" --recipe-path recipe.json

COPY . .
RUN . /rustflags.sh && \
    cargo build --profile "$BUILD_PROFILE" --features "$FEATURES" --locked --bin ethgas-node && \
    cp "/app/target/$BUILD_PROFILE/ethgas-node" /app/ethgas-node

FROM ubuntu:24.04 AS runtime

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates libssl3t64 && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/ethgas-node /usr/local/bin/

EXPOSE 8545 8546 8551 30303 30303/udp 9001

ENTRYPOINT ["/usr/local/bin/ethgas-node"]
