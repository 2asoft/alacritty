# Kitty graphics maintenance: rendering interfaces

Parent plan: [Kitty graphics maintenance](kitty-graphics-maintenance.md)

### Render snapshot boundary

Move placeholder resolution out of `alacritty/src/display/mod.rs` and into the terminal render snapshot boundary.

Expose only these render-facing types from `alacritty_terminal::graphics`:

```rust
#[derive(Default)]
pub struct GraphicsRenderSnapshot {
    pub classic: Vec<RenderableGraphic>,
    pub placeholders: Vec<RenderablePlaceholder>,
}

pub struct RenderablePlaceholder {
    pub point: Point<usize>,
    pub row: u16,
    pub column: u16,
    pub prototype: RenderableGraphic,
}
```

Add:

```rust
impl<T> Term<T> {
    pub fn graphics_render_snapshot(&self) -> GraphicsRenderSnapshot;
}
```

The method uses the active grid's current display offset and screen size. Classic renderables retain grid-relative lines. Placeholder `point` is already a viewport coordinate, matching `RenderableCell` and the application's glyph-suppression key. The application remains responsible for conversion from snapshot geometry to framebuffer pixels.

Internally, `GraphicsState` builds one prototype index:

```rust
type VirtualPrototypeKey = (u32, u32);
type VirtualPrototypeIndex = HashMap<VirtualPrototypeKey, PlacementHandle>;

impl GraphicsState {
    fn virtual_prototypes(&self) -> VirtualPrototypeIndex;
    fn required_virtual_origins(&self) -> HashSet<VirtualPrototypeKey>;
    fn renderable_for_virtual(
        &self,
        placement: PlacementHandle,
    ) -> Option<RenderableGraphic>;
}
```

The index stores placement handles rather than cloned renderables. Skip virtual placements without a nonzero external image ID; never collapse anonymous images onto `(0, 0)`. For every virtual placement with nonzero placement ID, insert its exact key. For every identified image, key `(image_id, 0)` selects the virtual placement with the lowest `creation_serial`. Exact keys also select the lowest creation serial if corrupted or legacy state contains duplicates. Materialize a `RenderableGraphic` only after a visible placeholder resolves through the index.

`required_virtual_origins` contains only virtual roots reached by classic relative placement chains, using the same `(image_id, placement_id.unwrap_or(0))` identity as placement resolution. If it is empty, `graphics_render_snapshot` scans only the visible viewport. If it is non-empty, scan the complete active grid range from `topmost_line()` through the screen bottom once, record independent minimum row and column for required keys, and collect visible placeholders during that same traversal. Preserve left-to-right inheritance state independently for every line and reset it after every non-placeholder cell.

If neither classic nor virtual placements exist, return `GraphicsRenderSnapshot::default()` without allocating an index or scanning any cell.

A placeholder enters `snapshot.placeholders` only when:

- its image/prototype resolves through the frame-local index;
- prototype rows and columns are present;
- placeholder row and column are inside that virtual grid.

The application suppresses glyphs and decorations for every returned viewport point, then computes framebuffer geometry. Unresolved or out-of-range placeholders remain ordinary visible text.

After migration, remove public `decode_placeholder`, `GraphicsState::placeholder_renderable`, `GraphicsState::renderables_with_virtual_origins`, and `Term::graphics` unless a verified non-test caller remains.

### Renderer pass selection

After sorting renderables, define:

```rust
let very_negative_end = graphics.partition_point(|image| image.z_index < i32::MIN / 2);
let negative_end = graphics.partition_point(|image| image.z_index < 0);
let split_cells = very_negative_end < negative_end;
```

Use two cell passes only when `split_cells` is true. Otherwise:

1. Draw very-negative images.
2. Draw cells once with normal background alpha and placeholder suppression.
3. Draw nonnegative images.
4. Draw cursor and UI overlays in the existing order.

When `split_cells` is true, preserve the current background-only and glyph-only passes around the middle negative stratum.

### Fuzz-only boundary

Do not keep mutation types public solely for `fuzz/`. Add an opt-in crate feature:

```toml
[features]
fuzzing = []
```

Enable it only in `fuzz/Cargo.toml` with `alacritty_terminal = { path = "../alacritty_terminal", features = ["fuzzing"] }`. The fuzz crate is a separate workspace and is not compiled by `cargo test --workspace`. Expose one feature-gated helper:

```rust
#[cfg(feature = "fuzzing")]
impl<T: EventListener> Term<T> {
    pub fn process_graphics_barrier_for_fuzzing(&mut self) {
        let mut work = self.take_deferred_graphics();
        while let Some(current) = work {
            work = self.commit_deferred_graphics(current.process());
        }
    }
}
```

This helper runs without a mutex in the fuzz harness but exercises the same request, processing, continuation, and commit implementations as production. No other parser, transaction, or mutation type becomes public for fuzzing.
