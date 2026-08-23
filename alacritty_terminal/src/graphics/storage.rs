use std::collections::HashMap;
use std::num::NonZeroU32;

use crate::index::{Line, Point};

use super::{
    Action, Command, GraphicsError, PixelBuffer, Placement, PlacementHandle, PlacementInsert,
    Placements, RenderableGraphic,
};

pub const MAX_IMAGES_PER_BUFFER: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ImageHandle(u64);

#[derive(Clone, Debug)]
pub struct Image {
    handle: ImageHandle,
    external_id: Option<NonZeroU32>,
    image_number: Option<u32>,
    pixels: PixelBuffer,
    content_generation: u64,
    creation_serial: u64,
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
        &self.pixels
    }

    pub fn content_generation(&self) -> u64 {
        self.content_generation
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StoreOutcome {
    pub handle: ImageHandle,
    pub image_id: Option<NonZeroU32>,
    pub image_number: Option<u32>,
}

#[derive(Debug)]
pub struct GraphicsState {
    images: HashMap<ImageHandle, Image>,
    image_ids: HashMap<NonZeroU32, ImageHandle>,
    placements: Placements,
    used_bytes: usize,
    storage_limit: usize,
    next_handle: u64,
    next_image_id: u32,
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
            next_handle: 1,
            next_image_id: 1,
            serial: 0,
        }
    }

    pub fn used_bytes(&self) -> usize {
        self.used_bytes
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

    pub fn renderables(&self) -> Vec<RenderableGraphic> {
        self.renderables_with_virtual_origins(|_, _| None)
    }

    pub fn renderables_with_virtual_origins(
        &self,
        virtual_origin: impl Fn(u32, u32) -> Option<Point>,
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
                    pixels: image.pixels.clone(),
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
                    content_generation: image.content_generation,
                    creation_serial: placement.creation_serial,
                })
            })
            .collect();
        renderables.sort_by_key(|graphic| {
            let image_id = self
                .images
                .get(&graphic.image)
                .and_then(|image| image.external_id)
                .map_or(0, NonZeroU32::get);
            (graphic.z_index, image_id, graphic.creation_serial)
        });
        renderables
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
            pixels: image.pixels.clone(),
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
            content_generation: image.content_generation,
            creation_serial: placement.creation_serial,
        })
    }

    pub fn scroll_up(&mut self, region: &std::ops::Range<Line>, lines: usize, history_size: usize) {
        let relative_images = self.placements.scroll_up(region, lines, history_size);
        self.remove_orphaned_relative_images(relative_images);
    }

    pub fn scroll_down(&mut self, region: &std::ops::Range<Line>, lines: usize) {
        let relative_images = self.placements.scroll_down(region, lines);
        self.remove_orphaned_relative_images(relative_images);
    }

    pub fn delete(
        &mut self,
        command: &Command,
        cursor: Point,
        visible_lines: std::ops::Range<Line>,
    ) -> Result<(), GraphicsError> {
        let selector = command.delete.map_or(b'a', |selector| selector.0);
        let free_data = selector.is_ascii_uppercase();
        let selector = selector.to_ascii_lowercase();
        let placement_id = NonZeroU32::new(command.placement_id.unwrap_or(0));

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
                    .resolved_anchor(placement, &|_, _| None)
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
            let columns = i64::from(placement.columns.unwrap_or(1));
            let rows = i64::from(placement.rows.unwrap_or(1));
            let location = resolved_anchors.get(&placement.handle).copied().unwrap_or_else(|| {
                super::placement::ResolvedPlacement {
                    line: placement.anchor.line,
                    column: i32::try_from(placement.anchor.column.0).unwrap_or(i32::MAX),
                }
            });
            let top = i64::from(location.line.0);
            let left = i64::from(location.column);
            let intersects = |(column, line): (i64, i64)| {
                line >= top && line < top + rows && column >= left && column < left + columns
            };
            match selector {
                b'a' => location.line >= visible_lines.start && location.line < visible_lines.end,
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
                b'x' => cell.0 >= left && cell.0 < left + columns,
                b'y' => cell.1 >= top && cell.1 < top + rows,
                b'z' => placement.z_index == command.z_index.unwrap_or(0),
                b'r' => placement.image_id.is_some_and(|id| {
                    id.get() >= command.x.unwrap_or(0) && id.get() <= command.y.unwrap_or(u32::MAX)
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
                        id.get() >= command.x.unwrap_or(0)
                            && id.get() <= command.y.unwrap_or(u32::MAX)
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
        let inserted = self.placements.insert(handle, image_id, command, anchor)?;
        self.finish_placement_insert(inserted)
    }

    pub fn place_handle(
        &mut self,
        handle: ImageHandle,
        command: &Command,
        anchor: Point,
    ) -> Result<PlacementHandle, GraphicsError> {
        let image_id = self.images.get(&handle).ok_or(GraphicsError::NotFound)?.external_id;
        let inserted = self.placements.insert(handle, image_id, command, anchor)?;
        self.finish_placement_insert(inserted)
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
    }

    pub fn set_storage_limit(&mut self, storage_limit: usize) {
        self.storage_limit = storage_limit;
        self.evict_until(0);
    }

    pub fn store(
        &mut self,
        command: &Command,
        pixels: PixelBuffer,
    ) -> Result<StoreOutcome, GraphicsError> {
        if command.image_id.unwrap_or(0) != 0 && command.image_number.unwrap_or(0) != 0 {
            return Err(GraphicsError::Invalid);
        }
        if command.action == Some(Action::Query) {
            return Ok(StoreOutcome {
                handle: ImageHandle(0),
                image_id: NonZeroU32::new(command.image_id.unwrap_or(0)),
                image_number: command.image_number,
            });
        }

        let external_id =
            match (NonZeroU32::new(command.image_id.unwrap_or(0)), command.image_number) {
                (Some(id), _) => Some(id),
                (None, Some(_)) => Some(self.allocate_image_id()?),
                (None, None) => None,
            };
        let replaced = external_id.and_then(|id| self.image_ids.get(&id).copied());
        let replaced_bytes = replaced
            .and_then(|handle| self.images.get(&handle))
            .map_or(0, |image| image.pixels.storage_bytes());
        let required = pixels.storage_bytes();
        if required > self.storage_limit {
            return Err(GraphicsError::NoSpace);
        }
        if replaced.is_none() && self.images.len() == MAX_IMAGES_PER_BUFFER {
            return Err(GraphicsError::NoSpace);
        }

        self.evict_until_with_exclusion(required, replaced_bytes, replaced);
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

        let handle = ImageHandle(self.next_handle);
        self.next_handle = self.next_handle.checked_add(1).ok_or(GraphicsError::TooLarge)?;
        self.serial = self.serial.checked_add(1).ok_or(GraphicsError::TooLarge)?;
        let image = Image {
            handle,
            external_id,
            image_number: command.image_number,
            pixels,
            content_generation: self.serial,
            creation_serial: self.serial,
        };
        self.used_bytes = self
            .used_bytes
            .checked_add(image.pixels.storage_bytes())
            .ok_or(GraphicsError::TooLarge)?;
        self.images.insert(handle, image);
        if let Some(id) = external_id {
            self.image_ids.insert(id, handle);
        }

        Ok(StoreOutcome { handle, image_id: external_id, image_number: command.image_number })
    }

    fn allocate_image_id(&mut self) -> Result<NonZeroU32, GraphicsError> {
        for _ in 0..u32::MAX {
            let candidate = NonZeroU32::new(self.next_image_id).ok_or(GraphicsError::TooLarge)?;
            self.next_image_id = self.next_image_id.checked_add(1).unwrap_or(1);
            if !self.image_ids.contains_key(&candidate) {
                return Ok(candidate);
            }
        }
        Err(GraphicsError::NoSpace)
    }

    fn evict_until(&mut self, incoming: usize) {
        self.evict_until_with_exclusion(incoming, 0, None);
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
                .min_by_key(|image| (image.creation_serial, image.handle.0))
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
            self.used_bytes -= image.pixels.storage_bytes();
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn pixels(value: u8, bytes: usize) -> PixelBuffer {
        PixelBuffer::from_rgba(1, 1, Arc::from(vec![value; bytes]))
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

        assert_ne!(first.image_id, second.image_id);
        assert_eq!(state.newest_by_number(9).unwrap().pixels().bytes(), &[2; 4]);
    }

    #[test]
    fn deletion_handles_large_spans_and_relative_offsets() {
        for rows in [i32::MAX as u32, u32::MAX] {
            let mut state = GraphicsState::new(4);
            let image = Command { image_id: Some(1), rows: Some(rows), ..Default::default() };
            state.store(&image, pixels(1, 4)).unwrap();
            state.place(&image, Point::new(Line(1), crate::index::Column(0))).unwrap();
            let delete = Command {
                delete: Some(crate::graphics::DeleteTarget(b'y')),
                y: Some(i32::MAX as u32 + 1),
                ..Default::default()
            };
            state.delete(&delete, Point::default(), Line(0)..Line(24)).unwrap();
            assert!(state.placements().next().is_none());
        }
        let mut state = GraphicsState::new(8);
        let root = Command { image_id: Some(1), placement_id: Some(1), ..Default::default() };
        state.store(&root, pixels(1, 4)).unwrap();
        state.place(&root, Point::default()).unwrap();
        let child = Command {
            image_id: Some(2),
            placement_id: Some(1),
            parent_image_id: Some(1),
            parent_placement_id: Some(1),
            vertical_offset: Some(i32::MAX),
            columns: Some(1),
            rows: Some(1),
            ..Default::default()
        };
        state.store(&child, pixels(1, 4)).unwrap();
        state.place(&child, Point::default()).unwrap();
        let delete = Command {
            delete: Some(crate::graphics::DeleteTarget(b'y')),
            y: Some(i32::MAX as u32 + 1),
            ..Default::default()
        };
        state.delete(&delete, Point::default(), Line(0)..Line(24)).unwrap();
        assert_eq!(state.placements().count(), 1);
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
    fn placements_follow_scrollback_and_are_pruned_with_history() {
        let mut state = GraphicsState::new(4);
        let command = Command { image_id: Some(1), ..Default::default() };
        state.store(&command, pixels(1, 4)).unwrap();
        state.place(&command, Point::new(Line(0), crate::index::Column(2))).unwrap();
        let region = Line(0)..Line(10);

        state.scroll_up(&region, 1, 2);
        assert_eq!(state.placements().next().unwrap().anchor().line, Line(-1));
        state.scroll_up(&region, 2, 2);
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

        state.scroll_up(&(Line(0)..Line(10)), 1, 10);
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
    fn relative_placement_resolves_virtual_parent_from_placeholder_origin() {
        let mut state = GraphicsState::new(8);
        let prototype = Command {
            image_id: Some(1),
            placement_id: Some(1),
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
                    parent_placement_id: Some(1),
                    horizontal_offset: Some(3),
                    vertical_offset: Some(2),
                    ..Default::default()
                },
                Point::default(),
            )
            .unwrap();

        assert!(state.renderables().is_empty());
        let renderables = state.renderables_with_virtual_origins(|image_id, placement_id| {
            (image_id == 1 && placement_id == 1)
                .then_some(Point::new(Line(4), crate::index::Column(5)))
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
