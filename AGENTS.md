# Factorio Control

- Rust application code and tests forbid unsafe code and deny Clippy unwrap_used/expect_used. Handle or propagate errors explicitly.
- React uses strict TypeScript, checked indexed access, exact optional properties, and type-aware ESLint. No explicit any, non-null assertions, or ignored promises.
- Keep process ownership and filesystem mutations serialized. Validate external inputs and preserve working data on failure.
- Stage explicit paths only. Keep documentation concise and describe current behavior.
- Verify UI changes in a browser at desktop and narrow widths. Run relevant tests, formatting, and lint checks.
