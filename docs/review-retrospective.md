# Review findings and test strategy

The review exposes missing behavioral coverage across trust boundaries, concurrency, recovery, and browser lifecycles. Passing type checks and happy-path tests does not establish those properties.

## Causes and prevention

| Gap | Why existing checks miss it | Required coverage |
| --- | --- | --- |
| Trusted metadata derived from logs | A well-formed string can still come from player chat. | Spoofed chat/log lines never change the version obtained from the configured binary. |
| Authentication availability | A test that proves throttling activates does not prove which clients it blocks. | Separate real TCP clients, targeted usernames, forged forwarding headers, cache saturation/expiry and an isolated reverse-proxy container with verified transport identity. |
| Unrealistic resource fixtures | Tiny ZIPs do not exercise real asset sizes, streaming or async worker starvation. | Chunked 70 MiB mod and 200 MiB save uploads, bounded archive validation and slow export barriers. |
| Recovery specified as rejection | Preserving an error is weaker than recovering an interrupted operation. | Restart, retry and every mutating entry point recover valid journals; malformed journals fail closed. |
| Function tests bypass routing | A valid operation can be unreachable because a static route shadows a mod name. | Exercise every reserved mod name through the HTTP router. |
| Browser tests omit timing | Isolated editor tests omit parent unmounts; cancelling a query does not cancel side effects inside its request helper. | Deferred polls/mutations and old-session replies, refresh failures, full-App settings drafts across expiry/re-authentication/cache collection, and real modal navigation. |
| Packaging checks omit engine contracts | Fake processes cannot establish Factorio argument or Lua behavior. | Bundled-engine telemetry/save tests and hardened container lifecycle smoke tests before image publication. |
| Review confidence exceeds evidence | Passing counts describe executed assertions, not the absence of defects. | Report scenarios, failed-baseline evidence and remaining deployment constraints alongside results. |

Rust and strict TypeScript remain useful for memory safety and API contracts. Typed client identities, stable account IDs, session generations and shared operation/session helpers encode stronger boundaries; behavioral tests establish their intended policy.

## Finding disposition

| Finding | Guard and regression coverage |
| --- | --- |
| Chat overrides engine version | Process output cannot assign engine version; spoofed messages cover stopped, starting, running and paused states; image smoke sends real chat. |
| Shared login/account lockout | Per-transport-client throttling, explicit proxy trust, bounded hashing and isolated-client TCP regressions. |
| Large mod ZIP prevents start | File-backed upload/download/validation and streaming copy/hash; 70 MiB upload/list/start/pack lifecycle regression. |
| Migrated whitelist ignored | Explicit whitelist setting and whitelist/banlist arguments; nonempty imports require an explicit decision; migrated bytes remain intact. |
| Heavy I/O blocks runtime | Dedicated storage gate plus blocking workers; deterministic export test checks HTTP status/telemetry and child output concurrently. |
| Console write blocks runtime | Owned stdin is written outside the runtime lock; a full child pipe cannot block status or emergency stop. |
| Recovery depends on unrelated action | Startup and mutation recovery share the same journal; retries and restarts are tested independently. |
| Refresh/session errors discard drafts | Cached data survives transient errors; expiry suspends UI; stable account/role identity controls draft restoration and stale callbacks. |
| Old request changes authentication | Request helpers scope 401 notifications to the initiating session generation and reject obsolete status replies, including renewed logins to the same account. |
| Temporary save selected as newest | `.tmp.zip` files, including uppercase variants, are excluded from save listings/latest selection. |
| Uppercase ZIP/name normalization | Upload media-type detection ignores extension case; create names trim whitespace without duplicating the extension. |
| RCON ephemeral-port race | Explicit loopback port 27015 replaces probe-and-release allocation; `FC_RCON_PORT` handles shared network namespaces. |
| Telemetry scans every technology | Frequent queries track current/queued research and prune cancellations; 30-second reconciliation catches unqueued completions; real-engine completion/cancellation/pause tests execute in CI. |
| Foreign directory/pack entry breaks listing | Listings skip names outside their managed namespace; valid archived saves/packs remain visible. |
| Crash leaves sync staging in mods | Staging resides outside the game's mod scan directory; startup cleans owned leftovers and recovers interrupted publication. |
| Reserved mod name cannot be removed | Explicit DELETE dispatch covers static routes as well as ordinary names. |
| Bundled game lacks checksum | Image extraction requires the pinned SHA-256; one command updates the version and digest together. |
| Startup Discard uses stale revision | Discard adopts the latest polled value and revision, including StrictMode replay coverage. |
| Poll overwrites mutation result | Cancel matching reads before publication; recheck session identity after awaits and publish cache/draft changes synchronously. |
| Validator dumps raw JSON | Invalid responses produce a short recovery-oriented message. |
| Latest-save notice has no name | The acknowledgement names the latest-save policy. |
| Duplicate admins remove together | Occurrence keys/removal preserve individual entries; backend still rejects duplicates on save. |
| Hidden dialog retains native modality | Route deactivation unmounts the dialog; Back navigation is tested. |
| Settings links shown to unauthorized roles | Render permission-appropriate guidance instead of unreachable links. |

## Evidence and limits

Selected regressions fail on the original implementation: spoofed version assignment, oversized mod rejection, static-route deletion, async export starvation and twelve browser lifecycle/response scenarios. These failures demonstrate defect detection rather than assertions that merely mirror the repaired implementation.

Full-App draft and shared-request-helper regressions also fail on intermediate fixes that passed isolated component tests. Five authentication lifecycle tests cover obsolete errors, successful status replies, password acknowledgements and logout acknowledgements across replacement sessions.

CI runs formatting, strict lint/type checks, Rust and browser regressions, release-script tests, high-severity dependency auditing, bundled-engine integration tests and image smoke tests before publication. Developer fixtures are synthetic and isolated from production credentials and worlds.

Public deployment requires correct reverse-proxy identity and restricted ingress. Shared NAT clients share a rate bucket, and bounded password workers do not provide distributed denial-of-service protection. Research reconciliation can delay unqueued completion notices by up to 30 seconds; changes that are reversed entirely between observations may be missed.

Factorio crash reports document temporary save files: [failed save leaves `.tmp.zip`](https://forums.factorio.com/viewtopic.php?t=123564). Authentication lockout policy follows [OWASP's denial-of-service considerations](https://cheatsheetseries.owasp.org/cheatsheets/Authentication_Cheat_Sheet.html#account-lockout).
