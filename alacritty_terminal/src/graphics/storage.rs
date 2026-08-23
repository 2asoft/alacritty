use std::collections::HashMap;
use std::num::NonZeroU32;

use crate::index::{Line, Point};

use super::{
    Action, Command, GraphicsError, PixelBuffer, Placement, PlacementHandle, Placements,
    RenderableGraphic,
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
        let mut renderables: Vec<_> = self
            .placements
            .values()
            .filter(|placement| !placement.virtual_placement)
            .filter_map(|placement| {
                let image = self.images.get(&placement.image)?;
                Some(RenderableGraphic {
                    image: image.handle,
                    pixels: image.pixels.clone(),
                    anchor: placement.anchor,
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
            anchor: placement.anchor,
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
        self.placements.scroll_up(region, lines, history_size);
    }

    pub fn scroll_down(&mut self, region: &std::ops::Range<Line>, lines: usize) {
        self.placements.scroll_down(region, lines);
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

        let cell = || {
            let x = command.x.unwrap_or(1).saturating_sub(1) as usize;
            let y = i64::from(command.y.unwrap_or(1).saturating_sub(1));
            (x, y)
        };
        let affected = self.placements.remove_matching(|placement| {
            let columns = placement.columns.unwrap_or(1) as usize;
            let rows = i64::from(placement.rows.unwrap_or(1));
            let top = i64::from(placement.anchor.line.0);
            let intersects = |(column, line): (usize, i64)| {
                line >= top
                    && line < top + rows
                    && column >= placement.anchor.column.0
                    && column < placement.anchor.column.0.saturating_add(columns)
            };
            match selector {
                b'a' => {
                    placement.anchor.line >= visible_lines.start
                        && placement.anchor.line < visible_lines.end
                },
                b'i' | b'n' => {
                    Some(placement.image) == explicit_image
                        && placement_id.is_none_or(|id| placement.placement_id == Some(id))
                },
                b'c' => intersects((cursor.column.0, i64::from(cursor.line.0))),
                b'p' => intersects(cell()),
                b'q' => intersects(cell()) && placement.z_index == command.z_index.unwrap_or(0),
                b'x' => {
                    let column = command.x.unwrap_or(1).saturating_sub(1) as usize;
                    column >= placement.anchor.column.0
                        && column < placement.anchor.column.0.saturating_add(columns)
                },
                b'y' => {
                    let line = i64::from(command.y.unwrap_or(1).saturating_sub(1));
                    line >= top && line < top + rows
                },
                b'z' => placement.z_index == command.z_index.unwrap_or(0),
                b'r' => placement.image_id.is_some_and(|id| {
                    id.get() >= command.x.unwrap_or(0) && id.get() <= command.y.unwrap_or(u32::MAX)
                }),
                _ => false,
            }
        });

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
        self.placements.insert(handle, image_id, command, anchor)
    }

    pub fn place_handle(
        &mut self,
        handle: ImageHandle,
        command: &Command,
        anchor: Point,
    ) -> Result<PlacementHandle, GraphicsError> {
        let image_id = self.images.get(&handle).ok_or(GraphicsError::NotFound)?.external_id;
        self.placements.insert(handle, image_id, command, anchor)
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
            self.placements.remove_image(handle);
            self.used_bytes -= image.pixels.storage_bytes();
            if let Some(id) = image.external_id {
                self.image_ids.remove(&id);
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
    fn deletion_handles_large_row_spans() {
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
