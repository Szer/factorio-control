# Factorio Control

- Rust application code and tests forbid unsafe code and deny Clippy unwrap_used/expect_used. Handle or propagate errors explicitly.
- React uses strict TypeScript, checked indexed access, exact optional properties, and type-aware ESLint. No explicit any, non-null assertions, or ignored promises.
- Keep process ownership and filesystem mutations serialized. Validate external inputs and preserve working data on failure.
- Stage explicit paths only. Keep documentation concise and describe current behavior.
- Verify UI changes in a browser at desktop and narrow widths. Run relevant tests, formatting, and lint checks.

- Runtime state locks protect memory only. Run filesystem work in blocking workers under the dedicated operation gate; retain owned guards through worker cancellation.
- Test trust boundaries through real HTTP/transport routes, including attacker isolation and spoofed inputs. Compile-time types do not establish trust in logs, filenames, or forwarded headers.
- Test failure transitions relevant to each change: interrupted/retried operations, delayed replies, background refresh errors, session changes, and conflicting writes. Preserve drafts and working files on failure.
- Use realistic archive sizes and deterministic slow-I/O barriers for resource and concurrency changes. Keep the bundled-engine tests and image smoke test mandatory in CI.
