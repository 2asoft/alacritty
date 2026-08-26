# Kitty graphics maintenance: execution contract

Parent plan: [Kitty graphics maintenance](kitty-graphics-maintenance.md)

## Commit workflow

For every coherent commit:

1. Add or update focused tests before implementation when they can constrain behavior.
2. Make the smallest complete change for the work item.
3. Run focused tests and checks.
4. Run `scripts/kitty-graphics-baseline.sh`.
5. Stage the replaced baseline bundle.
6. Amend the commit so its current generated output is included.
7. Validate `manifest.b3`, `git diff --check`, and a clean tree.

Prefer forward `test`, `refactor`, `perf`, and `docs` commits for this maintenance series. Rewrite an earlier feature commit only when a forward semantic increment would misrepresent the architecture. Preserve a backup reference before any rewrite. Do not push rewritten history without explicit approval.


## Risks and mitigations

- Two-stage deferred animation can accidentally resume parsing between stages. The event-loop helper owns the loop and returns only after final commit.
- Early animation snapshot can change protocol error precedence. Decode transmitted frame data before target preparation.
- Frame state can change while pixel work runs. Revalidate image handle and structural revision at commit.
- Streaming base64 can change padding or wire errors. Use padding-indifferent direct decoding, canonical local-name decoding, and independent padded, unpadded, malformed, split, and error-code tests.
- Moving placeholder decoding can change inheritance at line boundaries. Reset inheritance for every logical line and retain current tests.
- Conditional history scanning can miss virtual roots. Derive required keys from classic relative chains and retain independent-axis origin tests.
- One-pass cell rendering can alter transparency or decoration order. Gate with controlled framebuffer tests for every z stratum.
- API narrowing can weaken fuzz coverage. The feature-gated helper must call the exact production deferred methods.

## Rollback

Each work item is independently revertible before publication, except work items 1 through 4 form an ordered migration and should roll back as a group after work item 3 lands. The observational baseline records every state. Source rollback must retain the last verified protocol-complete commit. No data migration or configuration rollback is required.
