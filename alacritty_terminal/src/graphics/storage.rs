use std::collections::{HashMap, HashSet};
use std::num::NonZeroU32;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crate::index::{Line, Point};

use super::{
    Action, AnimationFrame, AnimationState, Command, DEFAULT_FRAME_GAP_MS, FrameComposition,
    GraphicsError, MAX_FRAMES_PER_BUFFER, MAX_FRAMES_PER_IMAGE, PixelBuffer, Placement,
    PlacementHandle, PlacementInsert, PlacementSpec, Placements, RenderableGraphic, blank_frame,
    compose,
};

pub const MAX_IMAGES_PER_BUFFER: usize = 4096;

static NEXT_IMAGE_HANDLE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ImageHandle(u64);

impl ImageHandle {
    fn allocate() -> Result<Self, GraphicsError> {
        NEXT_IMAGE_HANDLE
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |handle| handle.checked_add(1))
            .map(Self)
            .map_err(|_| GraphicsError::TooLarge)
    }
}

#[derive(Clone, Debug)]
pub struct Image {
    handle: ImageHandle,
    external_id: Option<NonZeroU32>,
    image_number: Option<u32>,
    pixels: PixelBuffer,
    frames: Vec<AnimationFrame>,
    current_frame: usize,
    root_gap_ms: i32,
    animation_state: AnimationState,
    loops: u32,
    completed_loops: u32,
    next_frame_at: Option<Instant>,
    content_generation: u64,
    frame_revision: u64,
    creation_serial: u64,
    last_used_serial: u64,
    transient: bool,
}

impl Image {
    pub fn handle(&self) -> ImageHandle {
        self.handle
    }

    pub fn external_id(&self) -> Option<NonZeroU32> {
        self.external_id
    }

    pub fn image_number(&self) -> Option<u32> {
        self.image_number
    }

    pub fn pixels(&self) -> &PixelBuffer {
        self.frame(self.current_frame).unwrap_or(&self.pixels)
    }

    fn frame(&self, index: usize) -> Option<&PixelBuffer> {
        if index == 0 {
            Some(&self.pixels)
        } else {
            self.frames.get(index - 1).map(|frame| &frame.pixels)
        }
    }

    pub fn content_generation(&self) -> u64 {
        self.content_generation
    }

