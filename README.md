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

## Docker and Dockge

The public image `ghcr.io/szer/factorio-control` bundles Control and the Linux x86-64 Factorio headless server. It runs as UID/GID 10001 with a read-only root filesystem, dropped capabilities, and a persistent `/data` volume. Authentication is mandatory. The container health check verifies the manager independently of whether the game is running.

Copy `compose.yaml` into a Dockge stack, set `FC_PUBLIC_URL` to the exact HTTPS origin and `FC_TRUSTED_PROXIES` to the real proxy IP addresses, then deploy. For Docker Compose:

```sh
cp .env.example .env
# Set your HTTPS origin and exact proxy IPs in .env.
docker compose up -d
```

The panel uses container TCP 8080 and host TCP 8181; the game uses UDP 34197. Host HTTP binds to loopback by default. Set `CONTROL_LISTEN_ADDRESS` to an address reachable by your reverse proxy if it runs elsewhere, and restrict that port to the proxy. Preserve the original Host header. RCON uses loopback TCP 27015 and needs no published port. Set `FC_RCON_PORT` to another nonzero port when running multiple managers in the same network namespace.

Read the initial administrator password with `docker compose exec control cat /data/bootstrap-admin.json`. Change it in the panel, create or upload a save, then start the game. Game startup is controlled through the panel. See [Dockge and FSM migration](docs/dockge.md) for existing installations.

`CONTROL_IMAGE` accepts a release tag or `ghcr.io/szer/factorio-control@sha256:...`. CI publishes `latest`, `factorio-<game-version>` and `sha-<full-commit>` after checks pass; `v*` Git tags publish version tags. Pin a digest for deployments; `latest` and the game-version alias move as Control changes.

To bump the bundled game, run `python3 scripts/update-factorio.py 2.1.23` with the desired version, review and commit `.factorio-version` and `.factorio-sha256`, then push to `main`. Image builds verify the pinned archive checksum before extraction. To build locally:

```sh
docker compose -f compose.yaml -f compose.build.yaml up --build -d
```

The persistent `factory-data` volume holds saves, mods, settings, panel accounts and game runtime files. Back it up before changing versions. Configuration writes are atomic and revision-checked; save uploads never overwrite existing files. Container recreation preserves the volume; `docker compose down -v` removes it.

## Overview and telemetry

Overview shows server state and its start time, online players, research progress, recent activity, and UPS history. Manager settings control the polling interval and history duration.

Detailed telemetry is opt-in: enable it in **Settings → Manager settings**. It uses Factorio script commands through private loopback RCON, which disable achievements for that save. Without it, lifecycle and player events remain available from game logs.

UPS is measured from elapsed game ticks against wall-clock time, with a target adjusted for game speed. Paused or unavailable samples appear as gaps. History and activity are bounded in memory and reset when the manager restarts. Frequent research queries cover current, queued, and previously observed research. A full reconciliation every 30 seconds catches unqueued or script-triggered completions without scanning every technology on each UPS poll.

## Saves, mods, and settings

- Saves support streamed ZIP uploads up to 1 GiB, download, fresh world creation, recoverable archive/restore, and fixed or latest-save startup. World creation uses a private temporary runtime and publishes only a validated ZIP. Selected saves cannot be archived; restoration never overwrites an existing save. Sync mods previews the exact versions from a Factorio 2.0 or 2.1 save, downloads missing ZIPs, and disables extra mods while the server is stopped. Progress shows the current mod, completed downloads, validation, and activation. Existing archives and startup settings are preserved.
- Mods support ZIP upload, download, enable/disable, portal search and release selection, compatible upgrades, bulk update previews, removal, named mod packs, and import/export of `mod-list.json`. Download all mods exports installed ZIPs and configuration together. Changes require a stopped server; dependency validation reports missing or incompatible mods.
- Mod ZIP uploads and portal downloads stream to disk, with limits of 1 GiB compressed and 4 GiB expanded per archive.
- Mod transactions validate staged archives before activation. A recovery journal restores configuration at manager startup and before mutations after an interrupted commit; archives remain available for pinned versions and mod packs.
- Portal downloads require `FACTORIO_USERNAME` and `FACTORIO_TOKEN`, or the username and token in server settings. Upload and download of installed ZIPs require no portal account.
- Startup settings support typed edits and import/export of `mod-settings.dat`. Import a game-generated file to populate the editor; runtime settings are preserved.
- Settings has form and Raw JSON editors for `server-settings.json`, `server-adminlist.json`, and `manager-settings.json`. Unknown server-setting keys survive form edits; manager preferences accept only supported keys.

