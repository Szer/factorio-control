# Factorio Control

A local Factorio control room built with Rust and strict React/TypeScript. Manage server lifecycle, saves, mods, startup settings, server settings, and administrators in a Factorio-style interface.

## Run locally

Use Rust 1.99.1 and Node 22. The service listens at http://127.0.0.1:9400 and stores files under `./data`.

```sh
cd web
npm ci
npm run build
cd ..
FC_FACTORIO_BIN=/absolute/path/to/factorio/bin/x64/factorio cargo run --locked
```

The executable is optional for exploring the UI; start stays disabled until it is configured. Upload a Factorio ZIP from Saves and select **Use on next start**. The selection persists across manager restarts.

## Docker on Linux

```sh
docker compose up --build -d
```

The image bundles the Linux x86-64 headless game and runs as UID/GID 10001. Compose uses host networking so the control panel stays on loopback; game traffic uses UDP 34197.

To change Factorio version, set one line in `.env`, then rebuild:

```sh
FACTORIO_VERSION=2.1.23
```

The persistent `factory-data` volume holds saves, mods, settings, administrators, and game runtime files. Configuration writes are atomic and revision-checked; uploads never overwrite an existing save.

## Overview and telemetry

Overview shows server state and its start time, online players, research progress, recent activity, and UPS history. Manager settings control the polling interval and history duration.

Detailed telemetry is opt-in: enable it in **Settings → Manager settings**. It uses Factorio script commands through private loopback RCON, which disable achievements for that save. Without it, lifecycle and player events remain available from game logs.

UPS is measured from elapsed game ticks against wall-clock time, with a target adjusted for game speed. Paused or unavailable samples appear as gaps. History and activity are bounded in memory and reset when the manager restarts.

## Saves, mods, and settings

- Saves support streamed ZIP uploads up to 1 GiB, download, and selection for the next server start. Sync mods previews the exact versions from a Factorio 2.0 or 2.1 save, downloads missing ZIPs, and disables extra mods while the server is stopped. Progress shows the current mod, completed downloads, validation, and activation. Existing archives and startup settings are preserved.
- Mods support ZIP upload, download, enable/disable, compatible update checks, and updates from the official mod portal. Changes require a stopped server; dependency validation reports missing or incompatible mods.
- Portal downloads require `FACTORIO_USERNAME` and `FACTORIO_TOKEN`, or the username and token in server settings. Upload and download of installed ZIPs require no portal account.
- Startup settings support typed edits and import/export of `mod-settings.dat`. Import a game-generated file to populate the editor; runtime settings are preserved.
- Settings has form and Raw JSON editors for `server-settings.json`, `server-adminlist.json`, and `manager-settings.json`. Unknown server-setting keys survive form edits; manager preferences accept only supported keys.

Edits survive page navigation. Conflicting writes preserve local drafts and require an explicit reload. Server, administrator, and mod startup settings apply when Factorio next starts.

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

Scheduled backups and an interactive RCON console are not implemented. Mod dependencies are validated but are not automatically downloaded.

Save uploads stream to temporary files with a 1 GiB limit. Mod ZIP uploads are capped at 64 MiB with a 256 MiB expanded-size limit. Archives are validated without extraction. Factorio determines save compatibility on launch.
