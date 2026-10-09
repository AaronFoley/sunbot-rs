FROM rust:1.99.0-slim-bookworm AS builder

WORKDIR /sunbot

RUN apt-get update \
    && apt-get upgrade -y \
    && apt-get install -y pkg-config libssl-dev \
    && apt-get autoremove --purge -y $(cat /tmp/cleanup-packages.txt) \
    && apt-get clean && rm -rf /var/lib/apt/lists/* /tmp/* /var/tmp/* /var/cache/apk/

# Copy only the Cargo files so that this layer only contains the dependencies
COPY ./Cargo.lock ./Cargo.lock
COPY ./Cargo.toml ./Cargo.toml

# We need to create dummy projects for each crate
RUN USER=root cargo new --bin crates/sunbot
RUN USER=root cargo new --lib crates/sunbot_config
RUN USER=root cargo new --lib crates/sunbot_db
RUN USER=root cargo new --lib crates/sunbot_migrations

COPY crates/sunbot/build.rs ./crates/sunbot/
COPY crates/sunbot/Cargo.toml ./crates/sunbot/
COPY crates/sunbot_config/Cargo.toml ./crates/sunbot_config/
COPY crates/sunbot_db/Cargo.toml ./crates/sunbot_db/
COPY crates/sunbot_migrations/Cargo.toml ./crates/sunbot_migrations/

RUN cargo build --release
RUN rm -rf crates/**/*.rs

# copy your source tree
COPY ./crates ./crates

# build for release
RUN rm -rf ./target/release/deps/sunbot* ./target/release/.fingerprint/sunbot*
RUN cargo build --release

FROM debian:bookworm-slim

WORKDIR /sunbot
ENV TZ=Australia/Sydney

RUN apt-get update \
    && apt-get upgrade -y \
    && apt-get install -y pkg-config libssl-dev ca-certificates tzdata \
    && apt-get autoremove --purge -y $(cat /tmp/cleanup-packages.txt) \
    && apt-get clean && rm -rf /var/lib/apt/lists/* /tmp/* /var/tmp/* /var/cache/apk/

COPY --from=builder /sunbot/target/release/sunbot /sunbot

# Setup local app user
RUN groupadd -g 442 app && \
    useradd -u 442 -g 442 -M -d /sunbot -c 'app user' app && \
    chown -R app:app /sunbot

USER app
CMD ["./sunbot"]
