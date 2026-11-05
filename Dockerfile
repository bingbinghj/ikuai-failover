FROM node:26-bookworm-slim AS web
WORKDIR /web
COPY frontends/web/package.json frontends/web/package-lock.json ./
RUN npm ci
COPY frontends/web ./
RUN npm run build

FROM rust:1-bookworm AS build
WORKDIR /source
RUN apt-get update && apt-get install -y --no-install-recommends libssl-dev pkg-config && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock ./
COPY config.example.yml ./
COPY apps/failover apps/failover
RUN cargo build --locked --release -p ikb-failover

FROM debian:bookworm-slim
LABEL org.opencontainers.image.title="ikuai-failover" \
      org.opencontainers.image.description="iKuai WAN failover and automatic failback" \
      org.opencontainers.image.source="https://github.com/bingbinghj/ikuai-failover" \
      org.opencontainers.image.licenses="AGPL-3.0"
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates libssl3 && rm -rf /var/lib/apt/lists/*
COPY --from=build /source/target/release/ikuai-failover /usr/local/bin/ikuai-failover
COPY --from=web /web/dist /usr/share/ikuai-failover/web
COPY LICENSE /usr/share/licenses/ikuai-failover/LICENSE
COPY frontends/web/third-party/MoviePilot-LICENSE /usr/share/licenses/MoviePilot-Frontend/LICENSE
VOLUME ["/etc/ikuai-failover", "/var/lib/ikuai-failover"]
EXPOSE 19002/tcp
ENTRYPOINT ["/usr/local/bin/ikuai-failover"]
CMD ["--config", "/etc/ikuai-failover/config.yml"]