    fn storage_bytes(&self) -> usize {
        self.frames.iter().fold(self.pixels.storage_bytes(), |total, frame| {
            total.saturating_add(frame.pixels.storage_bytes())
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StoreOutcome {
    pub handle: ImageHandle,
    pub image_id: Option<NonZeroU32>,
    pub image_number: Option<u32>,
}

pub(crate) enum FrameCanvas {
    Existing(PixelBuffer),
    Blank { width: u32, height: u32, rgba: u32 },
}

#[derive(Clone, Copy)]
pub(crate) enum FrameDestination {
    Insert { gap_ms: i32 },
    ReplaceRoot { gap_ms: i32 },
    Replace { index: usize, gap_ms: i32 },
    ComposeRoot,
    Compose { index: usize },
}

pub(crate) struct FrameWork {
    pub command: Command,
    image: ImageHandle,
    expected_revision: u64,
    canvas: FrameCanvas,
    source: PixelBuffer,
    composition: FrameComposition,
    destination: FrameDestination,
}

pub(crate) struct PreparedFrameMutation {
    pub command: Command,
    image: ImageHandle,
    expected_revision: u64,
    destination: FrameDestination,
    pixels: PixelBuffer,
}

#[derive(Debug)]
pub(crate) enum FrameCommit {
    Stored { frame_number: u32 },
    Composed,
}

impl FrameWork {
    pub(crate) fn process(self) -> Result<PreparedFrameMutation, (Box<Command>, GraphicsError)> {
        let destination = match self.canvas {
            FrameCanvas::Existing(pixels) => pixels,
            FrameCanvas::Blank { width, height, rgba } => match blank_frame(width, height, rgba) {
                Ok(pixels) => pixels,
                Err(error) => return Err((Box::new(self.command), error)),
            },
        };
        let pixels = match compose(destination, &self.source, self.composition) {
            Ok(pixels) => pixels,
            Err(error) => return Err((Box::new(self.command), error)),
        };
        Ok(PreparedFrameMutation {
            command: self.command,
            image: self.image,
            expected_revision: self.expected_revision,
            destination: self.destination,
            pixels,
        })
    }
}

#[derive(Clone, Debug)]
pub struct GraphicsState {
    images: HashMap<ImageHandle, Image>,
    image_ids: HashMap<NonZeroU32, ImageHandle>,
    placements: Placements,
    used_bytes: usize,
    frame_count: usize,
    storage_limit: usize,
    visible_lines: std::ops::Range<Line>,
    serial: u64,
}

impl GraphicsState {
    pub fn new(storage_limit: usize) -> Self {
        Self {
            storage_limit,
            images: Default::default(),
            image_ids: Default::default(),
            placements: Default::default(),
            used_bytes: 0,
            frame_count: 0,
            visible_lines: Line(0)..Line(i32::MAX),
            serial: 0,
        }
    }

    pub fn used_bytes(&self) -> usize {
        self.used_bytes
    }

    pub fn set_visible_lines(&mut self, screen_lines: usize) {
        self.visible_lines = Line(0)..Line(i32::try_from(screen_lines).unwrap_or(i32::MAX));
    }

    pub fn set_cell_dimensions(&mut self, cell_width: u16, cell_height: u16) {
        for placement in
            self.placements.values_mut().filter(|placement| !placement.virtual_placement)
        {
            let Some(image) = self.images.get(&placement.image) else { continue };
            let command = Command {
                x: Some(placement.source_x),
                y: Some(placement.source_y),
                crop_width: placement.source_width,
                crop_height: placement.source_height,
                x_offset: Some(placement.x_offset),
                y_offset: Some(placement.y_offset),
                columns: placement.columns,
                rows: placement.rows,
                ..Default::default()
            };
            // An inferred footprint may exceed the representable span after a font change.
            // Such a placement covers the terminal bounds and cannot fit a scroll margin.
            placement.cell_span =
                Self::placement_cell_span_for_image(image, &command, cell_width, cell_height)
                    .unwrap_or((u32::MAX, u32::MAX));
        }
    }

    pub fn decode_limit(&self, _command: &Command) -> usize {
        self.storage_limit
    }

    pub fn images(&self) -> impl Iterator<Item = &Image> {
        self.images.values()
    }

    pub fn image_by_id(&self, id: NonZeroU32) -> Option<&Image> {
        self.image_ids.get(&id).and_then(|handle| self.images.get(handle))
    }

    pub fn placements(&self) -> impl Iterator<Item = &Placement> {
        self.placements.values()
    }

    pub fn has_virtual_placements(&self) -> bool {
        self.placements.values().any(|placement| placement.virtual_placement)
    }

    pub fn has_classic_placements(&self) -> bool {
        self.placements.values().any(|placement| !placement.virtual_placement)
    }

    pub(crate) fn tracked_anchors(&self) -> Vec<(PlacementHandle, Point)> {
        self.placements.tracked_anchors()
    }

    pub(crate) fn update_tracked_anchors(&mut self, anchors: &HashMap<PlacementHandle, Point>) {
        let relative_images = self.placements.update_tracked_anchors(anchors);
        self.remove_orphaned_relative_images(relative_images);
    }

    pub fn renderables(&self) -> Vec<RenderableGraphic> {
        self.renderables_with_virtual_origins(|_| None)
    }

    pub fn renderables_with_virtual_origins(
        &self,
        virtual_origin: impl Fn(PlacementHandle) -> Option<Point>,
    ) -> Vec<RenderableGraphic> {
        let mut renderables: Vec<_> = self
            .placements
            .values()
            .filter(|placement| !placement.virtual_placement)
            .filter_map(|placement| {
                let image = self.images.get(&placement.image)?;
                let location = self.placements.resolved_anchor(placement, &virtual_origin)?;
                Some(RenderableGraphic {
                    image: image.handle,
                    pixels: image.pixels().clone(),
                    line: location.line,
                    column: location.column,
                    source_x: placement.source_x,
                    source_y: placement.source_y,
                    source_width: placement.source_width,
                    source_height: placement.source_height,
                    x_offset: placement.x_offset,
                    y_offset: placement.y_offset,
                    columns: placement.columns,
                    rows: placement.rows,
                    z_index: placement.z_index,
                    image_id: image.external_id.map_or(0, NonZeroU32::get),
                    content_generation: image.content_generation,
                    creation_serial: placement.creation_serial,
                    clip_region: location.clip_region,
                })
            })
            .collect();
        renderables
            .sort_by_key(|graphic| (graphic.z_index, graphic.image_id, graphic.creation_serial));
        renderables
    }

    pub(crate) fn virtual_prototypes(&self) -> HashMap<(u32, u32), PlacementHandle> {
        let mut prototypes = HashMap::new();
        for placement in self.placements.values().filter(|placement| placement.virtual_placement) {
            let Some(image_id) = placement.image_id.map(NonZeroU32::get) else {
                continue;
            };
            let placement_id = placement.placement_id.map_or(0, NonZeroU32::get);
            if placement_id != 0 {
                insert_oldest_prototype(&mut prototypes, (image_id, placement_id), placement);
            }
            insert_oldest_prototype(&mut prototypes, (image_id, 0), placement);
        }
        prototypes.into_iter().map(|(key, (_, handle))| (key, handle)).collect()
    }

    pub(crate) fn required_virtual_origins(&self) -> HashSet<PlacementHandle> {
        let mut required = HashSet::new();
        for placement in self.placements.values().filter(|placement| !placement.virtual_placement) {
            let mut root = placement;
            let mut depth = 0;
            while let Some(relative) = root.relative {
                depth += 1;
                if depth > crate::graphics::placement::MAX_RELATIVE_PLACEMENT_DEPTH {
                    break;
                }
                let Some(parent) = self.placements.get(relative.parent) else {
                    break;
                };
                root = parent;
            }
            if root.virtual_placement {
                required.insert(root.handle);
            }
        }
        required
    }

    pub(crate) fn virtual_placement_span(&self, handle: PlacementHandle) -> Option<(u32, u32)> {
        let placement = self.placements.get(handle)?;
        placement.columns.zip(placement.rows)
    }

    pub(crate) fn renderable_for_virtual(
        &self,
        handle: PlacementHandle,
    ) -> Option<RenderableGraphic> {
        let placement = self.placements.get(handle)?;
        if !placement.virtual_placement {
            return None;
        }
        let image = self.images.get(&placement.image)?;
        Some(RenderableGraphic {
            image: image.handle,
            pixels: image.pixels().clone(),
            line: placement.anchor.line,
            column: i32::try_from(placement.anchor.column.0).ok()?,
            source_x: placement.source_x,
            source_y: placement.source_y,
            source_width: placement.source_width,
            source_height: placement.source_height,
            x_offset: placement.x_offset,
            y_offset: placement.y_offset,
            columns: placement.columns,
            rows: placement.rows,
            z_index: placement.z_index,
            image_id: image.external_id.map_or(0, NonZeroU32::get),
            content_generation: image.content_generation,
            creation_serial: placement.creation_serial,
            clip_region: None,
        })
    }

    pub fn placeholder_renderable(
        &self,
        image_id: u32,
        placement_id: u32,
    ) -> Option<RenderableGraphic> {
        let image_id = NonZeroU32::new(image_id)?;
        let image = self.image_by_id(image_id)?;
        let placement_id = NonZeroU32::new(placement_id);
        let placement = self
            .placements
            .values()
            .filter(|placement| {
                placement.virtual_placement
                    && placement.image == image.handle
                    && placement_id.is_none_or(|id| placement.placement_id == Some(id))
            })
            .min_by_key(|placement| placement.creation_serial)?;
        Some(RenderableGraphic {
            image: image.handle,
            pixels: image.pixels().clone(),
            line: placement.anchor.line,
            column: i32::try_from(placement.anchor.column.0).ok()?,
            source_x: placement.source_x,
            source_y: placement.source_y,
            source_width: placement.source_width,
            source_height: placement.source_height,
            x_offset: placement.x_offset,
            y_offset: placement.y_offset,
            columns: placement.columns,
            rows: placement.rows,
            z_index: placement.z_index,
            image_id: image.external_id.map_or(0, NonZeroU32::get),
            content_generation: image.content_generation,
            creation_serial: placement.creation_serial,
            clip_region: placement.clip_region,
        })
    }

    pub fn scroll_up(
        &mut self,
        region: &std::ops::Range<Line>,
        lines: usize,
        history_size: usize,
        whole_screen: bool,
    ) {
        let relative_images = self.placements.scroll_up(region, lines, history_size, whole_screen);
        self.remove_orphaned_relative_images(relative_images);
    }

    pub fn scroll_down(
        &mut self,
        region: &std::ops::Range<Line>,
        lines: usize,
        whole_screen: bool,
    ) {
        let relative_images = self.placements.scroll_down(region, lines, whole_screen);
        self.remove_orphaned_relative_images(relative_images);
    }

    #[cfg(test)]
    pub fn delete(
        &mut self,
        command: &Command,
        cursor: Point,
        visible_lines: std::ops::Range<Line>,
    ) -> Result<(), GraphicsError> {
        self.delete_with_virtual_origins(command, cursor, visible_lines, &HashMap::new())
    }

    pub(crate) fn delete_with_virtual_origins(
        &mut self,
        command: &Command,
        cursor: Point,
        visible_lines: std::ops::Range<Line>,
        origins: &HashMap<PlacementHandle, Point>,
    ) -> Result<(), GraphicsError> {
        let selector = command.delete.map_or(b'a', |selector| selector.0);
        let free_data = selector.is_ascii_uppercase();
        let selector = selector.to_ascii_lowercase();
        let placement_id = NonZeroU32::new(command.placement_id.unwrap_or(0));
        if selector == b'f' {
            return self.delete_frames(command, free_data);
        }

        let explicit_image = match selector {
            b'i' => NonZeroU32::new(command.image_id.unwrap_or(0))
                .and_then(|id| self.image_ids.get(&id).copied()),
            b'n' => command
                .image_number
                .and_then(|number| self.newest_by_number(number))
                .map(|image| image.handle),
            _ => None,
        };
        if matches!(selector, b'i' | b'n') && explicit_image.is_none() {
            return Ok(());
        }

        let resolved_anchors: HashMap<_, _> = self
            .placements
            .values()
            .filter_map(|placement| {
                self.placements
                    .resolved_anchor(placement, &|handle| origins.get(&handle).copied())
                    .map(|location| (placement.handle, location))
            })
            .collect();
        let cell = (
            i64::from(command.x.unwrap_or(1).saturating_sub(1)),
            i64::from(command.y.unwrap_or(1).saturating_sub(1)),
        );
        let (affected, relative_images) = self.placements.remove_matching(|placement| {
            if placement.virtual_placement && !matches!(selector, b'i' | b'n' | b'r') {
                return false;
            }
            let bounds = resolved_anchors.get(&placement.handle).map(|location| {
                let left = i64::from(location.column);
                let top = i64::from(location.line.0);
                let bottom = top + i64::from(placement.cell_span.1);
                let (clip_top, clip_bottom) =
                    location.clip_region.map_or((i64::MIN, i64::MAX), |(start, end)| {
                        (i64::from(start.0), i64::from(end.0))
                    });
                (
                    left,
                    top.max(clip_top),
                    left + i64::from(placement.cell_span.0),
                    bottom.min(clip_bottom),
                )
            });
            let intersects = |(x, y): (i64, i64)| {
                bounds.is_some_and(|(left, top, right, bottom)| {
                    x >= left && x < right && y >= top && y < bottom
                })
            };
            match selector {
                b'a' => bounds.is_some_and(|(_, top, _, bottom)| {
                    top < bottom
                        && top < i64::from(visible_lines.end.0)
                        && bottom > i64::from(visible_lines.start.0)
                }),
                b'i' | b'n' => {
                    Some(placement.image) == explicit_image
                        && placement_id.is_none_or(|id| placement.placement_id == Some(id))
                },
                b'c' => intersects((
                    i64::try_from(cursor.column.0).unwrap_or(i64::MAX),
                    i64::from(cursor.line.0),
                )),
                b'p' => intersects(cell),
                b'q' => intersects(cell) && placement.z_index == command.z_index.unwrap_or(0),
                b'x' => bounds.is_some_and(|(left, top, right, bottom)| {
                    top < bottom && cell.0 >= left && cell.0 < right
                }),
                b'y' => bounds.is_some_and(|(_, top, _, bottom)| cell.1 >= top && cell.1 < bottom),
                b'z' => placement.z_index == command.z_index.unwrap_or(0),
                b'r' => placement.image_id.is_some_and(|id| {
                    id.get() >= command.x.unwrap_or(0) && id.get() <= command.y.unwrap_or(0)
                }),
                _ => false,
            }
        });

        self.remove_orphaned_relative_images(relative_images);

        if free_data {
            if let Some(image) = explicit_image {
                if placement_id.is_none() || !self.placements.image_is_placed(image) {
                    self.remove(image);
                }
            }
            for image in affected {
                if !self.placements.image_is_placed(image) {
                    self.remove(image);
                }
            }
            if selector == b'r' {
                let ids: Vec<_> = self
                    .image_ids
                    .iter()
                    .filter(|(id, _)| {
                        id.get() >= command.x.unwrap_or(0) && id.get() <= command.y.unwrap_or(0)
                    })
                    .map(|(_, handle)| *handle)
                    .collect();
                for handle in ids {
                    self.remove(handle);
                }
            }
        }
        Ok(())
    }

    pub fn placement_cell_span(
        &self,
        command: &Command,
        cell_width: u16,
        cell_height: u16,
    ) -> Result<(u32, u32), GraphicsError> {
        let handle = self.command_image_handle(command)?;
        let image = self.images.get(&handle).ok_or(GraphicsError::NotFound)?;
        Self::placement_cell_span_for_image(image, command, cell_width, cell_height)
    }

    fn placement_cell_span_for_image(
        image: &Image,
        command: &Command,
        cell_width: u16,
        cell_height: u16,
    ) -> Result<(u32, u32), GraphicsError> {
        let cell_width = cell_width.max(1);
        let cell_height = cell_height.max(1);
        let image_width = image.pixels.width();
        let image_height = image.pixels.height();
        let source_x = command.x.unwrap_or(0).min(image_width);
        let source_y = command.y.unwrap_or(0).min(image_height);
        let source_width = command
            .crop_width
            .filter(|width| *width != 0)
            .unwrap_or(image_width - source_x)
            .min(image_width - source_x);
        let source_height = command
            .crop_height
            .filter(|height| *height != 0)
            .unwrap_or(image_height - source_y)
            .min(image_height - source_y);
        if source_width == 0 || source_height == 0 {
            return Err(GraphicsError::Invalid);
        }
        let columns = command.columns.filter(|columns| *columns != 0);
        let rows = command.rows.filter(|rows| *rows != 0);
        let x_offset = command.x_offset.unwrap_or(0).min(u32::from(cell_width - 1));
        let y_offset = command.y_offset.unwrap_or(0).min(u32::from(cell_height - 1));
        match (columns, rows) {
            (Some(columns), Some(rows)) => Ok((columns, rows)),
            (Some(columns), None) => Ok((
                columns,
                scaled_cells(
                    columns,
                    cell_width,
                    x_offset,
                    source_height,
                    source_width,
                    cell_height,
                )?,
            )),
            (None, Some(rows)) => Ok((
                scaled_cells(rows, cell_height, y_offset, source_width, source_height, cell_width)?,
                rows,
            )),
            (None, None) => Ok((
                divide_ceil(
                    source_width.checked_add(x_offset).ok_or(GraphicsError::TooLarge)?,
                    u32::from(cell_width),
                )?,
                divide_ceil(
                    source_height.checked_add(y_offset).ok_or(GraphicsError::TooLarge)?,
                    u32::from(cell_height),
                )?,
            )),
        }
    }

    pub fn place(
        &mut self,
        command: &Command,
        anchor: Point,
    ) -> Result<PlacementHandle, GraphicsError> {
        let image = match NonZeroU32::new(command.image_id.unwrap_or(0)) {
            Some(id) => self.image_by_id(id),
            None => command.image_number.and_then(|number| self.newest_by_number(number)),
        }
        .ok_or(GraphicsError::NotFound)?;
        let handle = image.handle;
        let image_id = image.external_id;
        let span = self.placement_cell_span(command, 1, 1)?;
        let inserted = self.insert_placement(handle, image_id, command, anchor, span)?;
        self.finish_placement_insert(inserted)
    }

    pub fn place_handle(
        &mut self,
        handle: ImageHandle,
        command: &Command,
        anchor: Point,
    ) -> Result<PlacementHandle, GraphicsError> {
        let image_id = self.images.get(&handle).ok_or(GraphicsError::NotFound)?.external_id;
        let span = self.placement_cell_span(command, 1, 1)?;
        let inserted = self.insert_placement(handle, image_id, command, anchor, span)?;
        self.finish_placement_insert(inserted)
    }

    pub fn place_with_span(
        &mut self,
        command: &Command,
        anchor: Point,
        span: (u32, u32),
    ) -> Result<PlacementHandle, GraphicsError> {
        let handle = self.command_image_handle(command)?;
        let image_id = self.images.get(&handle).ok_or(GraphicsError::NotFound)?.external_id;
        let inserted = self.insert_placement(handle, image_id, command, anchor, span)?;
        self.finish_placement_insert(inserted)
    }

    fn insert_placement(
        &mut self,
        handle: ImageHandle,
        image_id: Option<NonZeroU32>,
        command: &Command,
        anchor: Point,
        span: (u32, u32),
    ) -> Result<PlacementInsert, GraphicsError> {
        let serial = self.serial.checked_add(1).ok_or(GraphicsError::TooLarge)?;
        if !self.images.contains_key(&handle) {
            return Err(GraphicsError::NotFound);
        }
        let placement = PlacementSpec {
            placement_id: image_id.and(command.placement_id),
            source_x: command.x.unwrap_or(0),
            source_y: command.y.unwrap_or(0),
            source_width: command.crop_width,
            source_height: command.crop_height,
            x_offset: command.x_offset.unwrap_or(0),
            y_offset: command.y_offset.unwrap_or(0),
            columns: if command.unicode_placeholder.unwrap_or(0) != 0 {
                Some(span.0)
            } else {
                command.columns
            },
            rows: if command.unicode_placeholder.unwrap_or(0) != 0 {
                Some(span.1)
            } else {
                command.rows
            },
            cell_span: span,
            z_index: command.z_index.unwrap_or(0),
            virtual_placement: command.unicode_placeholder.unwrap_or(0) != 0,
            parent_image_id: command.parent_image_id,
            parent_placement_id: command.parent_placement_id,
            horizontal_cells: command.horizontal_offset.unwrap_or(0),
            vertical_cells: command.vertical_offset.unwrap_or(0),
        };
        let inserted = self.placements.insert(handle, image_id, placement, anchor)?;
        self.serial = serial;
        self.images.get_mut(&handle).ok_or(GraphicsError::NotFound)?.last_used_serial = serial;
        Ok(inserted)
    }

    fn finish_placement_insert(
        &mut self,
        inserted: PlacementInsert,
    ) -> Result<PlacementHandle, GraphicsError> {
        self.remove_orphaned_relative_images(inserted.orphaned_relative_images);
        Ok(inserted.handle)
    }

    fn remove_orphaned_relative_images(&mut self, images: Vec<ImageHandle>) {
        for image in images {
            if !self.placements.image_is_placed(image) {
                self.remove(image);
            }
        }
    }

    fn delete_frames(&mut self, command: &Command, free_data: bool) -> Result<(), GraphicsError> {
        let handle = self.command_image_handle(command)?;
        let image = self.images.get(&handle).ok_or(GraphicsError::NotFound)?;
        if image.frames.is_empty() {
            if free_data {
                self.remove(handle);
            }
            return Ok(());
        }

        let frame_count = image.frames.len() + 1;
        let frame = command.rows.unwrap_or(1).max(1) as usize;
        let index = frame.saturating_sub(1).min(frame_count - 1);
        let revision = image.frame_revision.checked_add(1).ok_or(GraphicsError::TooLarge)?;
        let image = self.images.get_mut(&handle).ok_or(GraphicsError::NotFound)?;
        let removed_bytes = if index == 0 {
            let promoted = image.frames.remove(0);
            let bytes = image.pixels.storage_bytes();
            image.pixels = promoted.pixels;
            image.root_gap_ms = promoted.gap_ms;
            image.current_frame = image.current_frame.saturating_sub(1);
            bytes
        } else {
            let bytes = image.frames.remove(index - 1).pixels.storage_bytes();
            if image.current_frame > index {
                image.current_frame -= 1;
            } else if image.current_frame == index {
                image.current_frame = index.min(image.frames.len());
            }
            bytes
        };
        image.animation_state = AnimationState::Stopped;
        image.next_frame_at = None;
        self.used_bytes = self.used_bytes.saturating_sub(removed_bytes);
        self.frame_count = self.frame_count.saturating_sub(1);
        self.serial = self.serial.checked_add(1).ok_or(GraphicsError::TooLarge)?;
        image.content_generation = self.serial;
        image.frame_revision = revision;
        Ok(())
    }

    pub(crate) fn prepare_store_frame(
        &self,
        command: Command,
        source: PixelBuffer,
    ) -> Result<FrameWork, GraphicsError> {
        let handle = self.command_image_handle(&command)?;
        let image = self.images.get(&handle).ok_or(GraphicsError::NotFound)?;
        let edit_index =
            command.rows.and_then(|frame| frame.checked_sub(1)).map(|frame| frame as usize);
        let base_index =
            command.columns.and_then(|frame| frame.checked_sub(1)).map(|frame| frame as usize);
        let canvas = match edit_index {
            Some(index) => {
                FrameCanvas::Existing(image.frame(index).cloned().ok_or(GraphicsError::NotFound)?)
            },
            None => match base_index {
                Some(index) => FrameCanvas::Existing(
                    image.frame(index).cloned().ok_or(GraphicsError::NotFound)?,
                ),
                None => FrameCanvas::Blank {
                    width: image.pixels.width(),
                    height: image.pixels.height(),
                    rgba: command.y_offset.unwrap_or(0),
                },
            },
        };
        if edit_index.is_none()
            && (image.frames.len() == MAX_FRAMES_PER_IMAGE
                || self.frame_count == MAX_FRAMES_PER_BUFFER)
        {
            return Err(GraphicsError::NoSpace);
        }
        let existing_gap = edit_index.map(|index| {
            if index == 0 { image.root_gap_ms } else { image.frames[index - 1].gap_ms }
        });
        let gap_ms = command
            .z_index
            .filter(|gap| *gap != 0)
            .or(existing_gap)
            .unwrap_or(DEFAULT_FRAME_GAP_MS);
        let destination = match edit_index {
            None => FrameDestination::Insert { gap_ms },
            Some(0) => FrameDestination::ReplaceRoot { gap_ms },
            Some(index) => FrameDestination::Replace { index, gap_ms },
        };
        let composition = FrameComposition {
            source_x: 0,
            source_y: 0,
            destination_x: command.x.unwrap_or(0),
            destination_y: command.y.unwrap_or(0),
            width: source.width(),
            height: source.height(),
            overwrite: command.x_offset == Some(1),
        };
        Ok(FrameWork {
            command,
            image: handle,
            expected_revision: image.frame_revision,
            canvas,
            source,
            composition,
            destination,
        })
    }

    pub(crate) fn prepare_compose_frames(
        &self,
        command: Command,
    ) -> Result<FrameWork, GraphicsError> {
        let handle = self.command_image_handle(&command)?;
        let image = self.images.get(&handle).ok_or(GraphicsError::NotFound)?;
        let source_index =
            command.rows.and_then(|frame| frame.checked_sub(1)).ok_or(GraphicsError::Invalid)?
                as usize;
        let destination_index =
            command.columns.and_then(|frame| frame.checked_sub(1)).ok_or(GraphicsError::Invalid)?
                as usize;
        let source = image.frame(source_index).cloned().ok_or(GraphicsError::NotFound)?;
        let destination = image.frame(destination_index).cloned().ok_or(GraphicsError::NotFound)?;
        let source_x = command.x_offset.unwrap_or(0);
        let source_y = command.y_offset.unwrap_or(0);
        let width = command.crop_width.filter(|width| *width != 0).unwrap_or(image.pixels.width());
        let height =
            command.crop_height.filter(|height| *height != 0).unwrap_or(image.pixels.height());
        let destination_x = command.x.unwrap_or(0);
        let destination_y = command.y.unwrap_or(0);
        let overlaps = source_index == destination_index
            && source_x < destination_x.saturating_add(width)
            && destination_x < source_x.saturating_add(width)
            && source_y < destination_y.saturating_add(height)
            && destination_y < source_y.saturating_add(height);
        if overlaps {
            return Err(GraphicsError::Invalid);
        }
        let overwrite = command.cursor_policy == Some(1);
        let destination_kind = if destination_index == 0 {
            FrameDestination::ComposeRoot
        } else {
            FrameDestination::Compose { index: destination_index }
        };
        Ok(FrameWork {
            command,
            image: handle,
            expected_revision: image.frame_revision,
            canvas: FrameCanvas::Existing(destination),
            source,
            composition: FrameComposition {
                source_x,
                source_y,
                destination_x,
                destination_y,
                width,
                height,
                overwrite,
            },
            destination: destination_kind,
        })
    }

    pub(crate) fn commit_frame(
        &mut self,
        prepared: PreparedFrameMutation,
    ) -> Result<FrameCommit, GraphicsError> {
        let mut next = self.clone();
        let result = next.commit_frame_inner(prepared)?;
        *self = next;
        Ok(result)
    }

    fn commit_frame_inner(
        &mut self,
        prepared: PreparedFrameMutation,
    ) -> Result<FrameCommit, GraphicsError> {
        let image = self.images.get(&prepared.image).ok_or(GraphicsError::NotFound)?;
        if image.frame_revision != prepared.expected_revision {
            return Err(GraphicsError::Invalid);
        }
        let (old_bytes, visible_change) = match prepared.destination {
            FrameDestination::Insert { .. } => {
                if image.frames.len() == MAX_FRAMES_PER_IMAGE
                    || self.frame_count == MAX_FRAMES_PER_BUFFER
                {
                    return Err(GraphicsError::NoSpace);
                }
                (0, false)
            },
            FrameDestination::ReplaceRoot { .. } | FrameDestination::ComposeRoot => {
                (image.pixels.storage_bytes(), image.current_frame == 0)
            },
            FrameDestination::Replace { index, .. } | FrameDestination::Compose { index } => {
                let frame = image.frames.get(index - 1).ok_or(GraphicsError::NotFound)?;
                (frame.pixels.storage_bytes(), image.current_frame == index)
            },
        };
        let required = prepared.pixels.storage_bytes();
        let evictions = if matches!(
            prepared.destination,
            FrameDestination::Insert { .. }
                | FrameDestination::ReplaceRoot { .. }
                | FrameDestination::Replace { .. }
        ) {
            self.plan_evictions(required, old_bytes, Some(prepared.image))?
        } else {
            Vec::new()
        };
        let image = self.images.get(&prepared.image).ok_or(GraphicsError::NotFound)?;
        if image.frame_revision != prepared.expected_revision {
            return Err(GraphicsError::Invalid);
        }
        let mut projected = self.clone();
        for handle in &evictions {
            projected.remove(*handle);
        }
        if !projected.images.contains_key(&prepared.image) {
            return Err(GraphicsError::NotFound);
        }
        let target_usage = projected
            .used_bytes
            .checked_sub(old_bytes)
            .and_then(|used| used.checked_add(required))
            .ok_or(GraphicsError::TooLarge)?;
        if target_usage > self.storage_limit {
            return Err(GraphicsError::NoSpace);
        }
        let serial = self.serial.checked_add(1).ok_or(GraphicsError::TooLarge)?;
        let revision = image.frame_revision.checked_add(1).ok_or(GraphicsError::TooLarge)?;
        let frame_number = match prepared.destination {
            FrameDestination::Insert { .. } => u32::try_from(image.frames.len())
                .ok()
                .and_then(|count| count.checked_add(2))
                .ok_or(GraphicsError::TooLarge)?,
            FrameDestination::ReplaceRoot { .. } => 1,
            FrameDestination::Replace { index, .. } => u32::try_from(index)
                .ok()
                .and_then(|index| index.checked_add(1))
                .ok_or(GraphicsError::TooLarge)?,
            FrameDestination::ComposeRoot | FrameDestination::Compose { .. } => 0,
        };

        for handle in evictions {
            self.remove(handle);
        }
        let image = self.images.get_mut(&prepared.image).ok_or(GraphicsError::NotFound)?;
        match prepared.destination {
            FrameDestination::Insert { gap_ms } => {
                image.frames.push(AnimationFrame { pixels: prepared.pixels, gap_ms });
                self.frame_count += 1;
            },
            FrameDestination::ReplaceRoot { gap_ms } => {
                image.pixels = prepared.pixels;
                image.root_gap_ms = gap_ms;
            },
            FrameDestination::Replace { index, gap_ms } => {
                image.frames[index - 1] = AnimationFrame { pixels: prepared.pixels, gap_ms };
            },
            FrameDestination::ComposeRoot => image.pixels = prepared.pixels,
            FrameDestination::Compose { index } => image.frames[index - 1].pixels = prepared.pixels,
        }
        image.frame_revision = revision;
        if visible_change {
            image.content_generation = serial;
        }
        self.serial = serial;
        self.used_bytes = target_usage;
        Ok(if frame_number == 0 {
            FrameCommit::Composed
        } else {
            FrameCommit::Stored { frame_number }
        })
    }

    #[cfg(test)]
    pub fn store_frame(
        &mut self,
        command: &Command,
        pixels: PixelBuffer,
    ) -> Result<u32, GraphicsError> {
        let prepared = self
            .prepare_store_frame(command.clone(), pixels)?
            .process()
            .map_err(|(_, error)| error)?;
        match self.commit_frame(prepared)? {
            FrameCommit::Stored { frame_number } => Ok(frame_number),
            FrameCommit::Composed => Err(GraphicsError::Invalid),
        }
    }

    #[cfg(test)]
    pub fn compose_frames(&mut self, command: &Command) -> Result<(), GraphicsError> {
        let prepared =
            self.prepare_compose_frames(command.clone())?.process().map_err(|(_, error)| error)?;
        self.commit_frame(prepared).map(|_| ())
    }

    pub fn control_animation(&mut self, command: &Command) -> Result<(), GraphicsError> {
        let handle = self.command_image_handle(command)?;
        let image = self.images.get_mut(&handle).ok_or(GraphicsError::NotFound)?;
        if let (Some(frame), Some(gap)) = (
            command.rows.and_then(|frame| frame.checked_sub(1)),
            command.z_index.filter(|gap| *gap != 0),
        ) {
            if frame == 0 {
                image.root_gap_ms = gap;
            } else if let Some(frame) = image.frames.get_mut(frame as usize - 1) {
                frame.gap_ms = gap;
            }
        }
        if let Some(frame) = command.columns.and_then(|frame| frame.checked_sub(1)) {
            if image.frame(frame as usize).is_some() {
                image.current_frame = frame as usize;
                self.serial = self.serial.checked_add(1).ok_or(GraphicsError::TooLarge)?;
                image.content_generation = self.serial;
            }
        }
        match command.width.unwrap_or(0) {
            1 => {
                image.animation_state = AnimationState::Stopped;
                image.completed_loops = 0;
                image.next_frame_at = None;
            },
            2 => {
                image.animation_state = AnimationState::Loading;
                image.next_frame_at = None;
            },
            3 => {
                image.animation_state = AnimationState::Running;
                image.next_frame_at = None;
            },
            _ => (),
        }
        if command.height.unwrap_or(0) != 0 {
            image.loops = command.height.unwrap_or(1);
        }
        Ok(())
    }

    pub fn advance_animations(&mut self, now: Instant) -> (Option<Duration>, bool) {
        if self.images.is_empty() {
            return (None, false);
        }
        let placed: Vec<_> = self
            .images
            .keys()
            .copied()
            .filter(|handle| self.placements.image_is_placed(*handle))
            .collect();
        let mut next_deadline = None;
        let mut changed = false;
        for handle in placed {
            let Some(image) = self.images.get_mut(&handle) else {
                continue;
            };
            if image.animation_state == AnimationState::Stopped || image.frames.is_empty() {
                image.next_frame_at = None;
                continue;
            }
            if image.next_frame_at.is_none() {
                let gap = frame_gap_ms(image, image.current_frame);
                image.next_frame_at = if gap < 0 {
                    Some(now)
                } else {
                    Some(now + Duration::from_millis(gap.max(1) as u64))
                };
            }
            while image.next_frame_at.is_some_and(|deadline| deadline <= now) {
                match next_automatic_frame(image) {
                    AutomaticFrame::Display { index, wraps } => {
                        image.completed_loops = image.completed_loops.saturating_add(wraps);
                        if image.current_frame != index {
                            image.current_frame = index;
                            changed = true;
                            #[cfg(debug_assertions)]
                            if let Some(path) =
                                std::env::var_os("ALACRITTY_TEST_ANIMATION_STATE_FILE")
                            {
                                let _ = std::fs::write(path, index.to_string());
                            }
                            self.serial = self.serial.saturating_add(1);
                            image.content_generation = self.serial;
                        }
                        let gap = frame_gap_ms(image, index);
                        image.next_frame_at = Some(now + Duration::from_millis(gap.max(1) as u64));
                    },
                    AutomaticFrame::Stop { wraps } => {
                        image.completed_loops = image.completed_loops.saturating_add(wraps);
                        image.animation_state = AnimationState::Stopped;
                        image.next_frame_at = None;
                        break;
                    },
                    AutomaticFrame::Park | AutomaticFrame::Idle => {
                        image.next_frame_at = None;
                        break;
                    },
                }
            }
            if let Some(deadline) = image.next_frame_at {
                next_deadline =
                    Some(next_deadline.map_or(deadline, |current: Instant| current.min(deadline)));
            }
        }
        (next_deadline.map(|deadline| deadline.saturating_duration_since(now)), changed)
    }

    fn command_image_handle(&self, command: &Command) -> Result<ImageHandle, GraphicsError> {
        match NonZeroU32::new(command.image_id.unwrap_or(0)) {
            Some(id) => self.image_ids.get(&id).copied(),
            None => command
                .image_number
                .and_then(|number| self.newest_by_number(number))
                .map(|image| image.handle),
        }
        .ok_or(GraphicsError::NotFound)
    }

    pub fn newest_by_number(&self, number: u32) -> Option<&Image> {
        self.images
            .values()
            .filter(|image| image.image_number == Some(number))
            .max_by_key(|image| image.creation_serial)
    }

    pub fn clear(&mut self) {
        self.images.clear();
        self.image_ids.clear();
        self.placements.clear();
        self.used_bytes = 0;
        self.frame_count = 0;
    }

    pub fn set_storage_limit(&mut self, storage_limit: usize) {
        self.storage_limit = storage_limit;
        self.evict_until(0);
    }

    pub fn store_and_place(
        &mut self,
        command: &Command,
        pixels: PixelBuffer,
        anchor: Point,
        cell_width: u16,
        cell_height: u16,
    ) -> Result<(StoreOutcome, (u32, u32)), GraphicsError> {
        let mut transaction = self.clone();
        let outcome = transaction.store_inner(command, pixels)?;
        let image = transaction.images.get(&outcome.handle).ok_or(GraphicsError::NotFound)?;
        let span = Self::placement_cell_span_for_image(image, command, cell_width, cell_height)?;
        let inserted = transaction.insert_placement(
            outcome.handle,
            outcome.image_id,
            command,
            anchor,
            span,
        )?;
        transaction.finish_placement_insert(inserted)?;
        *self = transaction;
        Ok((outcome, span))
    }

    pub fn store(
        &mut self,
        command: &Command,
        pixels: PixelBuffer,
    ) -> Result<StoreOutcome, GraphicsError> {
        if command.action == Some(Action::Query) {
            return self.store_inner(command, pixels);
        }
        let mut transaction = self.clone();
        let outcome = transaction.store_inner(command, pixels)?;
        *self = transaction;
        Ok(outcome)
    }

    fn store_inner(
        &mut self,
        command: &Command,
        pixels: PixelBuffer,
    ) -> Result<StoreOutcome, GraphicsError> {
        if command.image_id.unwrap_or(0) != 0 && command.image_number.unwrap_or(0) != 0 {
            return Err(GraphicsError::Invalid);
        }
        let image_number = command.image_number.filter(|number| *number != 0);
        if command.action == Some(Action::Query) {
            return Ok(StoreOutcome {
                handle: ImageHandle(0),
                image_id: NonZeroU32::new(command.image_id.unwrap_or(0)),
                image_number,
            });
        }

        let external_id = match (NonZeroU32::new(command.image_id.unwrap_or(0)), image_number) {
            (Some(id), _) => Some(id),
            (None, Some(_)) => Some(self.allocate_image_id()?),
            (None, None) => None,
        };
        let replaced = external_id.and_then(|id| self.image_ids.get(&id).copied());
        let replaced_bytes =
            replaced.and_then(|handle| self.images.get(&handle)).map_or(0, Image::storage_bytes);
        let required = pixels.storage_bytes();
        if required > self.storage_limit {
            return Err(GraphicsError::NoSpace);
        }
        if replaced.is_none() && self.images.len() == MAX_IMAGES_PER_BUFFER {
            return Err(GraphicsError::NoSpace);
        }

        self.evict_until_with_exclusion(required, replaced_bytes, replaced);
        // Evicting a parent can reclaim the replaced image through its relative placements.
        let replaced_bytes =
            replaced.and_then(|handle| self.images.get(&handle)).map_or(0, Image::storage_bytes);
        let target_usage = self
            .used_bytes
            .checked_sub(replaced_bytes)
            .and_then(|used| used.checked_add(required))
            .ok_or(GraphicsError::TooLarge)?;
        if target_usage > self.storage_limit {
            return Err(GraphicsError::NoSpace);
        }

        if let Some(handle) = replaced {
            self.remove(handle);
        }

        let handle = ImageHandle::allocate()?;
        self.serial = self.serial.checked_add(1).ok_or(GraphicsError::TooLarge)?;
        let image = Image {
            handle,
            external_id,
            image_number,
            pixels,
            frames: Vec::new(),
            current_frame: 0,
            root_gap_ms: 0,
            animation_state: AnimationState::Stopped,
            loops: 1,
            completed_loops: 0,
            next_frame_at: None,
            content_generation: self.serial,
            frame_revision: 0,
            creation_serial: self.serial,
            last_used_serial: self.serial,
            transient: command.usage.is_some_and(|usage| usage & 1 != 0),
        };
        self.used_bytes = self
            .used_bytes
            .checked_add(image.pixels.storage_bytes())
            .ok_or(GraphicsError::TooLarge)?;
        self.images.insert(handle, image);
        if let Some(id) = external_id {
            self.image_ids.insert(id, handle);
        }

        Ok(StoreOutcome { handle, image_id: external_id, image_number })
    }

    fn allocate_image_id(&self) -> Result<NonZeroU32, GraphicsError> {
        (1..=u32::MAX)
            .filter_map(NonZeroU32::new)
            .find(|candidate| !self.image_ids.contains_key(candidate))
            .ok_or(GraphicsError::NoSpace)
    }

    fn evict_until(&mut self, incoming: usize) {
        self.evict_until_with_exclusion(incoming, 0, None);
    }

    fn plan_evictions(
        &self,
        incoming: usize,
        credit: usize,
        excluded: Option<ImageHandle>,
    ) -> Result<Vec<ImageHandle>, GraphicsError> {
        let mut simulation = self.clone();
        let mut evictions = Vec::new();
        while simulation.used_bytes.saturating_sub(credit).saturating_add(incoming)
            > simulation.storage_limit
        {
            let candidate = simulation
                .images
                .values()
                .filter(|image| Some(image.handle) != excluded)
                .min_by_key(|image| {
                    let placed = simulation.placements.image_is_placed(image.handle);
                    let visible = placed
                        && simulation
                            .placements
                            .image_is_visible(image.handle, &simulation.visible_lines);
                    let priority = match (placed, visible, image.transient) {
                        (false, _, true) => 0,
                        (false, _, false) => 1,
                        (true, false, true) => 2,
                        (true, false, false) => 3,
                        (true, true, _) => 4,
                    };
                    (priority, image.last_used_serial, image.creation_serial, image.handle.0)
                })
                .map(|image| image.handle)
                .ok_or(GraphicsError::NoSpace)?;
            evictions.push(candidate);
            simulation.remove(candidate);
            if excluded.is_some_and(|handle| !simulation.images.contains_key(&handle)) {
                return Err(GraphicsError::NotFound);
            }
        }
        Ok(evictions)
    }

    fn evict_until_with_exclusion(
        &mut self,
        incoming: usize,
        credit: usize,
        excluded: Option<ImageHandle>,
    ) {
        while self.used_bytes.saturating_sub(credit).saturating_add(incoming) > self.storage_limit {
            let candidate = self
                .images
                .values()
                .filter(|image| Some(image.handle) != excluded)
                .min_by_key(|image| {
                    let placed = self.placements.image_is_placed(image.handle);
                    let visible = placed
                        && self.placements.image_is_visible(image.handle, &self.visible_lines);
                    let priority = match (placed, visible, image.transient) {
                        (false, _, true) => 0,
                        (false, _, false) => 1,
                        (true, false, true) => 2,
                        (true, false, false) => 3,
                        (true, true, _) => 4,
                    };
                    (priority, image.last_used_serial, image.creation_serial, image.handle.0)
                })
                .map(|image| image.handle);
            match candidate {
                Some(handle) => self.remove(handle),
                None => break,
            }
        }
    }

    fn remove(&mut self, handle: ImageHandle) {
        if let Some(image) = self.images.remove(&handle) {
            let relative_images = self.placements.remove_image(handle);
            self.used_bytes -= image.storage_bytes();
            self.frame_count = self.frame_count.saturating_sub(image.frames.len());
            if let Some(id) = image.external_id {
                self.image_ids.remove(&id);
            }
            for relative_image in relative_images {
                if !self.placements.image_is_placed(relative_image) {
                    self.remove(relative_image);
                }
            }
        }
    }
}

enum AutomaticFrame {
    Display { index: usize, wraps: u32 },
    Stop { wraps: u32 },
    Park,
    Idle,
}

fn frame_gap_ms(image: &Image, index: usize) -> i32 {
    if index == 0 { image.root_gap_ms } else { image.frames[index - 1].gap_ms }
}

fn next_automatic_frame(image: &Image) -> AutomaticFrame {
    let last = image.frames.len();
    let mut index = image.current_frame;
    let mut wraps: u32 = 0;
    for _ in 0..=last {
        if image.animation_state == AnimationState::Loading && index == last {
            return AutomaticFrame::Park;
        }
        index += 1;
        if index > last {
            if image.animation_state == AnimationState::Loading {
                return AutomaticFrame::Park;
            }
            wraps = wraps.saturating_add(1);
            let completed = image.completed_loops.saturating_add(wraps);
            if image.loops > 1 && completed >= image.loops - 1 {
                return AutomaticFrame::Stop { wraps };
            }
            index = 0;
        }
        if frame_gap_ms(image, index) >= 0 {
            return AutomaticFrame::Display { index, wraps };
        }
    }
    AutomaticFrame::Idle
}

fn insert_oldest_prototype(
    prototypes: &mut HashMap<(u32, u32), (u64, PlacementHandle)>,
    key: (u32, u32),
    placement: &Placement,
) {
    let candidate = (placement.creation_serial, placement.handle);
    match prototypes.get_mut(&key) {
        Some(current) if candidate.0 < current.0 => *current = candidate,
        None => {
            prototypes.insert(key, candidate);
        },
        _ => (),
    }
}

fn scaled_cells(
    cells: u32,
    cell_pixels: u16,
    pixel_offset: u32,
    source_numerator: u32,
    source_denominator: u32,
    other_cell_pixels: u16,
) -> Result<u32, GraphicsError> {
    let numerator = u128::from(cells)
        .checked_mul(u128::from(cell_pixels))
        .and_then(|value| value.checked_add(u128::from(pixel_offset)))
        .and_then(|value| value.checked_mul(u128::from(source_numerator)))
        .ok_or(GraphicsError::TooLarge)?;
    let denominator = u128::from(source_denominator)
        .checked_mul(u128::from(other_cell_pixels))
        .ok_or(GraphicsError::TooLarge)?;
    let result = numerator
        .checked_add(denominator - 1)
        .and_then(|value| value.checked_div(denominator))
        .ok_or(GraphicsError::TooLarge)?;
    u32::try_from(result.max(1)).map_err(|_| GraphicsError::TooLarge)
}

fn divide_ceil(numerator: u32, denominator: u32) -> Result<u32, GraphicsError> {
    numerator
        .checked_add(denominator - 1)
        .and_then(|value| value.checked_div(denominator))
        .map(|value| value.max(1))
        .ok_or(GraphicsError::TooLarge)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn assert_invariants(state: &GraphicsState) {
        assert_eq!(
            state.used_bytes,
            state.images.values().map(Image::storage_bytes).sum::<usize>()
        );
        assert!(state.used_bytes <= state.storage_limit);
        assert_eq!(
            state.frame_count,
            state.images.values().map(|image| image.frames.len()).sum::<usize>()
        );
        assert!(
            state.placements.values().all(|placement| state.images.contains_key(&placement.image))
        );
        assert!(state.image_ids.iter().all(|(id, handle)| {
            state.images.get(handle).is_some_and(|image| image.external_id == Some(*id))
        }));
    }

    fn pixels(value: u8, bytes: usize) -> PixelBuffer {
        PixelBuffer::from_rgba(1, 1, Arc::from(vec![value; bytes]))
    }

    #[test]
    fn replacement_accounts_for_parent_eviction_and_preserves_state_on_failure() {
        let mut state = GraphicsState::new(12);
        let parent = Command { image_id: Some(1), placement_id: Some(1), ..Default::default() };
        let child = Command {
            image_id: Some(2),
            placement_id: Some(1),
            parent_image_id: Some(1),
            parent_placement_id: Some(1),
            ..Default::default()
        };
        state.store(&parent, pixels(1, 4)).unwrap();
        state.store(&child, pixels(2, 4)).unwrap();
        state.place(&parent, Point::default()).unwrap();
        state.place(&child, Point::default()).unwrap();
        let replacement = Command { image_id: Some(2), ..Default::default() };
        let large = PixelBuffer::from_rgba(3, 1, Arc::from(vec![3; 12]));
        let mut exhausted = state.clone();
        exhausted.serial = u64::MAX;
        assert_eq!(exhausted.store(&replacement, large.clone()), Err(GraphicsError::TooLarge));
        assert_eq!(exhausted.images.len(), 2);
        assert_eq!(exhausted.placements.values().count(), 2);
        assert_eq!(exhausted.used_bytes, 8);
        state.store(&replacement, large).unwrap();
        assert_eq!(state.images.len(), 1);
        assert_eq!(state.used_bytes, 12);
        assert_eq!(
            state.image_by_id(NonZeroU32::new(2).unwrap()).unwrap().pixels().bytes(),
            &[3; 12]
        );
        assert_invariants(&state);
    }

    #[test]
    fn bounds_animation_frame_metadata_per_buffer() {
        let mut state = GraphicsState::new(8);
        let image = Command { image_id: Some(1), ..Default::default() };
        state.store(&image, pixels(1, 4)).unwrap();
        state.frame_count = MAX_FRAMES_PER_BUFFER;

        assert_eq!(
            state.store_frame(
                &Command {
                    action: Some(Action::TransmitFrame),
                    image_id: Some(1),
                    ..Default::default()
                },
                pixels(2, 4),
            ),
            Err(GraphicsError::NoSpace)
        );
    }

    #[test]
    fn loads_edits_and_selects_animation_frames() {
        let mut state = GraphicsState::new(32);
        let image = Command { image_id: Some(1), ..Default::default() };
        state
            .store(
                &image,
                PixelBuffer::from_rgba(2, 1, Arc::from([255, 0, 0, 255, 255, 0, 0, 255])),
            )
            .unwrap();
        state
            .store_frame(
                &Command {
                    action: Some(Action::TransmitFrame),
                    image_id: Some(1),
                    x: Some(1),
                    x_offset: Some(1),
                    ..Default::default()
                },
                PixelBuffer::from_rgba(1, 1, Arc::from([0, 0, 255, 255])),
            )
            .unwrap();
        state
            .control_animation(&Command {
                action: Some(Action::Animate),
                image_id: Some(1),
                columns: Some(2),
                ..Default::default()
            })
            .unwrap();

        assert_eq!(state.image_by_id(NonZeroU32::new(1).unwrap()).unwrap().pixels().bytes(), &[
            0, 0, 0, 0, 0, 0, 255, 255
        ]);
        assert_eq!(state.used_bytes(), 16);

        state
            .store_frame(
                &Command {
                    action: Some(Action::TransmitFrame),
                    image_id: Some(1),
                    rows: Some(2),
                    x_offset: Some(1),
                    ..Default::default()
                },
                PixelBuffer::from_rgba(1, 1, Arc::from([0, 255, 0, 255])),
            )
            .unwrap();
        assert_eq!(state.image_by_id(NonZeroU32::new(1).unwrap()).unwrap().pixels().bytes(), &[
            0, 255, 0, 255, 0, 0, 255, 255
        ]);
        assert_eq!(state.used_bytes(), 16);
    }

    #[test]
    fn prepared_frame_commit_revalidates_revision_quota_and_eviction() {
        let image = Command { image_id: Some(1), ..Default::default() };
        let mut state = GraphicsState::new(12);
        state.store(&image, pixels(1, 4)).unwrap();
        state.store(&Command { image_id: Some(2), ..Default::default() }, pixels(2, 4)).unwrap();
        let stale =
            state.prepare_store_frame(image.clone(), pixels(3, 4)).unwrap().process().unwrap();
        state.store_frame(&image, pixels(4, 4)).unwrap();
        assert_eq!(state.commit_frame(stale).unwrap_err(), GraphicsError::Invalid);

        let mut state = GraphicsState::new(8);
        state.store(&image, pixels(1, 4)).unwrap();
        state.store(&Command { image_id: Some(2), ..Default::default() }, pixels(2, 4)).unwrap();
        let prepared =
            state.prepare_store_frame(image.clone(), pixels(5, 4)).unwrap().process().unwrap();
        state.commit_frame(prepared).unwrap();
        assert!(state.image_by_id(NonZeroU32::new(2).unwrap()).is_none());
        assert_eq!(state.image_by_id(NonZeroU32::new(1).unwrap()).unwrap().frames.len(), 1);

        let prepared =
            state.prepare_store_frame(image.clone(), pixels(6, 4)).unwrap().process().unwrap();
        state.storage_limit = 4;
        let before = state.clone();
        assert_eq!(state.commit_frame(prepared).unwrap_err(), GraphicsError::NoSpace);
        assert_eq!(state.used_bytes, before.used_bytes);
        assert_eq!(state.images[&state.command_image_handle(&image).unwrap()].frames.len(), 1);
    }

    #[test]
    fn compose_commit_never_evicts_and_generation_tracks_visible_pixels() {
        let image = Command { image_id: Some(1), ..Default::default() };
        let mut state = GraphicsState::new(12);
        state.store(&image, pixels(1, 4)).unwrap();
        let handle = state.command_image_handle(&image).unwrap();
        let generation = state.images[&handle].content_generation;
        state.store_frame(&image, pixels(2, 4)).unwrap();
        assert_eq!(state.images[&handle].content_generation, generation);

        state.store(&Command { image_id: Some(2), ..Default::default() }, pixels(3, 4)).unwrap();
        let prepared = state
            .prepare_compose_frames(Command {
                action: Some(Action::ComposeFrame),
                image_id: Some(1),
                rows: Some(2),
                columns: Some(1),
                cursor_policy: Some(1),
                ..Default::default()
            })
            .unwrap()
            .process()
            .unwrap();
        state.storage_limit = 8;
        assert_eq!(state.commit_frame(prepared).unwrap_err(), GraphicsError::NoSpace);
        assert!(state.image_by_id(NonZeroU32::new(2).unwrap()).is_some());
        assert_eq!(state.images[&handle].content_generation, generation);

        state.storage_limit = 12;
        let prepared = state
            .prepare_compose_frames(Command {
                action: Some(Action::ComposeFrame),
                image_id: Some(1),
                rows: Some(2),
                columns: Some(1),
                cursor_policy: Some(1),
                ..Default::default()
            })
            .unwrap()
            .process()
            .unwrap();
        state.commit_frame(prepared).unwrap();
        assert!(state.images[&handle].content_generation > generation);
    }

    #[test]
    fn frame_defaults_canvases_and_base_frames_follow_protocol() {
        let mut state = GraphicsState::new(32);
        let image = Command { image_id: Some(1), ..Default::default() };
        state
            .store(&image, PixelBuffer::from_rgba(2, 1, Arc::from([1, 1, 1, 255, 2, 2, 2, 255])))
            .unwrap();
        state
            .store_frame(
                &Command {
                    image_id: Some(1),
                    x: Some(1),
                    y_offset: Some(0xff0000ff),
                    x_offset: Some(1),
                    ..Default::default()
                },
                PixelBuffer::from_rgba(1, 1, Arc::from([0, 255, 0, 255])),
            )
            .unwrap();
        let handle = state.command_image_handle(&image).unwrap();
        assert_eq!(state.images[&handle].frames[0].gap_ms, DEFAULT_FRAME_GAP_MS);
        assert_eq!(state.images[&handle].frames[0].pixels.bytes(), &[
            255, 0, 0, 255, 0, 255, 0, 255
        ]);

        state
            .store_frame(
                &Command {
                    image_id: Some(1),
                    columns: Some(2),
                    x: Some(0),
                    x_offset: Some(1),
                    z_index: Some(0),
                    ..Default::default()
                },
                PixelBuffer::from_rgba(1, 1, Arc::from([0, 0, 255, 255])),
            )
            .unwrap();
        assert_eq!(state.images[&handle].frames[1].gap_ms, DEFAULT_FRAME_GAP_MS);
        assert_eq!(state.images[&handle].frames[1].pixels.bytes(), &[
            0, 0, 255, 255, 0, 255, 0, 255
        ]);
    }

    #[test]
    fn edits_root_and_preserves_existing_frame_gap() {
        let mut state = GraphicsState::new(32);
        let image = Command { image_id: Some(1), ..Default::default() };
        state.store(&image, pixels(1, 4)).unwrap();
        assert_eq!(
            state
                .store_frame(
                    &Command { image_id: Some(1), z_index: Some(77), ..Default::default() },
                    pixels(2, 4),
                )
                .unwrap(),
            2
        );
        assert_eq!(
            state
                .store_frame(
                    &Command {
                        image_id: Some(1),
                        rows: Some(2),
                        x_offset: Some(1),
                        ..Default::default()
                    },
                    pixels(3, 4),
                )
                .unwrap(),
            2
        );
        assert_eq!(
            state
                .store_frame(
                    &Command {
                        image_id: Some(1),
                        rows: Some(1),
                        x_offset: Some(1),
                        ..Default::default()
                    },
                    pixels(4, 4),
                )
                .unwrap(),
            1
        );

        let image = state.image_by_id(NonZeroU32::new(1).unwrap()).unwrap();
        assert_eq!(image.pixels.bytes(), &[4; 4]);
        assert_eq!(image.frames[0].pixels.bytes(), &[3; 4]);
        assert_eq!(image.frames[0].gap_ms, 77);
        assert_invariants(&state);
    }

    #[test]
    fn deleting_animation_root_promotes_next_frame_and_uppercase_frees_root_only_image() {
        let mut state = GraphicsState::new(32);
        let image = Command { image_id: Some(1), ..Default::default() };
        state.store(&image, pixels(1, 4)).unwrap();
        state
            .store_frame(
                &Command { image_id: Some(1), z_index: Some(10), ..Default::default() },
                pixels(2, 4),
            )
            .unwrap();
        state
            .store_frame(
                &Command { image_id: Some(1), z_index: Some(20), ..Default::default() },
                pixels(3, 4),
            )
            .unwrap();

        state
            .delete(
                &Command {
                    action: Some(Action::Delete),
                    delete: Some(crate::graphics::DeleteTarget(b'f')),
                    image_id: Some(1),
                    ..Default::default()
                },
                Point::default(),
                Line(0)..Line(1),
            )
            .unwrap();
        let image = state.image_by_id(NonZeroU32::new(1).unwrap()).unwrap();
        assert_eq!(image.pixels.bytes(), &[2; 4]);
        assert_eq!(image.root_gap_ms, 10);
        assert_eq!(image.frames.len(), 1);

        state
            .delete(
                &Command {
                    action: Some(Action::Delete),
                    delete: Some(crate::graphics::DeleteTarget(b'f')),
                    image_id: Some(1),
                    rows: Some(99),
                    ..Default::default()
                },
                Point::default(),
                Line(0)..Line(1),
            )
            .unwrap();
        assert!(state.image_by_id(NonZeroU32::new(1).unwrap()).is_some());
        assert!(state.image_by_id(NonZeroU32::new(1).unwrap()).unwrap().frames.is_empty());
        state
            .place(&Command { image_id: Some(1), ..Default::default() }, Point::default())
            .unwrap();

        state
            .delete(
                &Command {
                    action: Some(Action::Delete),
                    delete: Some(crate::graphics::DeleteTarget(b'F')),
                    image_id: Some(1),
                    ..Default::default()
                },
                Point::default(),
                Line(0)..Line(1),
            )
            .unwrap();
        assert!(state.image_by_id(NonZeroU32::new(1).unwrap()).is_none());
        assert_eq!(state.placements().count(), 0);
        assert_invariants(&state);
    }

    #[test]
    fn composes_and_deletes_animation_frames() {
        let mut state = GraphicsState::new(16);
        let image = Command { image_id: Some(1), ..Default::default() };
        state.store(&image, pixels(1, 4)).unwrap();
        state
            .store_frame(
                &Command {
                    action: Some(Action::TransmitFrame),
                    image_id: Some(1),
                    ..Default::default()
                },
                pixels(2, 4),
            )
            .unwrap();
        state
            .compose_frames(&Command {
                action: Some(Action::ComposeFrame),
                image_id: Some(1),
                rows: Some(1),
                columns: Some(2),
                crop_width: Some(1),
                crop_height: Some(1),
                cursor_policy: Some(1),
                ..Default::default()
            })
            .unwrap();
        state
            .control_animation(&Command {
                image_id: Some(1),
                columns: Some(2),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(
            state.image_by_id(NonZeroU32::new(1).unwrap()).unwrap().pixels().bytes(),
            &[1; 4]
        );

        assert_eq!(
            state.compose_frames(&Command {
                action: Some(Action::ComposeFrame),
                image_id: Some(1),
                rows: Some(1),
                columns: Some(1),
                ..Default::default()
            }),
            Err(GraphicsError::Invalid)
        );
        assert_eq!(
            state.compose_frames(&Command {
                image_id: Some(1),
                rows: Some(9),
                columns: Some(1),
                ..Default::default()
            }),
            Err(GraphicsError::NotFound)
        );
        assert_eq!(
            state.compose_frames(&Command {
                image_id: Some(1),
                rows: Some(1),
                columns: Some(2),
                x: Some(2),
                crop_width: Some(1),
                crop_height: Some(1),
                ..Default::default()
            }),
            Err(GraphicsError::Invalid)
        );

        state
            .delete(
                &Command {
                    action: Some(Action::Delete),
                    delete: Some(crate::graphics::DeleteTarget(b'f')),
                    image_id: Some(1),
                    ..Default::default()
                },
                Point::default(),
                Line(0)..Line(1),
            )
            .unwrap();
        assert_eq!(state.used_bytes(), 4);
        assert_eq!(
            state.image_by_id(NonZeroU32::new(1).unwrap()).unwrap().pixels().bytes(),
            &[1; 4]
        );
    }

    #[test]
    fn animation_control_ignores_unknown_state_and_frame_values() {
        let mut state = GraphicsState::new(8);
        let image = Command { image_id: Some(1), ..Default::default() };
        state.store(&image, pixels(1, 4)).unwrap();
        state
            .store_frame(&Command { image_id: Some(1), ..Default::default() }, pixels(2, 4))
            .unwrap();

        state
            .control_animation(&Command {
                image_id: Some(1),
                rows: Some(9),
                columns: Some(9),
                width: Some(7),
                z_index: Some(50),
                ..Default::default()
            })
            .unwrap();
        let image = state.image_by_id(NonZeroU32::new(1).unwrap()).unwrap();
        assert_eq!(image.current_frame, 0);
        assert_eq!(image.root_gap_ms, 0);
        assert_eq!(image.animation_state, AnimationState::Stopped);
    }

    #[test]
    fn advances_running_animations_on_deadlines() {
        let mut state = GraphicsState::new(16);
        let image = Command { image_id: Some(1), ..Default::default() };
        state.store(&image, pixels(1, 4)).unwrap();
        state
            .store_frame(
                &Command {
                    action: Some(Action::TransmitFrame),
                    image_id: Some(1),
                    ..Default::default()
                },
                pixels(2, 4),
            )
            .unwrap();
        state.place(&image, Point::default()).unwrap();
        state
            .control_animation(&Command {
                action: Some(Action::Animate),
                image_id: Some(1),
                width: Some(3),
                rows: Some(1),
                z_index: Some(10),
                ..Default::default()
            })
            .unwrap();

        let start = Instant::now();
        assert_eq!(state.advance_animations(start), (Some(Duration::from_millis(10)), false));
        state.advance_animations(start + Duration::from_millis(10));
        assert_eq!(
            state.image_by_id(NonZeroU32::new(1).unwrap()).unwrap().pixels().bytes(),
            &[2; 4]
        );
    }

    #[test]
    fn loading_and_finite_loop_playback_follow_protocol() {
        let mut state = GraphicsState::new(8);
        let image = Command { image_id: Some(1), ..Default::default() };
        state.store(&image, pixels(1, 4)).unwrap();
        state
            .store_frame(
                &Command { image_id: Some(1), z_index: Some(1), ..Default::default() },
                pixels(2, 4),
            )
            .unwrap();
        state.place(&image, Point::default()).unwrap();
        state
            .control_animation(&Command {
                image_id: Some(1),
                width: Some(2),
                rows: Some(1),
                z_index: Some(1),
                ..Default::default()
            })
            .unwrap();
        let start = Instant::now();
        state.advance_animations(start);
        state.advance_animations(start + Duration::from_millis(1));
        state.advance_animations(start + Duration::from_millis(2));
        let handle = state.command_image_handle(&image).unwrap();
        assert_eq!(state.images[&handle].current_frame, 1);
        assert_eq!(state.images[&handle].animation_state, AnimationState::Loading);
        assert!(state.images[&handle].next_frame_at.is_none());

        state
            .control_animation(&Command {
                image_id: Some(1),
                width: Some(3),
                height: Some(2),
                ..Default::default()
            })
            .unwrap();
        state.advance_animations(start + Duration::from_millis(3));
        state.advance_animations(start + Duration::from_millis(43));
        assert_eq!(state.images[&handle].animation_state, AnimationState::Stopped);
        assert_eq!(state.images[&handle].completed_loops, 1);
    }

    #[test]
    fn loading_animation_resumes_when_a_frame_arrives() {
        let mut state = GraphicsState::new(12);
        let image = Command { image_id: Some(1), ..Default::default() };
        state.store(&image, pixels(1, 4)).unwrap();
        state
            .store_frame(
                &Command { image_id: Some(1), z_index: Some(1), ..Default::default() },
                pixels(2, 4),
            )
            .unwrap();
        state.place(&image, Point::default()).unwrap();
        state
            .control_animation(&Command {
                image_id: Some(1),
                width: Some(2),
                rows: Some(1),
                z_index: Some(1),
                ..Default::default()
            })
            .unwrap();
        let start = Instant::now();
        state.advance_animations(start);
        state.advance_animations(start + Duration::from_millis(1));
        state.advance_animations(start + Duration::from_millis(2));
        state
            .store_frame(
                &Command { image_id: Some(1), z_index: Some(1), ..Default::default() },
                pixels(3, 4),
            )
            .unwrap();
        state.advance_animations(start + Duration::from_millis(3));
        state.advance_animations(start + Duration::from_millis(4));
        let handle = state.command_image_handle(&image).unwrap();
        assert_eq!(state.images[&handle].current_frame, 2);
        assert_eq!(state.images[&handle].animation_state, AnimationState::Loading);
    }

    #[test]
    fn gapless_frames_are_skipped_without_a_display_deadline() {
        let mut state = GraphicsState::new(16);
        let image = Command { image_id: Some(1), ..Default::default() };
        state.store(&image, pixels(1, 4)).unwrap();
        state
            .store_frame(
                &Command { image_id: Some(1), z_index: Some(-1), ..Default::default() },
                pixels(2, 4),
            )
            .unwrap();
        state
            .store_frame(
                &Command { image_id: Some(1), z_index: Some(20), ..Default::default() },
                pixels(3, 4),
            )
            .unwrap();
        state.place(&image, Point::default()).unwrap();
        state
            .control_animation(&Command {
                image_id: Some(1),
                width: Some(3),
                rows: Some(1),
                z_index: Some(10),
                ..Default::default()
            })
            .unwrap();

        let start = Instant::now();
        state.advance_animations(start);
        state.advance_animations(start + Duration::from_millis(10));
        let image = state.image_by_id(NonZeroU32::new(1).unwrap()).unwrap();
        assert_eq!(image.current_frame, 2);
        assert_eq!(image.pixels().bytes(), &[3; 4]);
    }

    #[test]
    fn finite_animation_never_finishes_on_gapless_frame() {
        let mut state = GraphicsState::new(8);
        let command = Command { image_id: Some(1), ..Default::default() };
        state.store(&command, pixels(1, 4)).unwrap();
        state.place(&command, Point::default()).unwrap();
        state
            .store_frame(
                &Command { image_id: Some(1), z_index: Some(-1), ..Default::default() },
                pixels(2, 4),
            )
            .unwrap();
        state
            .control_animation(&Command {
                image_id: Some(1),
                rows: Some(1),
                z_index: Some(10),
                width: Some(3),
                height: Some(2),
                ..Default::default()
            })
            .unwrap();
        let now = Instant::now();
        state.advance_animations(now);
        state.advance_animations(now + Duration::from_millis(10));
        let image = state.image_by_id(NonZeroU32::new(1).unwrap()).unwrap();
        assert_eq!(image.pixels().bytes(), &[1; 4]);
        assert_eq!(image.current_frame, 0);
        assert_eq!(image.animation_state, AnimationState::Stopped);
    }

    #[test]
    fn initial_gapless_frame_is_skipped_when_animation_starts() {
        let mut state = GraphicsState::new(8);
        let image = Command { image_id: Some(1), ..Default::default() };
        state.store(&image, pixels(1, 4)).unwrap();
        state
            .store_frame(
                &Command { image_id: Some(1), z_index: Some(10), ..Default::default() },
                pixels(2, 4),
            )
            .unwrap();
        state.place(&image, Point::default()).unwrap();
        state
            .control_animation(&Command {
                image_id: Some(1),
                width: Some(3),
                rows: Some(1),
                z_index: Some(-1),
                ..Default::default()
            })
            .unwrap();
        let (deadline, changed) = state.advance_animations(Instant::now());
        let image = state.image_by_id(NonZeroU32::new(1).unwrap()).unwrap();
        assert!(changed);
        assert_eq!(image.current_frame, 1);
        assert_eq!(image.pixels().bytes(), &[2; 4]);
        assert_eq!(deadline, Some(Duration::from_millis(10)));
    }

    #[test]
    fn all_gapless_animation_does_not_publish_or_busy_poll() {
        let mut state = GraphicsState::new(8);
        let image = Command { image_id: Some(1), ..Default::default() };
        state.store(&image, pixels(1, 4)).unwrap();
        state
            .store_frame(
                &Command { image_id: Some(1), z_index: Some(-1), ..Default::default() },
                pixels(2, 4),
            )
            .unwrap();
        state.place(&image, Point::default()).unwrap();
        state
            .control_animation(&Command {
                image_id: Some(1),
                width: Some(3),
                rows: Some(1),
                z_index: Some(-1),
                ..Default::default()
            })
            .unwrap();
        let start = Instant::now();
        let handle = state.command_image_handle(&image).unwrap();
        let generation = state.images[&handle].content_generation;
        assert_eq!(state.advance_animations(start), (None, false));
        assert_eq!(state.images[&handle].current_frame, 0);
        assert_eq!(state.images[&handle].pixels().bytes(), &[1; 4]);
        assert_eq!(state.images[&handle].content_generation, generation);
        assert!(state.images[&handle].next_frame_at.is_none());
        assert_eq!(state.advance_animations(start + Duration::from_millis(1)), (None, false));
        assert_eq!(state.images[&handle].current_frame, 0);
        assert_eq!(state.images[&handle].content_generation, generation);
        assert!(state.images[&handle].next_frame_at.is_none());
    }

    #[test]
    fn stopping_animation_resets_completed_loop_count() {
        let mut state = GraphicsState::new(8);
        let image = Command { image_id: Some(1), ..Default::default() };
        state.store(&image, pixels(1, 4)).unwrap();
        state
            .store_frame(&Command { image_id: Some(1), ..Default::default() }, pixels(2, 4))
            .unwrap();
        let handle = state.command_image_handle(&image).unwrap();
        state.images.get_mut(&handle).unwrap().completed_loops = 3;

        state
            .control_animation(&Command { image_id: Some(1), width: Some(1), ..Default::default() })
            .unwrap();

        assert_eq!(state.images.get(&handle).unwrap().completed_loops, 0);
    }

    #[test]
    fn retransmitting_base_image_removes_animation_state() {
        let mut state = GraphicsState::new(12);
        let image = Command { image_id: Some(1), ..Default::default() };
        state.store(&image, pixels(1, 4)).unwrap();
        state
            .store_frame(&Command { image_id: Some(1), ..Default::default() }, pixels(2, 4))
            .unwrap();
        state
            .control_animation(&Command { image_id: Some(1), width: Some(3), ..Default::default() })
            .unwrap();

        state.store(&image, pixels(3, 4)).unwrap();

        let image = state.image_by_id(NonZeroU32::new(1).unwrap()).unwrap();
        assert!(image.frames.is_empty());
        assert_eq!(image.animation_state, AnimationState::Stopped);
        assert_eq!(image.pixels().bytes(), &[3; 4]);
        assert_eq!(state.frame_count, 0);
    }

    #[test]
    fn failed_transmit_and_place_preserves_replaced_image_and_placements() {
        let mut state = GraphicsState::new(8);
        let old = Command { image_id: Some(1), placement_id: Some(1), ..Default::default() };
        state.store(&old, pixels(1, 4)).unwrap();
        state.place(&old, Point::default()).unwrap();
        let replacement = Command {
            action: Some(Action::TransmitAndPlace),
            image_id: Some(1),
            placement_id: Some(2),
            parent_image_id: Some(9),
            parent_placement_id: Some(9),
            ..Default::default()
        };

        assert_eq!(
            state.store_and_place(&replacement, pixels(2, 4), Point::default(), 1, 1),
            Err(GraphicsError::NoParent)
        );
        assert_eq!(
            state.image_by_id(NonZeroU32::new(1).unwrap()).unwrap().pixels().bytes(),
            &[1; 4]
        );
        assert_eq!(state.placements().count(), 1);
    }

    #[test]
    fn inferred_placement_spans_widen_pixel_offsets_before_scaling() {
        let mut state = GraphicsState::new(20_000);
        let image = Command { image_id: Some(1), ..Default::default() };
        state.store(&image, PixelBuffer::from_rgba(100, 50, Arc::from(vec![255; 20_000]))).unwrap();
        assert_eq!(
            state
                .placement_cell_span(
                    &Command { columns: Some(u32::MAX), x_offset: Some(1), ..image.clone() },
                    2,
                    2,
                )
                .unwrap(),
            (u32::MAX, 1 << 31)
        );
        assert_eq!(
            state
                .placement_cell_span(
                    &Command { rows: Some(u32::MAX), y_offset: Some(1), ..image },
                    8,
                    2,
                )
                .unwrap(),
            (1 << 31, u32::MAX)
        );
    }

    #[test]
    fn computes_explicit_inferred_and_native_placement_spans() {
        let mut state = GraphicsState::new(20_000);
        let image = Command { image_id: Some(1), ..Default::default() };
        state.store(&image, PixelBuffer::from_rgba(100, 50, Arc::from(vec![255; 20_000]))).unwrap();

        assert_eq!(state.placement_cell_span(&image, 10, 20).unwrap(), (10, 3));
        assert_eq!(
            state
                .placement_cell_span(&Command { columns: Some(5), ..image.clone() }, 10, 20)
                .unwrap(),
            (5, 2)
        );
        assert_eq!(
            state.placement_cell_span(&Command { rows: Some(2), ..image.clone() }, 10, 20).unwrap(),
            (8, 2)
        );

        state.store(&image, PixelBuffer::from_rgba(10, 20, Arc::from(vec![255; 800]))).unwrap();
        assert_eq!(
            state
                .placement_cell_span(
                    &Command { x_offset: Some(5), y_offset: Some(5), ..image },
                    10,
                    20,
                )
                .unwrap(),
            (2, 2)
        );
    }

    #[test]
    fn placement_geometry_clamps_offsets_and_intersects_source_crop() {
        let mut state = GraphicsState::new(400);
        let image = Command { image_id: Some(1), ..Default::default() };
        state.store(&image, PixelBuffer::from_rgba(10, 10, Arc::from(vec![255; 400]))).unwrap();

        assert_eq!(
            state
                .placement_cell_span(
                    &Command { x_offset: Some(99), y_offset: Some(99), ..image.clone() },
                    8,
                    16,
                )
                .unwrap(),
            (3, 2)
        );
        assert_eq!(
            state
                .placement_cell_span(
                    &Command {
                        x: Some(8),
                        y: Some(8),
                        crop_width: Some(20),
                        crop_height: Some(20),
                        ..image
                    },
                    1,
                    1,
                )
                .unwrap(),
            (2, 2)
        );
    }

    #[test]
    fn image_limit_allows_replacement_without_temporary_growth() {
        let mut state = GraphicsState::new(MAX_IMAGES_PER_BUFFER * 4);
        for image_id in 1..=MAX_IMAGES_PER_BUFFER as u32 {
            state
                .store(&Command { image_id: Some(image_id), ..Default::default() }, pixels(1, 4))
                .unwrap();
        }
        assert_eq!(state.images().count(), MAX_IMAGES_PER_BUFFER);
        state.store(&Command { image_id: Some(1), ..Default::default() }, pixels(2, 4)).unwrap();
        assert_eq!(
            state.store(
                &Command { image_id: Some(MAX_IMAGES_PER_BUFFER as u32 + 1), ..Default::default() },
                pixels(3, 4),
            ),
            Err(GraphicsError::NoSpace)
        );
    }

    #[test]
    fn placement_limit_allows_replacement_without_temporary_growth() {
        let mut state = GraphicsState::new(4);
        let image = Command { image_id: Some(1), ..Default::default() };
        state.store(&image, pixels(1, 4)).unwrap();
        for placement_id in 1..=crate::graphics::placement::MAX_PLACEMENTS_PER_BUFFER as u32 {
            state
                .place(
                    &Command { placement_id: Some(placement_id), ..image.clone() },
                    Point::default(),
                )
                .unwrap();
        }
        assert_eq!(
            state.placements().count(),
            crate::graphics::placement::MAX_PLACEMENTS_PER_BUFFER
        );
        state
            .place(
                &Command { placement_id: Some(1), ..image.clone() },
                Point::new(Line(1), crate::index::Column(1)),
            )
            .unwrap();
        assert_eq!(
            state.place(
                &Command {
                    placement_id: Some(
                        crate::graphics::placement::MAX_PLACEMENTS_PER_BUFFER as u32 + 1
                    ),
                    ..image
                },
                Point::default(),
            ),
            Err(GraphicsError::NoSpace)
        );
    }

    #[test]
    fn decode_limit_allows_eviction_at_commit() {
        let mut state = GraphicsState::new(8);
        let existing = Command { image_id: Some(1), ..Default::default() };
        state.store(&existing, pixels(1, 8)).unwrap();

        let replacement = Command { image_id: Some(2), ..Default::default() };
        assert_eq!(state.decode_limit(&replacement), 8);
        state.store(&replacement, pixels(2, 8)).unwrap();
        assert!(state.image_by_id(NonZeroU32::new(1).unwrap()).is_none());
        assert_eq!(
            state.image_by_id(NonZeroU32::new(2).unwrap()).unwrap().pixels().bytes(),
            &[2; 8]
        );
    }

    #[test]
    fn randomized_operations_preserve_graphics_state_invariants() {
        let mut state = GraphicsState::new(64);
        let mut random = 0x4b1d_5eed_u64;
        for _ in 0..5_000 {
            random ^= random << 13;
            random ^= random >> 7;
            random ^= random << 17;
            let id = (random as u32 % 8) + 1;
            let command = Command { image_id: Some(id), ..Default::default() };
            match (random >> 8) % 5 {
                0 => {
                    let _ = state.store(&command, pixels(id as u8, 4));
                },
                1 => {
                    let _ = state.place(
                        &Command { placement_id: Some(((random >> 16) as u32 % 4) + 1), ..command },
                        Point::new(
                            Line((random as i32 % 8).abs()),
                            crate::index::Column(id as usize),
                        ),
                    );
                },
                2 => {
                    let _ = state.delete(
                        &Command { delete: Some(crate::graphics::DeleteTarget(b'i')), ..command },
                        Point::default(),
                        Line(0)..Line(8),
                    );
                },
                3 => state.set_storage_limit(((random >> 24) as usize % 16 + 1) * 4),
                _ => state.scroll_up(&(Line(0)..Line(8)), 1, 8, true),
            }
            assert_invariants(&state);
        }
    }

    #[test]
    fn failed_replacement_preserves_old_image() {
        let id = NonZeroU32::new(7).unwrap();
        let command = Command { image_id: Some(id.get()), ..Default::default() };
        let mut state = GraphicsState::new(4);
        state.store(&command, pixels(1, 4)).unwrap();

        assert_eq!(state.store(&command, pixels(2, 5)), Err(GraphicsError::NoSpace));
        assert_eq!(state.image_by_id(id).unwrap().pixels().bytes(), &[1; 4]);
    }

    #[test]
    fn image_numbers_get_ids_and_resolve_newest() {
        let command = Command { image_number: Some(9), ..Default::default() };
        let mut state = GraphicsState::new(8);
        let first = state.store(&command, pixels(1, 4)).unwrap();
        let second = state.store(&command, pixels(2, 4)).unwrap();

        assert_eq!(first.image_id, NonZeroU32::new(1));
        assert_eq!(second.image_id, NonZeroU32::new(2));
        assert_eq!(state.newest_by_number(9).unwrap().pixels().bytes(), &[2; 4]);
        state
            .delete(
                &Command {
                    delete: Some(crate::graphics::DeleteTarget(b'N')),
                    image_number: Some(9),
                    ..Default::default()
                },
                Point::default(),
                Line(0)..Line(1),
            )
            .unwrap();
        assert_eq!(state.newest_by_number(9).unwrap().pixels().bytes(), &[1; 4]);

        state
            .delete(
                &Command {
                    delete: Some(crate::graphics::DeleteTarget(b'I')),
                    image_id: first.image_id.map(NonZeroU32::get),
                    ..Default::default()
                },
                Point::default(),
                Line(0)..Line(1),
            )
            .unwrap();
        let reused = state.store(&command, pixels(3, 4)).unwrap();
        assert_eq!(reused.image_id, first.image_id);
    }

    #[test]
    fn successful_replacement_and_placement_identity_follow_protocol() {
        let mut state = GraphicsState::new(16);
        let image = Command { image_id: Some(1), ..Default::default() };
        state.store(&image, pixels(1, 4)).unwrap();
        state.place(&Command { placement_id: Some(7), ..image.clone() }, Point::default()).unwrap();
        state
            .place(
                &Command { placement_id: Some(7), ..image.clone() },
                Point::new(Line(2), crate::index::Column(3)),
            )
            .unwrap();
        assert_eq!(state.placements().count(), 1);
        assert_eq!(
            state.placements().next().unwrap().anchor(),
            Point::new(Line(2), crate::index::Column(3))
        );

        state.place(&image, Point::default()).unwrap();
        state.place(&image, Point::new(Line(1), crate::index::Column(1))).unwrap();
        assert_eq!(state.placements().count(), 3);

        state.store(&image, pixels(2, 4)).unwrap();
        assert!(state.placements().next().is_none());
        assert_eq!(
            state.image_by_id(NonZeroU32::new(1).unwrap()).unwrap().pixels().bytes(),
            &[2; 4]
        );

        let anonymous = Command { image_id: Some(0), placement_id: Some(9), ..Default::default() };
        let outcome =
            state.store_and_place(&anonymous, pixels(3, 4), Point::default(), 1, 1).unwrap().0;
        assert_eq!(outcome.image_id, None);
        assert_eq!(outcome.image_number, None);
        assert_eq!(state.placements().next().unwrap().placement_id, None);

        let anonymous_number = Command { image_number: Some(0), ..Default::default() };
        let outcome = state.store(&anonymous_number, pixels(4, 4)).unwrap();
        assert_eq!(outcome.image_id, None);
        assert_eq!(outcome.image_number, None);
    }

    #[test]
    fn deletion_distinguishes_placement_and_image_data() {
        let id = NonZeroU32::new(1).unwrap();
        let mut state = GraphicsState::new(4);
        let image = Command { image_id: Some(id.get()), ..Default::default() };
        state.store(&image, pixels(1, 4)).unwrap();
        state.place(&image, Point::new(Line(0), crate::index::Column(0))).unwrap();
        let visible = Line(0)..Line(10);

        let soft = Command {
            delete: Some(crate::graphics::DeleteTarget(b'i')),
            image_id: Some(id.get()),
            ..Default::default()
        };
        state.delete(&soft, Point::default(), visible.clone()).unwrap();
        assert!(state.placements().next().is_none());
        assert!(state.image_by_id(id).is_some());

        let hard = Command { delete: Some(crate::graphics::DeleteTarget(b'I')), ..soft };
        state.delete(&hard, Point::default(), visible).unwrap();
        assert!(state.image_by_id(id).is_none());
    }

    #[test]
    fn deletion_selectors_cover_visibility_coordinates_z_and_ranges() {
        let setup = || {
            let mut state = GraphicsState::new(16);
            for id in 1..=4 {
                let image = Command { image_id: Some(id), ..Default::default() };
                state.store(&image, pixels(id as u8, 4)).unwrap();
                if id != 3 {
                    state
                        .place(
                            &Command {
                                placement_id: Some(1),
                                columns: Some(if id == 1 { 3 } else { 2 }),
                                rows: Some(if id == 1 { 3 } else { 2 }),
                                z_index: Some(if id == 1 { 5 } else { 7 }),
                                ..image
                            },
                            if id == 1 {
                                Point::new(Line(0), crate::index::Column(0))
                            } else if id == 2 {
                                Point::new(Line(5), crate::index::Column(5))
                            } else {
                                Point::new(Line(-2), crate::index::Column(0))
                            },
                        )
                        .unwrap();
                }
            }
            state
        };
        let visible = Line(0)..Line(10);

        for (selector, cursor, x, y, z, remaining_id, remaining_count) in [
            (b'c', Point::new(Line(1), crate::index::Column(1)), None, None, None, 2, 2),
            (b'p', Point::default(), Some(6), Some(6), None, 1, 2),
            (b'q', Point::default(), Some(2), Some(2), Some(5), 2, 2),
            (b'x', Point::default(), Some(2), None, None, 2, 1),
            (b'y', Point::default(), None, Some(6), None, 1, 2),
            (b'z', Point::default(), None, None, Some(7), 1, 1),
        ] {
            let mut state = setup();
            state
                .delete(
                    &Command {
                        delete: Some(crate::graphics::DeleteTarget(selector)),
                        x,
                        y,
                        z_index: z,
                        ..Default::default()
                    },
                    cursor,
                    visible.clone(),
                )
                .unwrap();
            let ids: Vec<_> =
                state.placements().filter_map(|placement| placement.image_id).collect();
            assert!(
                ids.contains(&NonZeroU32::new(remaining_id).unwrap()),
                "selector {}",
                selector as char
            );
            assert_eq!(ids.len(), remaining_count, "selector {}", selector as char);
        }

        let mut state = setup();
        state
            .delete(
                &Command {
                    delete: Some(crate::graphics::DeleteTarget(b'A')),
                    ..Default::default()
                },
                Point::default(),
                visible.clone(),
            )
            .unwrap();
        assert!(state.image_by_id(NonZeroU32::new(1).unwrap()).is_none());
        assert!(state.image_by_id(NonZeroU32::new(2).unwrap()).is_none());
        assert!(state.image_by_id(NonZeroU32::new(3).unwrap()).is_some());
        assert!(state.image_by_id(NonZeroU32::new(4).unwrap()).is_some());

        let mut state = setup();
        state
            .delete(
                &Command {
                    delete: Some(crate::graphics::DeleteTarget(b'R')),
                    ..Default::default()
                },
                Point::default(),
                visible.clone(),
            )
            .unwrap();
        assert_eq!(state.images().count(), 4);
        state
            .delete(
                &Command {
                    delete: Some(crate::graphics::DeleteTarget(b'R')),
                    y: Some(2),
                    ..Default::default()
                },
                Point::default(),
                visible,
            )
            .unwrap();
        assert!(state.image_by_id(NonZeroU32::new(1).unwrap()).is_none());
        assert!(state.image_by_id(NonZeroU32::new(2).unwrap()).is_none());
        assert!(state.image_by_id(NonZeroU32::new(3).unwrap()).is_some());
        assert_invariants(&state);

        let mut state = setup();
        state
            .delete(
                &Command {
                    delete: Some(crate::graphics::DeleteTarget(b'r')),
                    x: Some(3),
                    y: Some(2),
                    ..Default::default()
                },
                Point::default(),
                Line(0)..Line(10),
            )
            .unwrap();
        assert_eq!(state.images().count(), 4);
        assert_eq!(state.placements().count(), 3);
    }

    #[test]
    fn uppercase_coordinate_deletes_free_data_and_number_delete_honors_placement() {
        let visible = Line(0)..Line(10);
        for (selector, x, y, z) in [
            (b'C', None, None, None),
            (b'P', Some(1), Some(1), None),
            (b'Q', Some(1), Some(1), Some(5)),
            (b'X', Some(1), None, None),
            (b'Y', None, Some(1), None),
            (b'Z', None, None, Some(5)),
        ] {
            let mut state = GraphicsState::new(4);
            let image = Command { image_id: Some(1), ..Default::default() };
            state.store(&image, pixels(1, 4)).unwrap();
            state
                .place(
                    &Command { placement_id: Some(7), z_index: Some(5), ..image },
                    Point::default(),
                )
                .unwrap();
            state
                .delete(
                    &Command {
                        delete: Some(crate::graphics::DeleteTarget(selector)),
                        x,
                        y,
                        z_index: z,
                        ..Default::default()
                    },
                    Point::default(),
                    visible.clone(),
                )
                .unwrap();
            assert!(state.images().next().is_none(), "selector {}", selector as char);
        }

        let mut state = GraphicsState::new(8);
        for placement_id in [1, 2] {
            let command = Command { image_number: Some(9), ..Default::default() };
            state.store(&command, pixels(placement_id, 4)).unwrap();
            state
                .place(
                    &Command {
                        image_number: Some(9),
                        placement_id: Some(placement_id as u32),
                        ..Default::default()
                    },
                    Point::default(),
                )
                .unwrap();
        }
        state
            .delete(
                &Command {
                    delete: Some(crate::graphics::DeleteTarget(b'n')),
                    image_number: Some(9),
                    placement_id: Some(2),
                    ..Default::default()
                },
                Point::default(),
                visible,
            )
            .unwrap();
        assert_eq!(state.placements().count(), 1);
        assert_eq!(state.placements().next().unwrap().placement_id, NonZeroU32::new(1));
        assert_eq!(state.images().count(), 2);
    }

    #[test]
    fn virtual_placements_obey_only_identity_and_range_deletes() {
        for &selector in b"acpqxyzACPQXYZ" {
            let mut state = GraphicsState::new(4);
            let virtual_image = Command {
                image_id: Some(1),
                placement_id: Some(1),
                unicode_placeholder: Some(1),
                ..Default::default()
            };
            state.store(&virtual_image, pixels(1, 4)).unwrap();
            state.place(&virtual_image, Point::default()).unwrap();
            state
                .delete(
                    &Command {
                        delete: Some(crate::graphics::DeleteTarget(selector)),
                        x: Some(1),
                        y: Some(1),
                        z_index: Some(0),
                        ..Default::default()
                    },
                    Point::default(),
                    Line(0)..Line(10),
                )
                .unwrap();
            assert_eq!(state.placements().count(), 1, "selector {}", selector as char);
        }

        for &selector in b"iInNrR" {
            let mut state = GraphicsState::new(4);
            let virtual_image = Command {
                image_id: Some(1),
                image_number: None,
                placement_id: Some(1),
                unicode_placeholder: Some(1),
                ..Default::default()
            };
            state.store(&virtual_image, pixels(1, 4)).unwrap();
            state.place(&virtual_image, Point::default()).unwrap();
            let mut delete = Command {
                delete: Some(crate::graphics::DeleteTarget(selector)),
                image_id: Some(1),
                image_number: None,
                x: Some(1),
                y: Some(1),
                ..Default::default()
            };
            if matches!(selector, b'n' | b'N') {
                delete.image_id = None;
                delete.image_number = Some(7);
                let handle = state.command_image_handle(&virtual_image).unwrap();
                state.images.get_mut(&handle).unwrap().image_number = Some(7);
            }
            state.delete(&delete, Point::default(), Line(0)..Line(10)).unwrap();
            assert!(state.placements().next().is_none(), "selector {}", selector as char);
        }
    }

    #[test]
    fn virtual_placement_prototypes_ignore_scrolling() {
        for scroll_up in [true, false] {
            let mut state = GraphicsState::new(4);
            let command = Command {
                image_id: Some(1),
                unicode_placeholder: Some(1),
                columns: Some(8),
                rows: Some(4),
                ..Default::default()
            };
            state.store(&command, pixels(1, 4)).unwrap();
            state.place(&command, Point::new(Line(4), crate::index::Column(75))).unwrap();
            let region = Line(3)..Line(18);

            if scroll_up {
                state.scroll_up(&region, 15, 0, false);
            } else {
                state.scroll_down(&region, 15, false);
            }

            assert!(state.placeholder_renderable(1, 0).is_some());
            assert_eq!(
                state.placements().next().unwrap().anchor(),
                Point::new(Line(4), crate::index::Column(75))
            );
        }
    }

    #[test]
    fn partial_region_scroll_moves_only_contained_placements_and_retains_clipping() {
        let mut state = GraphicsState::new(4);
        let image = Command { image_id: Some(1), ..Default::default() };
        state.store(&image, pixels(1, 4)).unwrap();
        let contained = Command { columns: Some(1), rows: Some(4), ..image.clone() };
        state.place(&contained, Point::new(Line(2), crate::index::Column(0))).unwrap();
        state.place(&contained, Point::new(Line(0), crate::index::Column(1))).unwrap();
        let region = Line(1)..Line(8);

        state.scroll_up(&region, 3, 0, false);
        let mut placements: Vec<_> = state.placements().collect();
        placements.sort_by_key(|placement| placement.anchor().column);
        assert_eq!(placements[0].anchor().line, Line(-1));
        assert_eq!(placements[0].clip_region, Some((Line(1), Line(8))));
        assert_eq!(placements[1].anchor().line, Line(0));

        state.scroll_up(&region, 3, 0, false);
        assert_eq!(state.placements().count(), 1);
    }

    #[test]
    fn reverse_partial_region_scroll_moves_only_contained_placements() {
        let mut state = GraphicsState::new(4);
        let image = Command { image_id: Some(1), rows: Some(2), ..Default::default() };
        state.store(&image, pixels(1, 4)).unwrap();
        state.place(&image, Point::new(Line(2), crate::index::Column(0))).unwrap();
        state.place(&image, Point::new(Line(0), crate::index::Column(1))).unwrap();
        let region = Line(1)..Line(8);

        state.scroll_down(&region, 2, false);
        let mut placements: Vec<_> = state.placements().collect();
        placements.sort_by_key(|placement| placement.anchor().column);
        assert_eq!(placements[0].anchor().line, Line(4));
        assert_eq!(placements[0].clip_region, Some((Line(1), Line(8))));
        assert_eq!(placements[1].anchor().line, Line(0));
    }

    #[test]
    fn placements_follow_scrollback_and_are_pruned_with_history() {
        let mut state = GraphicsState::new(4);
        let command = Command { image_id: Some(1), ..Default::default() };
        state.store(&command, pixels(1, 4)).unwrap();
        state.place(&command, Point::new(Line(0), crate::index::Column(2))).unwrap();
        let region = Line(0)..Line(10);

        state.scroll_up(&region, 1, 2, true);
        assert_eq!(state.placements().next().unwrap().anchor().line, Line(-1));
        state.scroll_up(&region, 2, 2, true);
        assert!(state.placements().next().is_none());
    }

    #[test]
    fn relative_placement_requires_parent_and_tracks_its_position() {
        let mut state = GraphicsState::new(8);
        let parent = Command { image_id: Some(1), placement_id: Some(1), ..Default::default() };
        let child_image = Command { image_id: Some(2), ..Default::default() };
        state.store(&parent, pixels(1, 4)).unwrap();
        state.store(&child_image, pixels(2, 4)).unwrap();

        let relative = Command {
            image_id: Some(2),
            placement_id: Some(1),
            parent_image_id: Some(1),
            parent_placement_id: Some(1),
            horizontal_offset: Some(2),
            vertical_offset: Some(-1),
            ..Default::default()
        };
        assert_eq!(state.place(&relative, Point::default()), Err(GraphicsError::NoParent));

        state.place(&parent, Point::new(Line(3), crate::index::Column(4))).unwrap();
        state.place(&relative, Point::default()).unwrap();
        let child = state
            .renderables()
            .into_iter()
            .find(|renderable| {
                renderable.image == state.image_by_id(NonZeroU32::new(2).unwrap()).unwrap().handle()
            })
            .unwrap();
        assert_eq!((child.line, child.column), (Line(2), 6));

        state.scroll_up(&(Line(0)..Line(10)), 1, 10, true);
        let child = state
            .renderables()
            .into_iter()
            .find(|renderable| {
                renderable.image == state.image_by_id(NonZeroU32::new(2).unwrap()).unwrap().handle()
            })
            .unwrap();
        assert_eq!((child.line, child.column), (Line(1), 6));

        state
            .place(
                &Command { placement_id: Some(2), horizontal_offset: Some(-6), ..relative },
                Point::default(),
            )
            .unwrap();
        assert!(state.renderables().iter().any(|renderable| renderable.column == -2));
    }

    #[test]
    fn relative_parent_without_placement_id_uses_lowest_parent_placement() {
        let mut state = GraphicsState::new(8);
        let parent = Command { image_id: Some(1), ..Default::default() };
        let child = Command { image_id: Some(2), ..Default::default() };
        state.store(&parent, pixels(1, 4)).unwrap();
        state.store(&child, pixels(2, 4)).unwrap();
        state
            .place(
                &Command { placement_id: Some(7), ..parent.clone() },
                Point::new(Line(1), crate::index::Column(1)),
            )
            .unwrap();
        state
            .place(
                &Command { placement_id: Some(3), ..parent },
                Point::new(Line(2), crate::index::Column(2)),
            )
            .unwrap();
        state
            .place(
                &Command { placement_id: Some(1), parent_image_id: Some(1), ..child },
                Point::default(),
            )
            .unwrap();

        let child =
            state.renderables().into_iter().find(|renderable| renderable.image_id == 2).unwrap();
        assert_eq!((child.line, child.column), (Line(2), 2));
    }

    #[test]
    fn location_deletion_does_not_affect_virtual_placements() {
        let mut state = GraphicsState::new(4);
        let prototype = Command {
            image_id: Some(1),
            placement_id: Some(1),
            unicode_placeholder: Some(1),
            ..Default::default()
        };
        state.store(&prototype, pixels(1, 4)).unwrap();
        state.place(&prototype, Point::default()).unwrap();
        state
            .delete(
                &Command {
                    delete: Some(crate::graphics::DeleteTarget(b'a')),
                    ..Default::default()
                },
                Point::default(),
                Line(0)..Line(10),
            )
            .unwrap();
        assert_eq!(state.placements().count(), 1);
    }

    #[test]
    fn virtual_placement_cannot_be_relative() {
        let mut state = GraphicsState::new(8);
        for id in [1, 2] {
            state
                .store(&Command { image_id: Some(id), ..Default::default() }, pixels(id as u8, 4))
                .unwrap();
        }
        state
            .place(
                &Command { image_id: Some(1), placement_id: Some(1), ..Default::default() },
                Point::default(),
            )
            .unwrap();
        assert_eq!(
            state.place(
                &Command {
                    image_id: Some(2),
                    unicode_placeholder: Some(1),
                    parent_image_id: Some(1),
                    parent_placement_id: Some(1),
                    ..Default::default()
                },
                Point::default(),
            ),
            Err(GraphicsError::Invalid)
        );
        assert_eq!(state.placements().count(), 1);
    }

    #[test]
    fn relative_placement_resolves_virtual_parent_from_placeholder_origin() {
        let mut state = GraphicsState::new(8);
        let prototype = Command {
            image_id: Some(1),
            unicode_placeholder: Some(1),
            columns: Some(1),
            rows: Some(1),
            ..Default::default()
        };
        let child_image = Command { image_id: Some(2), ..Default::default() };
        state.store(&prototype, pixels(1, 4)).unwrap();
        state.store(&child_image, pixels(2, 4)).unwrap();
        state.place(&prototype, Point::default()).unwrap();
        state
            .place(
                &Command {
                    image_id: Some(2),
                    placement_id: Some(1),
                    parent_image_id: Some(1),
                    horizontal_offset: Some(3),
                    vertical_offset: Some(2),
                    ..Default::default()
                },
                Point::default(),
            )
            .unwrap();

        assert!(state.renderables().is_empty());
        let prototype_handle = state.virtual_prototypes()[&(1, 0)];
        let renderables = state.renderables_with_virtual_origins(|handle| {
            (handle == prototype_handle).then_some(Point::new(Line(4), crate::index::Column(5)))
        });
        assert_eq!(renderables.len(), 1);
        assert_eq!((renderables[0].line, renderables[0].column), (Line(6), 8));
    }

    #[test]
    fn relative_placement_rejects_excessive_depth_and_cycles() {
        let mut state = GraphicsState::new(4);
        let image = Command { image_id: Some(1), ..Default::default() };
        state.store(&image, pixels(1, 4)).unwrap();
        state.place(&Command { placement_id: Some(1), ..image.clone() }, Point::default()).unwrap();
        for placement_id in 2..=9 {
            state
                .place(
                    &Command {
                        placement_id: Some(placement_id),
                        parent_image_id: Some(1),
                        parent_placement_id: Some(placement_id - 1),
                        ..image.clone()
                    },
                    Point::default(),
                )
                .unwrap();
        }
        assert_eq!(
            state.place(
                &Command {
                    placement_id: Some(10),
                    parent_image_id: Some(1),
                    parent_placement_id: Some(9),
                    ..image.clone()
                },
                Point::default(),
            ),
            Err(GraphicsError::TooDeep)
        );
        assert_eq!(
            state.place(
                &Command {
                    placement_id: Some(1),
                    parent_image_id: Some(1),
                    parent_placement_id: Some(9),
                    ..image.clone()
                },
                Point::default(),
            ),
            Err(GraphicsError::Cycle)
        );
        assert_eq!(
            state.place(
                &Command {
                    placement_id: Some(9),
                    parent_image_id: Some(1),
                    parent_placement_id: Some(9),
                    ..image
                },
                Point::default(),
            ),
            Err(GraphicsError::Cycle)
        );
    }

    #[test]
    fn replacing_relative_parent_cascades_and_frees_orphan_child_image() {
        let mut state = GraphicsState::new(8);
        let parent = Command { image_id: Some(1), placement_id: Some(1), ..Default::default() };
        let child = Command { image_id: Some(2), ..Default::default() };
        state.store(&parent, pixels(1, 4)).unwrap();
        state.store(&child, pixels(2, 4)).unwrap();
        state.place(&parent, Point::default()).unwrap();
        state
            .place(
                &Command {
                    image_id: Some(2),
                    placement_id: Some(1),
                    parent_image_id: Some(1),
                    parent_placement_id: Some(1),
                    ..Default::default()
                },
                Point::default(),
            )
            .unwrap();

        state.place(&parent, Point::new(Line(1), crate::index::Column(1))).unwrap();
        assert_eq!(state.placements().count(), 1);
        assert!(state.image_by_id(NonZeroU32::new(2).unwrap()).is_none());
    }

    #[test]
    fn deleting_relative_parent_cascades_and_frees_orphan_child_image() {
        let mut state = GraphicsState::new(8);
        let parent = Command { image_id: Some(1), placement_id: Some(1), ..Default::default() };
        let child = Command { image_id: Some(2), ..Default::default() };
        state.store(&parent, pixels(1, 4)).unwrap();
        state.store(&child, pixels(2, 4)).unwrap();
        state.place(&parent, Point::default()).unwrap();
        state
            .place(
                &Command {
                    image_id: Some(2),
                    placement_id: Some(1),
                    parent_image_id: Some(1),
                    parent_placement_id: Some(1),
                    ..Default::default()
                },
                Point::default(),
            )
            .unwrap();

        state
            .delete(
                &Command {
                    delete: Some(crate::graphics::DeleteTarget(b'i')),
                    image_id: Some(1),
                    placement_id: Some(1),
                    ..Default::default()
                },
                Point::default(),
                Line(0)..Line(10),
            )
            .unwrap();
        assert!(state.placements().next().is_none());
        assert!(state.image_by_id(NonZeroU32::new(2).unwrap()).is_none());
    }

    #[test]
    fn equal_z_renderables_order_by_image_id_then_creation() {
        let mut state = GraphicsState::new(12);
        for id in [3, 1, 2] {
            let image = Command { image_id: Some(id), z_index: Some(5), ..Default::default() };
            state.store(&image, pixels(id as u8, 4)).unwrap();
            state.place(&image, Point::default()).unwrap();
        }
        let ids: Vec<_> = state.renderables().into_iter().map(|image| image.image_id).collect();
        assert_eq!(ids, [1, 2, 3]);
    }

    #[test]
    fn eviction_prioritizes_transient_and_unplaced_images() {
        let mut state = GraphicsState::new(8);
        let first = Command { image_id: Some(1), ..Default::default() };
        let transient = Command { image_id: Some(2), usage: Some(3), ..Default::default() };
        state.store(&first, pixels(1, 4)).unwrap();
        state.store(&transient, pixels(2, 4)).unwrap();
        let transient_handle = state.command_image_handle(&transient).unwrap();
        state
            .place(
                &Command { image_id: Some(2), usage: Some(0), ..Default::default() },
                Point::new(Line(-1), crate::index::Column(0)),
            )
            .unwrap();
        assert!(state.images[&transient_handle].transient);
        state
            .delete(
                &Command {
                    delete: Some(crate::graphics::DeleteTarget(b'i')),
                    image_id: Some(2),
                    ..Default::default()
                },
                Point::default(),
                Line(0)..Line(10),
            )
            .unwrap();
        state.store(&Command { image_id: Some(3), ..Default::default() }, pixels(3, 4)).unwrap();
        assert!(state.image_by_id(NonZeroU32::new(1).unwrap()).is_some());
        assert!(state.image_by_id(NonZeroU32::new(2).unwrap()).is_none());

        state.place(&first, Point::default()).unwrap();
        state.store(&Command { image_id: Some(4), ..Default::default() }, pixels(4, 4)).unwrap();
        assert!(state.image_by_id(NonZeroU32::new(1).unwrap()).is_some());
        assert!(state.image_by_id(NonZeroU32::new(3).unwrap()).is_none());
        assert_invariants(&state);

        let mut state = GraphicsState::new(8);
        state.set_visible_lines(10);
        let visible = Command { image_id: Some(1), ..Default::default() };
        let history = Command { image_id: Some(2), ..Default::default() };
        state.store(&visible, pixels(1, 4)).unwrap();
        state.place(&visible, Point::new(Line(0), crate::index::Column(0))).unwrap();
        state.store(&history, pixels(2, 4)).unwrap();
        state.place(&history, Point::new(Line(-2), crate::index::Column(0))).unwrap();
        state.store(&Command { image_id: Some(3), ..Default::default() }, pixels(3, 4)).unwrap();
        assert!(state.image_by_id(NonZeroU32::new(1).unwrap()).is_some());
        assert!(state.image_by_id(NonZeroU32::new(2).unwrap()).is_none());
        assert_invariants(&state);
    }

    #[test]
    fn native_footprint_visibility_protects_partially_scrolled_image() {
        let mut state = GraphicsState::new(12);
        state.set_visible_lines(1);
        let visible = Command { image_id: Some(1), ..Default::default() };
        state.store(&visible, PixelBuffer::from_rgba(1, 2, Arc::from([255; 8]))).unwrap();
        state.place(&visible, Point::new(Line(-1), crate::index::Column(0))).unwrap();
        let hidden = Command { image_id: Some(2), ..Default::default() };
        state.store(&hidden, pixels(2, 4)).unwrap();
        state.place(&hidden, Point::new(Line(-2), crate::index::Column(0))).unwrap();
        state.store(&Command { image_id: Some(3), ..Default::default() }, pixels(3, 4)).unwrap();
        assert!(state.image_by_id(NonZeroU32::new(1).unwrap()).is_some());
        assert!(state.image_by_id(NonZeroU32::new(2).unwrap()).is_none());
    }

    #[test]
    fn frame_composition_defaults_and_mode_follow_protocol() {
        fn compose_with_mode(mode: u32) -> Vec<u8> {
            let mut state = GraphicsState::new(16);
            let image = Command { image_id: Some(1), ..Default::default() };
            state.store(&image, PixelBuffer::from_rgba(1, 1, Arc::from([100, 0, 0, 255]))).unwrap();
            state
                .store_frame(
                    &Command { image_id: Some(1), ..Default::default() },
                    PixelBuffer::from_rgba(1, 1, Arc::from([0, 100, 0, 128])),
                )
                .unwrap();
            state
                .compose_frames(&Command {
                    image_id: Some(1),
                    rows: Some(2),
                    columns: Some(1),
                    cursor_policy: Some(mode),
                    ..Default::default()
                })
                .unwrap();
            state.image_by_id(NonZeroU32::new(1).unwrap()).unwrap().pixels().bytes().to_vec()
        }

        assert_eq!(compose_with_mode(2), compose_with_mode(0));

        let mut state = GraphicsState::new(32);
        let image = Command { image_id: Some(1), ..Default::default() };
        state.store(&image, PixelBuffer::from_rgba(2, 1, Arc::from([0; 8]))).unwrap();
        state
            .store_frame(
                &Command { image_id: Some(1), ..Default::default() },
                PixelBuffer::from_rgba(2, 1, Arc::from([1; 8])),
            )
            .unwrap();
        for (crop_width, crop_height) in [(None, None), (Some(0), Some(0))] {
            assert_eq!(
                state.compose_frames(&Command {
                    image_id: Some(1),
                    rows: Some(2),
                    columns: Some(1),
                    x_offset: Some(1),
                    crop_width,
                    crop_height,
                    ..Default::default()
                }),
                Err(GraphicsError::Invalid)
            );
        }
    }

    #[test]
    fn repeated_upload_place_and_delete_restores_all_accounting() {
        let mut state = GraphicsState::new(4);
        for generation in 0..100 {
            let image = Command { image_id: Some(1), ..Default::default() };
            state.store(&image, pixels(generation, 4)).unwrap();
            state.place(&image, Point::default()).unwrap();
            state
                .delete(
                    &Command {
                        delete: Some(crate::graphics::DeleteTarget(b'I')),
                        image_id: Some(1),
                        ..Default::default()
                    },
                    Point::default(),
                    Line(0)..Line(1),
                )
                .unwrap();
            assert_eq!(
                (state.used_bytes(), state.images().count(), state.placements().count()),
                (0, 0, 0)
            );
            assert_invariants(&state);
        }
    }

    #[test]
    fn evicts_oldest_image_deterministically() {
        let mut state = GraphicsState::new(8);
        let first = Command { image_id: Some(1), ..Default::default() };
        let second = Command { image_id: Some(2), ..Default::default() };
        let third = Command { image_id: Some(3), ..Default::default() };
        state.store(&first, pixels(1, 4)).unwrap();
        state.store(&second, pixels(2, 4)).unwrap();
        state.store(&third, pixels(3, 4)).unwrap();

        assert!(state.image_by_id(NonZeroU32::new(1).unwrap()).is_none());
        assert!(state.image_by_id(NonZeroU32::new(2).unwrap()).is_some());
        assert!(state.image_by_id(NonZeroU32::new(3).unwrap()).is_some());
    }
}
