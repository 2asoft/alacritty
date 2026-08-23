# Project practices

Follow repository formatting and test conventions. For Rust changes, run focused tests first, then `cargo +nightly fmt --all --check`, relevant workspace tests, and `cargo clippy --workspace --all-targets -- -D warnings` when practical. CI uses nightly formatting because repository settings require unstable rustfmt options.

## Graphics work

- Add behavior tests at the earliest responsible boundary before implementation when the expected result is known.
- Parse graphics control fields into typed commands. Use checked arithmetic for every untrusted size, offset, and coordinate conversion.
- Do not log payload bytes or echo unbounded input in protocol errors.
- Keep state replacement transactional. A failed replacement must preserve the previous image and placements.
- Keep image and metadata storage bounded independently.
- Verify rendering with controlled framebuffer output and screenshots for user-visible milestones.
- Update [graphics memory](graphics/summary.md) and the [active plan](plans/kitty-graphics.md) as verified phases complete.