Settings survive page navigation. Session expiry hides drafts in tab memory; signing back into the same account and role restores them. Signing out or changing identity discards drafts. Conflicting writes preserve local drafts and require an explicit reload. Server, administrator, and mod startup settings apply when Factorio next starts.

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

Rust owns the OpenAPI schema; frontend response validators are checked against generated types. Rust application code and tests forbid unsafe code and deny unwrap/expect; CI checks both languages. CI also runs dependency auditing, failure/concurrency regressions, real-engine telemetry/save tests, and the hardened image lifecycle smoke test. See [review findings and test coverage](docs/review-retrospective.md).

## Console and recovery

Console sends bounded, single-line commands or chat to the owned game process. Commands that may change the world require acknowledgement; scripting can disable achievements. Command text is not recorded by the manager. Emergency force-stop requires confirmation and can lose unsaved progress.

The game log tab reads a bounded, redacted tail of `factorio-current.log`. Live output remains bounded in memory. Save archives are stored under the data directory and can be restored through Saves.

## Panel access

Authentication is required on every installation, including loopback. On first start, the manager creates `admin` with a unique 256-bit random temporary password. Read it from the running container:

```sh
docker compose exec control cat /data/bootstrap-admin.json
```

The file lives in the persistent data volume, has mode `0600`, and is never printed in logs or served through the API. With a custom `FC_DATA_DIR`, it is stored in that directory instead. Sign in, then change the temporary password to unlock the panel. Only password change and sign-out are available until this is complete. A successful password change deletes the file and revokes every session for that account. Startup retries cleanup after an interrupted change. Restarts preserve existing accounts and never reset their passwords; a missing account file after setup disables startup.

Passwords use Argon2id v19 with 19 MiB memory, two iterations, one lane, a unique 16-byte random salt and a 32-byte output, matching the OWASP Password Storage Cheat Sheet minimum. Only salted PHC hashes are stored in `panel-accounts.json`. Password verification runs in two bounded workers. Login attempts are limited to five per minute per verified client IP; account names do not create shared lockouts.

Use at least 12 characters for your replacement password. Panel accounts are separate from Factorio.com credentials and game administrators. Administrators manage accounts and server configuration; operators manage game operations, saves, and mods; viewers have read-only access. Password and role changes revoke the affected sessions. The final administrator cannot be removed or demoted.

For a reverse proxy, configure `FC_TRUSTED_PROXIES` with its exact IP addresses as seen by Control; restrict ingress to those proxies and have the proxy replace or append the real client address in `X-Forwarded-For`. Only configured transport peers can supply forwarded addresses. Public HTTPS deployment refuses to start without explicit proxy trust. Local operation with no public URL ignores forwarded headers unless configured; clients sharing NAT share one rate bucket. Do not guess a Docker gateway address. See [Dockge proxy configuration](docs/dockge.md).

For a reverse proxy, add `FC_PUBLIC_URL` to the service's `environment` with the exact HTTPS origin (for example `https://factorio.example.com`) and preserve its Host header. Bind to loopback when the proxy runs on the same host; a nonloopback `FC_BIND` requires an explicit HTTPS public origin. TLS terminates at the proxy; forwarded headers do not expand trusted origins. Session cookies are HttpOnly, SameSite=Strict, and Secure with a public origin. Sessions expire and must be re-established after manager restarts.

Keep `panel-accounts.json` and the data directory private. Back them up before moving installations. Choose your permanent password during first sign-in. Do not share or retain the bootstrap credential file.

## Current boundaries

Scheduled backups and automatic dependency installation are not implemented. Mod dependency conflicts are reported before activation.

Save uploads stream to temporary files with a 1 GiB limit. Mod ZIP uploads stream to disk with a 1 GiB compressed and 4 GiB expanded-size limit. Archives are validated without extraction. Factorio determines save compatibility on launch.

## License

Factorio Control source is [MIT licensed](LICENSE). Factorio is proprietary software from Wube Software and remains subject to [its own terms](https://factorio.com/terms-of-service). Frontend dependency and font notices are included at `/third-party-notices.txt` in the image.
