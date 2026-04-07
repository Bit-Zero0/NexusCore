FROM rust:1.77-bookworm AS builder

WORKDIR /workspace
COPY . .

RUN cargo build -p nexus-alert-relay --release

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --create-home --uid 10001 nexus
WORKDIR /app
COPY --from=builder /workspace/target/release/nexus-alert-relay /usr/local/bin/nexus-alert-relay

USER nexus
EXPOSE 18081

CMD ["nexus-alert-relay"]
