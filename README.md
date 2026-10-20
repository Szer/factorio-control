# Factorio Control

A local Factorio control room built with Rust and strict React/TypeScript. The first release manages saves, server settings, game administrators, process lifecycle, and recent logs.

## Run locally

Use Rust 1.99.1 and Node 22. The service listens at http://127.0.0.1:9400 and stores files under `./data`.

```sh
cd web
npm ci
npm run build
cd ..
FC_FACTORIO_BIN=/absolute/path/to/factorio/bin/x64/factorio cargo run --locked
```

The executable is optional for exploring the UI; start stays disabled until it is configured. Upload a Factorio ZIP save from Worlds, then choose it in the start dialog.

## Docker on Linux

```sh
docker compose up --build -d
```

The image bundles the Linux x86-64 headless game and runs as UID/GID 10001. Compose uses host networking so the control panel stays on loopback; game traffic uses UDP 34197.

To change Factorio version, set one line in `.env`, then rebuild:

```sh
FACTORIO_VERSION=2.1.23
```

The persistent `factory-data` volume holds saves, settings, administrators, and game runtime files. Manager settings and administrator files are replaced atomically; upload never overwrites an existing save.

## Development

Run `cargo run --locked` at the project root and `npm run dev` in `web`. Vite proxies `/api` to the Rust service; keep its default Host forwarding.

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo run --locked -- --openapi > openapi.json
cd web
npm run api:generate
npm run format:check
npm run check
npm test
npm run build
```

Rust owns the OpenAPI schema; frontend response validators are checked against generated types. Rust application code and tests forbid unsafe code and deny unwrap/expect; CI checks both languages.

## Current boundaries

This release is local-only: no panel authentication or remote deployment yet. The service rejects non-loopback bind addresses and foreign Host/Origin headers.

Logs refresh every two seconds. Mod management, RCON commands, scheduled backups, and multiplayer telemetry are not implemented; SQLite is deferred until there is persistent manager metadata to store.

Settings and administrator edits survive page navigation. Conflicting writes preserve local edits and require an explicit reload; settings apply when Factorio next starts.

Uploads are capped at 64 MiB and structurally checked as Factorio ZIPs without extraction. Game-version compatibility is determined by Factorio on launch.
