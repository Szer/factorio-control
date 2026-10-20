# Dockge deployment

Use the repository's `compose.yaml` as the stack definition. Set `FC_PUBLIC_URL` in Dockge's environment editor to the exact public HTTPS origin, without a trailing path. Set `CONTROL_IMAGE` to a published digest for a pinned deployment.

The image serves HTTP on container port 8080. The existing reverse proxy can keep using host port 8181 and must preserve the public Host header. Host HTTP binds to 127.0.0.1 unless `CONTROL_LISTEN_ADDRESS` is set. A container or remote proxy needs an address it can reach; allow that proxy through the host firewall. TLS terminates at the proxy. Do not publish RCON.

The game listens on container UDP 34197. `FACTORIO_PORT` changes the host port mapping; retain the game's internal port at 34197 unless the Compose mapping is also changed. A stopped or paused game does not make the manager unhealthy.

## First sign-in

Run from the stack directory or open a shell for the `control` service:

```sh
docker compose exec control cat /data/bootstrap-admin.json
```

Sign in as `admin`, then change the temporary password. The bootstrap file is deleted after the change. The file and accounts are stored in the persistent volume; replacing the image does not reset credentials.

## Import from FSM

Keep FSM running while preparing an empty Control stack on different host ports. Upload a copy of a save and mod ZIPs through Control, import `mod-settings.dat`, and copy server settings and the separate administrator list using their Raw JSON editors. Factorio.com credentials are independent of panel accounts.

For a filesystem copy, stop both game servers and back up the original data first. Copy the contents of FSM's `saves` and `mods` directories into `/data/saves` and `/data/mods` in the new volume. Copy these files from its `config` directory into `/data` when present:

- `server-settings.json`
- `server-adminlist.json`
- `server-banlist.json`
- `server-whitelist.json`

Retain existing destination panel accounts and manager settings. Do not copy FSM's manager database or `config.ini`; Control creates its own runtime paths. Give the copied destination files to UID/GID 10001 and restrict the data directory to its owner. Never change ownership or permissions of FSM's original directories for Control.

Start Control, select the imported save, inspect settings and mods, then start the game. Switch the reverse proxy to Control after verification. Two servers cannot use the same host UDP port or safely write to the same world and mods directories.

## Upgrade and rollback

Stop the game through Control and back up the full data volume, then replace `CONTROL_IMAGE` with the new digest and redeploy. Choose the previous digest to roll the application back. A world saved by a newer Factorio version may require restoring its pre-upgrade backup to run on an older engine.

The image is Linux AMD64 only. Check the GitHub Actions run summary for the published digest and game-version label.
