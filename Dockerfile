FROM node:22-bookworm-slim AS web
WORKDIR /build/web
COPY web/package*.json ./
RUN npm ci --no-audit --no-fund
COPY web/ ./
RUN npm run check && npm test && npm run build

FROM rust:1.99.1-bookworm AS service
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
RUN cargo build --locked --release

FROM debian:bookworm-slim AS game
ARG FACTORIO_VERSION
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl xz-utils && rm -rf /var/lib/apt/lists/*
RUN test -n "$FACTORIO_VERSION" && curl --fail --location --retry 3 "https://factorio.com/get-download/${FACTORIO_VERSION}/headless/linux64" -o /tmp/factorio.tar.xz && tar -xJf /tmp/factorio.tar.xz -C /opt && rm /tmp/factorio.tar.xz

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates libstdc++6 libgcc-s1 && rm -rf /var/lib/apt/lists/* && useradd --uid 10001 --create-home control && mkdir /data && chown control:control /data
WORKDIR /app
COPY --from=game /opt/factorio /opt/factorio
COPY --from=service /build/target/release/factorio-control /app/factorio-control
COPY --from=web /build/web/dist /app/web/dist
USER 10001:10001
ENV FC_DATA_DIR=/data FC_FACTORIO_BIN=/opt/factorio/bin/x64/factorio
STOPSIGNAL SIGTERM
ENTRYPOINT ["/app/factorio-control"]
