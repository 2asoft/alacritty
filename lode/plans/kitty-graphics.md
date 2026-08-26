# Kitty graphics implementation

Status: Complete

## Outcome

This fork implements the complete Kitty terminal graphics protocol described by the accepted [RFC](../../docs/kitty-graphics-rfc.md) and tracked by the [conformance matrix](../../docs/kitty-graphics-conformance.md). Support is always available and uses capability queries rather than a changed terminal identity.

The implementation covers ordered queries, direct and local transports, RGB, RGBA, PNG, zlib, classic and placeholder placements, relative placement graphs, animation, bounded CPU and GPU resources, reflow-safe anchors, layered rendering, texture tiling, and context recovery. Non-Unix shared-memory transmission returns `ENOTSUP`; this fork makes no Windows KGP shared-memory conformance claim.

## Accepted decisions

- The official Kitty protocol is authoritative for wire behavior. The RFC defines private-fork policy where the protocol does not.
- Canonical straight-alpha RGBA8 data belongs to per-screen terminal graphics state. GPU textures are disposable caches.
- Generic APC recognition streams data without whole-string buffering.
- Heavy transport, decode, allocation, and composition work forms an ordered parser barrier and runs outside `Term`'s mutex.
- Classic placements are independent terminal objects with reflow-aware anchors. Placeholder instances remain ordinary cells backed by immutable virtual placement prototypes.
- Local transports default on but have a configuration kill switch.
- Graphics support has no protocol-disable option. Resource policy remains configurable.
- Primary and alternate screens own independent graphics state.

## Compatibility boundaries

The vendored `vte` fork provides streaming APC lifecycle callbacks and ordered termination. Keep it pinned until a released upstream version satisfies the exit criteria in the [maintenance architecture](kitty-graphics-maintenance.md).

tmux clients require enabled DCS passthrough. tmux does not retain graphics payloads for a fresh terminal attachment, and initial incremental placeholder output may require `refresh-client`. Zellij 0.45.0 proxies Kitty graphics inside synchronized updates; every completion, timeout, and overflow replay path must preserve deferred barriers before parsing later bytes.

Tests derived from external scenarios use independent fixtures; GPL Kitty test code is not copied. The protocol remains the behavioral authority over any individual client implementation.

## Risks and rollback

Parser changes affect all terminal input, so generic APC support must remain isolated and existing OSC, DCS, PM, and SOS behavior must remain covered. Grid tracking must remove anchors pruned by normal capacity changes without remapping anchors detached by text-only scrollback erasure. Rendering must retain the empty-state fast path and controlled framebuffer coverage for every image stratum.

Graphics is always available. Rollback is source-level and should preserve coherent protocol increments rather than introduce a runtime feature switch.

## Related Lode

- [Graphics domain](../graphics/summary.md)
- [Maintenance architecture](kitty-graphics-maintenance.md)
- [Repository practices](../practices.md)
