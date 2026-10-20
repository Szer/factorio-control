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
COPY .factorio-version /tmp/factorio-version
COPY .factorio-sha256 /tmp/factorio-sha256
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl xz-utils && rm -rf /var/lib/apt/lists/*
RUN factorio_version="$(cat /tmp/factorio-version)" && \
    printf '%s' "$factorio_version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$' && \
    curl --fail --location --retry 3 "https://factorio.com/get-download/${factorio_version}/headless/linux64" -o /tmp/factorio.tar.xz && \
    printf '%s  /tmp/factorio.tar.xz\n' "$(cat /tmp/factorio-sha256)" | sha256sum --check --strict - && \
    tar -xJf /tmp/factorio.tar.xz -C /opt && rm /tmp/factorio.tar.xz

FROM service AS engine-tests
COPY --from=game /opt/factorio /opt/factorio
COPY tests/ tests/
RUN printf '{"width":32,"height":32}\n' > /tmp/map-gen.json && \
    /opt/factorio/bin/x64/factorio --create /tmp/review-world.zip --map-gen-settings /tmp/map-gen.json && \
    FC_TEST_FACTORIO_BIN=/opt/factorio/bin/x64/factorio FC_TEST_SAVE=/tmp/review-world.zip cargo test --locked --release --lib -- --ignored

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates libstdc++6 libgcc-s1 && rm -rf /var/lib/apt/lists/* && useradd --uid 10001 --create-home control && mkdir /data && chown control:control /data
WORKDIR /app
COPY --from=game /opt/factorio /opt/factorio
COPY --from=service /build/target/release/factorio-control /app/factorio-control
COPY --from=web /build/web/dist /app/web/dist
COPY LICENSE /usr/share/doc/factorio-control/LICENSE
USER 10001:10001
ENV FC_DATA_DIR=/data FC_FACTORIO_BIN=/opt/factorio/bin/x64/factorio FC_BIND=0.0.0.0:8080
EXPOSE 8080 34197/udp
HEALTHCHECK --interval=30s --timeout=10s --start-period=15s --retries=3 CMD ["/app/factorio-control", "--healthcheck"]
STOPSIGNAL SIGTERM
ENTRYPOINT ["/app/factorio-control"]
