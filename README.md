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

The executable is optional for exploring the UI; start stays disabled until it is configured. Upload a Factorio ZIP from Saves and select **Use on next start**, or enable **Always load latest** to resolve the newest save, including autosaves, on every start. Existing fixed selections remain fixed.

## Docker on Linux

```sh
docker compose up --build -d
```

The image bundles the Linux x86-64 headless game and runs as UID/GID 10001. Compose uses host networking so the control panel stays on loopback; game traffic uses UDP 34197.

To change Factorio version, set one line in `.env`, then rebuild:

```sh
FACTORIO_VERSION=2.1.23
```

The persistent `factory-data` volume holds saves, mods, settings, panel accounts, and game runtime files. Configuration writes are atomic and revision-checked; uploads never overwrite an existing save. Game bind address and UDP port are configured under Manager settings and apply on the next start. Runtime paths shows the executable, configuration, and read/write data paths without exposing unrelated configuration values.

## Overview and telemetry

Overview shows server state and its start time, online players, research progress, recent activity, and UPS history. Manager settings control the polling interval and history duration.

Detailed telemetry is opt-in: enable it in **Settings → Manager settings**. It uses Factorio script commands through private loopback RCON, which disable achievements for that save. Without it, lifecycle and player events remain available from game logs.

UPS is measured from elapsed game ticks against wall-clock time, with a target adjusted for game speed. Paused or unavailable samples appear as gaps. History and activity are bounded in memory and reset when the manager restarts.

## Saves, mods, and settings

- Saves support streamed ZIP uploads up to 1 GiB, download, fresh world creation, recoverable archive/restore, and fixed or latest-save startup. World creation uses a private temporary runtime and publishes only a validated ZIP. Selected saves cannot be archived; restoration never overwrites an existing save. Sync mods previews the exact versions from a Factorio 2.0 or 2.1 save, downloads missing ZIPs, and disables extra mods while the server is stopped. Progress shows the current mod, completed downloads, validation, and activation. Existing archives and startup settings are preserved.
- Mods support ZIP upload, download, enable/disable, portal search and release selection, compatible upgrades, bulk update previews, removal, named mod packs, and import/export of `mod-list.json`. Download all mods exports installed ZIPs and configuration together. Changes require a stopped server; dependency validation reports missing or incompatible mods.
- Mod transactions validate staged archives before activation. A recovery journal restores configuration after an interrupted commit; archives remain available for pinned versions and mod packs.
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

## Console and recovery

Console sends bounded, single-line commands or chat to the owned game process. Commands that may change the world require acknowledgement; scripting can disable achievements. Command text is not recorded by the manager. Emergency force-stop requires confirmation and can lose unsaved progress.

The game log tab reads a bounded, redacted tail of `factorio-current.log`. Live output remains bounded in memory. Save archives are stored under the data directory and can be restored through Saves.

## Panel access

Authentication is required on every installation, including loopback. On first start, the manager creates `admin` with a unique 256-bit random temporary password. Read it from the running container:

```sh
docker compose exec control cat /data/bootstrap-admin.json
```

The file lives in the persistent data volume, has mode `0600`, and is never printed in logs or served through the API. With a custom `FC_DATA_DIR`, it is stored in that directory instead. Sign in, then change the temporary password to unlock the panel. Only password change and sign-out are available until this is complete. A successful password change deletes the file and revokes every session for that account. Startup retries cleanup after an interrupted change. Restarts preserve existing accounts and never reset their passwords; a missing account file after setup disables startup.

Passwords use Argon2id v19 with 19 MiB memory, two iterations, one lane, a unique 16-byte random salt and a 32-byte output, matching the OWASP Password Storage Cheat Sheet minimum. Only salted PHC hashes are stored in `panel-accounts.json`. Password verification runs in a bounded worker pool and login attempts are rate-limited.

Use at least 12 characters for your replacement password. Panel accounts are separate from Factorio.com credentials and game administrators. Administrators manage accounts and server configuration; operators manage game operations, saves, and mods; viewers have read-only access. Password and role changes revoke the affected sessions. The final administrator cannot be removed or demoted.

For a reverse proxy, add `FC_PUBLIC_URL` to the service's `environment` with the exact HTTPS origin (for example `https://factorio.example.com`) and preserve its Host header. Bind to loopback when the proxy runs on the same host; a nonloopback `FC_BIND` requires an explicit HTTPS public origin. TLS terminates at the proxy; forwarded headers do not expand trusted origins. Session cookies are HttpOnly, SameSite=Strict, and Secure with a public origin. Sessions expire and must be re-established after manager restarts.

Keep `panel-accounts.json` and the data directory private. Back them up before moving installations. Choose your permanent password during first sign-in. Do not share or retain the bootstrap credential file.

## Current boundaries

Scheduled backups and automatic dependency installation are not implemented. Mod dependency conflicts are reported before activation.

Save uploads stream to temporary files with a 1 GiB limit. Mod ZIP uploads are capped at 64 MiB with a 256 MiB expanded-size limit. Archives are validated without extraction. Factorio determines save compatibility on launch.
