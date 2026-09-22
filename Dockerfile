FROM lukemathwalker/cargo-chef:latest-rust-1.95-trixie AS chef
WORKDIR /app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
RUN apt-get update && \
    apt-get install -y --no-install-recommends libclang-dev m4 pkg-config && \
    rm -rf /var/lib/apt/lists/*

COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json

COPY . .
RUN cargo build --release --locked --bin ethgas-node

FROM ubuntu:24.04 AS runtime

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates libssl3t64 && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/ethgas-node /usr/local/bin/

EXPOSE 8545 8546 8551 30303 30303/udp 9001

ENTRYPOINT ["/usr/local/bin/ethgas-node"]
