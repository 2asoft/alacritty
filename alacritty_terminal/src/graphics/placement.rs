use std::collections::HashMap;
use std::num::NonZeroU32;

use crate::index::{Line, Point};

use super::{Command, GraphicsError, ImageHandle, PixelBuffer};

pub const MAX_PLACEMENTS_PER_BUFFER: usize = 65_536;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PlacementHandle(pub(crate) u64);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Placement {
    pub(crate) handle: PlacementHandle,
    pub(crate) image: ImageHandle,
    pub(crate) image_id: Option<NonZeroU32>,
    pub(crate) placement_id: Option<NonZeroU32>,
    pub(crate) anchor: Point,
    pub(crate) source_x: u32,
    pub(crate) source_y: u32,
    pub(crate) source_width: Option<u32>,
    pub(crate) source_height: Option<u32>,
    pub(crate) x_offset: u32,
    pub(crate) y_offset: u32,
    pub(crate) columns: Option<u32>,
    pub(crate) rows: Option<u32>,
    pub(crate) z_index: i32,
    pub(crate) creation_serial: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderableGraphic {
    pub image: ImageHandle,
    pub pixels: PixelBuffer,
    pub anchor: Point,
    pub source_x: u32,
    pub source_y: u32,
    pub source_width: Option<u32>,
    pub source_height: Option<u32>,
    pub x_offset: u32,
    pub y_offset: u32,
    pub columns: Option<u32>,
    pub rows: Option<u32>,
    pub z_index: i32,
    pub content_generation: u64,
    pub creation_serial: u64,
}

impl Placement {
    pub fn handle(&self) -> PlacementHandle {
        self.handle
    }

    pub fn image(&self) -> ImageHandle {
        self.image
    }

    pub fn anchor(&self) -> Point {
        self.anchor
    }

    pub fn z_index(&self) -> i32 {
        self.z_index
    }
}

#[derive(Debug, Default)]
pub(crate) struct Placements {
    entries: HashMap<PlacementHandle, Placement>,
    named: HashMap<(NonZeroU32, NonZeroU32), PlacementHandle>,
    next_handle: u64,
    serial: u64,
}

impl Placements {
    pub fn values(&self) -> impl Iterator<Item = &Placement> {
        self.entries.values()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.named.clear();
    }

    pub fn scroll_up(&mut self, region: &std::ops::Range<Line>, lines: usize, history_size: usize) {
        let lines = lines as i32;
        let full_screen = region.start == Line(0);
        self.retain(|placement| {
            let outside_region = if full_screen {
                placement.anchor.line >= region.end
            } else {
                placement.anchor.line < region.start || placement.anchor.line >= region.end
            };
            if outside_region {
                return true;
            }
            placement.anchor.line -= lines;
            if full_screen {
                placement.anchor.line >= Line(-(history_size as i32))
            } else {
                placement.anchor.line >= region.start
            }
        });
    }

    pub fn scroll_down(&mut self, region: &std::ops::Range<Line>, lines: usize) {
        let lines = lines as i32;
        self.retain(|placement| {
            if placement.anchor.line < region.start || placement.anchor.line >= region.end {
                return true;
            }
            placement.anchor.line += lines;
            placement.anchor.line < region.end
        });
    }

    pub fn remove_matching(
        &mut self,
        mut matches: impl FnMut(&Placement) -> bool,
    ) -> Vec<ImageHandle> {
        let handles: Vec<_> = self
            .entries
            .values()
            .filter(|placement| matches(placement))
            .map(|placement| placement.handle)
            .collect();
        let mut images = Vec::with_capacity(handles.len());
        for handle in handles {
            if let Some(placement) = self.entries.get(&handle) {
                images.push(placement.image);
            }
            self.remove(handle);
        }
        images
    }

    pub fn image_is_placed(&self, image: ImageHandle) -> bool {
        self.entries.values().any(|placement| placement.image == image)
    }

    pub fn remove_image(&mut self, image: ImageHandle) {
        let handles: Vec<_> = self
            .entries
            .values()
            .filter(|placement| placement.image == image)
            .map(|placement| placement.handle)
            .collect();
        for handle in handles {
            self.remove(handle);
        }
    }

    pub fn insert(
        &mut self,
        image: ImageHandle,
        image_id: Option<NonZeroU32>,
        command: &Command,
        anchor: Point,
    ) -> Result<PlacementHandle, GraphicsError> {
        let placement_id = NonZeroU32::new(command.placement_id.unwrap_or(0));
        let named = image_id.zip(placement_id);
        let replaced = named.and_then(|key| self.named.get(&key).copied());
        if replaced.is_none() && self.entries.len() == MAX_PLACEMENTS_PER_BUFFER {
            return Err(GraphicsError::NoSpace);
        }
        if let Some(handle) = replaced {
            self.remove(handle);
        }

        let handle = PlacementHandle(self.next_handle);
        self.next_handle = self.next_handle.checked_add(1).ok_or(GraphicsError::TooLarge)?;
        self.serial = self.serial.checked_add(1).ok_or(GraphicsError::TooLarge)?;
        let placement = Placement {
            handle,
            image,
            image_id,
            placement_id,
            anchor,
            source_x: command.x.unwrap_or(0),
            source_y: command.y.unwrap_or(0),
            source_width: NonZeroU32::new(command.crop_width.unwrap_or(0)).map(NonZeroU32::get),
            source_height: NonZeroU32::new(command.crop_height.unwrap_or(0)).map(NonZeroU32::get),
            x_offset: command.x_offset.unwrap_or(0),
            y_offset: command.y_offset.unwrap_or(0),
            columns: NonZeroU32::new(command.columns.unwrap_or(0)).map(NonZeroU32::get),
            rows: NonZeroU32::new(command.rows.unwrap_or(0)).map(NonZeroU32::get),
            z_index: command.z_index.unwrap_or(0),
            creation_serial: self.serial,
        };
        self.entries.insert(handle, placement);
        if let Some(key) = named {
            self.named.insert(key, handle);
        }
        Ok(handle)
    }

    fn retain(&mut self, mut keep: impl FnMut(&mut Placement) -> bool) {
        let removed: Vec<_> = self
            .entries
            .iter_mut()
            .filter_map(|(handle, placement)| (!keep(placement)).then_some(*handle))
            .collect();
        for handle in removed {
            self.remove(handle);
        }
    }

    fn remove(&mut self, handle: PlacementHandle) {
        if let Some(placement) = self.entries.remove(&handle) {
            if let Some(key) = placement.image_id.zip(placement.placement_id) {
                self.named.remove(&key);
            }
        }
    }
}
