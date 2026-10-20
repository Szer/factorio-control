# Dockge deployment

Use the repository's `compose.yaml` as the stack definition. Set `FC_PUBLIC_URL` to the exact public HTTPS origin and `FC_TRUSTED_PROXIES` to the actual proxy IPs in Dockge's environment editor. Set `CONTROL_IMAGE` to a published digest for a pinned deployment.

The image serves HTTP on container port 8080. The existing reverse proxy can keep using host port 8181 and must preserve the public Host header. Host HTTP binds to 127.0.0.1 unless `CONTROL_LISTEN_ADDRESS` is set. A container or remote proxy needs an address it can reach; allow that proxy through the host firewall. TLS terminates at the proxy. Do not publish RCON.


## Proxy client addresses

Set `FC_TRUSTED_PROXIES` in Dockge to the exact comma-separated proxy IPs that connect to Control. The proxy must preserve Host and replace or append the actual client IP in one `X-Forwarded-For` header. Restrict port 8181 to those proxies; a trusted Docker gateway must not also represent untrusted direct traffic.

The setting accepts up to 64 IPv4/IPv6 addresses, with no CIDR ranges. Public HTTPS deployment requires this setting and refuses startup when it is empty. Local operation without a public URL can leave it empty and ignores forwarding headers. Clients sharing NAT share the five-attempts-per-minute login bucket; identify the real proxy network before public deployment instead of guessing an address.

The forwarded chain is bounded and evaluated from the trusted connection toward the client. Untrusted peers cannot choose their rate-limit identity by sending a header. Incorrectly configured trusted proxies fail login rather than trusting a fabricated address.

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

Start Control, select the imported save, and inspect settings and mods. If FSM used a whitelist, enable **Settings → Manager settings → Enable server whitelist** before starting. A nonempty imported whitelist without an explicit manager choice blocks startup; disabling it also requires saving that explicit choice. Whitelist and banlist paths are passed directly to Factorio. Switch the reverse proxy to Control after verification. Two servers cannot use the same host UDP port or safely write to the same world and mods directories.

## Upgrade and rollback

Stop the game through Control and back up the full data volume, then replace `CONTROL_IMAGE` with the new digest and redeploy. Choose the previous digest to roll the application back. A world saved by a newer Factorio version may require restoring its pre-upgrade backup to run on an older engine.

The image is Linux AMD64 only. Check the GitHub Actions run summary for the published digest and game-version label.
