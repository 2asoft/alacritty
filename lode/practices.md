# Project practices

Follow repository formatting and test conventions. For Rust changes, run focused tests first, then `cargo +nightly fmt --all --check`, relevant workspace tests, and `cargo clippy --workspace --all-targets -- -D warnings` when practical. CI uses nightly formatting because repository settings require unstable rustfmt options.

## Feature branch history

- Keep each commit as a forward implementation step. Do not retain commits described as fixes for behavior introduced earlier on the same unpublished feature branch.
- Fold corrections into the commit that introduced the behavior, or rewrite them as a coherent later implementation increment when earlier source structure cannot express them.
- Drop unrelated cleanup from the feature branch unless it is required to build or verify the feature. Fold required mechanical cleanup into the commit that creates the requirement.
- Preserve operator work before rewriting and restore it afterward. Re-run direct verification against the rewritten tree.

## Graphics work

- Add behavior tests at the earliest responsible boundary before implementation when the expected result is known.
- Parse graphics control fields into typed commands. Use checked arithmetic for every untrusted size, offset, and coordinate conversion.
- Do not log payload bytes or echo unbounded input in protocol errors.
- Keep state replacement transactional. A failed replacement must preserve the previous image and placements.
- Keep image and metadata storage bounded independently.
- Verify rendering with controlled framebuffer output and screenshots for user-visible milestones.
- Update [graphics memory](graphics/summary.md) and the [active plan](plans/kitty-graphics.md) as verified phases complete.
