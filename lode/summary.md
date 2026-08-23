# Alacritty fork

This repository contains Alacritty, its reusable terminal core, configuration crates, and a private Kitty graphics implementation. PTY bytes flow through `vte` and `alacritty_terminal`; the application snapshots terminal state and renders it with OpenGL.

## Boundaries

- `alacritty_terminal` owns terminal semantics, grids, screen state, and protocol state.
- `alacritty` owns windows, configuration reload, scheduling, display snapshots, and GPU resources.
- `alacritty_config` and `alacritty_config_derive` own configuration loading support.
- The external `vte` dependency owns generic VT byte-stream recognition. This fork may need a pinned fork to expose streaming APC events.

## Governing constraints

- Preserve ordinary terminal behavior and the no-placement fast path while graphics support remains always available.
- Treat PTY output and local transport references as untrusted input.
- Keep canonical image data in terminal-owned CPU state. GPU resources are disposable caches.
- Keep protocol operations ordered and large I/O, decode, and allocation work outside the terminal mutex.
- Do not change terminal identity to advertise graphics support.

## Domains

- [Kitty graphics](graphics/summary.md)
