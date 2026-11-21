# Headless Ferrent daemon. Build from the repo root:
#
#   docker build -f packaging/docker/Dockerfile -t ferrent .
#
# The RPC port is published on the host; the TLS material lives in the volume
# so a restart keeps the same fingerprint.
# syntax=docker/dockerfile:1

FROM rust:1-trixie AS build

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential pkg-config cmake clang libclang-dev \
        libtorrent-rasterbar-dev libboost-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /src
COPY Cargo.toml Cargo.lock rust-toolchain.toml rustfmt.toml ./
COPY crates ./crates
COPY buf.yaml ./
RUN cargo build --release -p ferrent-daemon -p ferrent-cli --features ferrent-daemon/libtorrent

FROM debian:trixie-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends libtorrent-rasterbar2.0 ca-certificates \
    && rm -rf /var/lib/apt/lists/*

RUN useradd --system --uid 1000 --home-dir /var/lib/ferrent --shell /usr/sbin/nologin ferrent \
    && install -d -o ferrent -g ferrent /var/lib/ferrent /var/lib/ferrent/downloads

COPY --from=build /src/target/release/ferrentd /usr/bin/ferrentd
COPY --from=build /src/target/release/ferrent-cli /usr/bin/ferrent-cli
COPY packaging/docker/config.toml /etc/ferrent/config.toml
COPY packaging/docker/entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod 0755 /usr/local/bin/entrypoint.sh

USER ferrent
ENV FERRENT_CONFIG=/etc/ferrent/config.toml \
    FERRENT_DATA_DIR=/var/lib/ferrent \
    FERRENT_BIND=0.0.0.0:50129

VOLUME ["/var/lib/ferrent"]
EXPOSE 50129

# The daemon speaks gRPC over TLS; the CLI is the cheapest liveness probe.
HEALTHCHECK --interval=30s --timeout=10s --start-period=30s --retries=3 \
    CMD ferrent-cli --server https://localhost:50129 \
        --ca /var/lib/ferrent/tls-cert.pem --token "$FERRENT_TOKEN" \
        list --limit 1 >/dev/null || exit 1

ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
