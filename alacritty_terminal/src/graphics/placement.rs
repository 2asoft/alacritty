use std::collections::HashMap;
use std::num::NonZeroU32;

use crate::index::{Line, Point};

use super::{Command, GraphicsError, ImageHandle, PixelBuffer};

pub const MAX_PLACEMENTS_PER_BUFFER: usize = 65_536;
pub const MAX_RELATIVE_PLACEMENT_DEPTH: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PlacementHandle(pub(crate) u64);

#[derive(Debug)]
pub(crate) struct PlacementInsert {
    pub handle: PlacementHandle,
    pub orphaned_relative_images: Vec<ImageHandle>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RelativePlacement {
    pub parent: PlacementHandle,
    pub horizontal_cells: i32,
    pub vertical_cells: i32,
}

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
    pub(crate) cell_span: (u32, u32),
    pub(crate) z_index: i32,
    pub(crate) virtual_placement: bool,
    pub(crate) relative: Option<RelativePlacement>,
    pub(crate) clip_region: Option<(Line, Line)>,
    pub(crate) creation_serial: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ResolvedPlacement {
    pub line: Line,
    pub column: i32,
    pub clip_region: Option<(Line, Line)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderableGraphic {
    pub image: ImageHandle,
    pub pixels: PixelBuffer,
    pub line: Line,
    pub column: i32,
    pub source_x: u32,
    pub source_y: u32,
    pub source_width: Option<u32>,
    pub source_height: Option<u32>,
    pub x_offset: u32,
    pub y_offset: u32,
    pub columns: Option<u32>,
    pub rows: Option<u32>,
    pub z_index: i32,
    pub image_id: u32,
    pub content_generation: u64,
    pub creation_serial: u64,
    pub clip_region: Option<(Line, Line)>,
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

    pub fn is_virtual(&self) -> bool {
        self.virtual_placement
    }
}

#[derive(Clone, Debug, Default)]
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

    pub fn tracked_anchors(&self) -> Vec<(PlacementHandle, Point)> {
        self.entries
            .values()
            .filter(|placement| !placement.virtual_placement && placement.relative.is_none())
            .map(|placement| (placement.handle, placement.anchor))
            .collect()
    }

    pub fn update_tracked_anchors(
        &mut self,
        anchors: &HashMap<PlacementHandle, Point>,
    ) -> Vec<ImageHandle> {
        let removed: Vec<_> = self
            .entries
            .values_mut()
            .filter(|placement| !placement.virtual_placement && placement.relative.is_none())
            .filter_map(|placement| match anchors.get(&placement.handle) {
                Some(anchor) => {
                    placement.anchor = *anchor;
                    None
                },
                None => Some(placement.handle),
            })
            .collect();
        let mut relative_images = Vec::new();
        for handle in removed {
            relative_images.extend(self.remove(handle));
        }
        relative_images
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.named.clear();
    }

    pub fn resolved_anchor(
        &self,
        placement: &Placement,
        virtual_origin: &impl Fn(u32, u32) -> Option<Point>,
    ) -> Option<ResolvedPlacement> {
        let mut root = placement;
        let mut horizontal_cells = 0i32;
        let mut vertical_cells = 0i32;
        let mut depth = 0;
        while let Some(location) = root.relative {
            depth += 1;
            if depth > MAX_RELATIVE_PLACEMENT_DEPTH {
                return None;
            }
            horizontal_cells = horizontal_cells.checked_add(location.horizontal_cells)?;
            vertical_cells = vertical_cells.checked_add(location.vertical_cells)?;
            root = self.entries.get(&location.parent)?;
        }
        let root_anchor = if root.virtual_placement {
            virtual_origin(root.image_id?.get(), root.placement_id?.get())?
        } else {
            root.anchor
        };
        Some(ResolvedPlacement {
            line: Line(root_anchor.line.0.checked_add(vertical_cells)?),
            column: i32::try_from(root_anchor.column.0).ok()?.checked_add(horizontal_cells)?,
            clip_region: root.clip_region,
        })
    }

    pub fn scroll_up(
        &mut self,
        region: &std::ops::Range<Line>,
        lines: usize,
        history_size: usize,
        whole_screen: bool,
    ) -> Vec<ImageHandle> {
        let lines = lines as i32;
        let creates_history = region.start == Line(0);
        self.retain(|placement| {
            if placement.relative.is_some() {
                return true;
            }
            let bottom = i64::from(placement.anchor.line.0) + i64::from(placement.cell_span.1);
            let contained =
                placement.anchor.line >= region.start && bottom <= i64::from(region.end.0);
            let clipped_to_region = placement.clip_region == Some((region.start, region.end));
            if if whole_screen {
                placement.anchor.line >= region.end
            } else {
                !contained && !clipped_to_region
            } {
                return true;
            }
            placement.anchor.line -= lines;
            if !whole_screen {
                placement.clip_region = Some((region.start, region.end));
            }
            if creates_history {
                placement.anchor.line.0 >= -(history_size as i32)
            } else {
                i64::from(placement.anchor.line.0) + i64::from(placement.cell_span.1)
                    > i64::from(region.start.0)
            }
        })
    }

    pub fn scroll_down(
        &mut self,
        region: &std::ops::Range<Line>,
        lines: usize,
        whole_screen: bool,
    ) -> Vec<ImageHandle> {
        let lines = lines as i32;
        self.retain(|placement| {
            if placement.relative.is_some() {
                return true;
            }
            let bottom = i64::from(placement.anchor.line.0) + i64::from(placement.cell_span.1);
            let contained =
                placement.anchor.line >= region.start && bottom <= i64::from(region.end.0);
            let clipped_to_region = placement.clip_region == Some((region.start, region.end));
            if !contained && !clipped_to_region {
                return true;
            }
            placement.anchor.line += lines;
            if !whole_screen {
                placement.clip_region = Some((region.start, region.end));
            }
            placement.anchor.line < region.end
        })
    }

    pub fn remove_matching(
        &mut self,
        mut matches: impl FnMut(&Placement) -> bool,
    ) -> (Vec<ImageHandle>, Vec<ImageHandle>) {
        let handles: Vec<_> = self
            .entries
            .values()
            .filter(|placement| matches(placement))
            .map(|placement| placement.handle)
            .collect();
        let mut selected_images = Vec::with_capacity(handles.len());
        let mut relative_images = Vec::new();
        for handle in handles {
            if let Some(placement) = self.entries.get(&handle) {
                selected_images.push(placement.image);
            }
            relative_images.extend(self.remove(handle));
        }
        (selected_images, relative_images)
    }

    pub fn image_is_placed(&self, image: ImageHandle) -> bool {
        self.entries.values().any(|placement| placement.image == image)
    }

    pub fn remove_image(&mut self, image: ImageHandle) -> Vec<ImageHandle> {
        let handles: Vec<_> = self
            .entries
            .values()
            .filter(|placement| placement.image == image)
            .map(|placement| placement.handle)
            .collect();
        let mut relative_images = Vec::new();
        for handle in handles {
            relative_images.extend(self.remove(handle));
        }
        relative_images
    }

    pub fn insert(
        &mut self,
        image: ImageHandle,
        image_id: Option<NonZeroU32>,
        command: &Command,
        anchor: Point,
        cell_span: (u32, u32),
    ) -> Result<PlacementInsert, GraphicsError> {
        let placement_id = NonZeroU32::new(command.placement_id.unwrap_or(0));
        let named = image_id.zip(placement_id);
        let replaced = named.and_then(|key| self.named.get(&key).copied());
        let relative = match (
            NonZeroU32::new(command.parent_image_id.unwrap_or(0)),
            NonZeroU32::new(command.parent_placement_id.unwrap_or(0)),
        ) {
            (None, None) => None,
            (Some(parent_image), Some(parent_placement)) => {
                if command.unicode_placeholder == Some(1) {
                    return Err(GraphicsError::Invalid);
                }
                let parent = self
                    .named
                    .get(&(parent_image, parent_placement))
                    .copied()
                    .ok_or(GraphicsError::NoParent)?;
                let mut ancestor = Some(parent);
                let mut depth = 0;
                while let Some(handle) = ancestor {
                    if Some(handle) == replaced {
                        return Err(GraphicsError::Cycle);
                    }
                    depth += 1;
                    if depth > MAX_RELATIVE_PLACEMENT_DEPTH {
                        return Err(GraphicsError::TooDeep);
                    }
                    ancestor = self
                        .entries
                        .get(&handle)
                        .and_then(|placement| placement.relative.map(|relative| relative.parent));
                }
                Some(RelativePlacement {
                    parent,
                    horizontal_cells: command.horizontal_offset.unwrap_or(0),
                    vertical_cells: command.vertical_offset.unwrap_or(0),
                })
            },
            _ => return Err(GraphicsError::Invalid),
        };
        if replaced.is_none() && self.entries.len() == MAX_PLACEMENTS_PER_BUFFER {
            return Err(GraphicsError::NoSpace);
        }
        let orphaned_relative_images = replaced.map_or_else(Vec::new, |handle| self.remove(handle));

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
            cell_span,
            z_index: command.z_index.unwrap_or(0),
            virtual_placement: command.unicode_placeholder == Some(1),
            relative,
            clip_region: None,
            creation_serial: self.serial,
        };
        self.entries.insert(handle, placement);
        if let Some(key) = named {
            self.named.insert(key, handle);
        }
        Ok(PlacementInsert { handle, orphaned_relative_images })
    }

    fn retain(&mut self, mut keep: impl FnMut(&mut Placement) -> bool) -> Vec<ImageHandle> {
        let removed: Vec<_> = self
            .entries
            .iter_mut()
            .filter_map(|(handle, placement)| (!keep(placement)).then_some(*handle))
            .collect();
        let mut relative_images = Vec::new();
        for handle in removed {
            relative_images.extend(self.remove(handle));
        }
        relative_images
    }

    fn remove(&mut self, handle: PlacementHandle) -> Vec<ImageHandle> {
        let Some(placement) = self.entries.remove(&handle) else {
            return Vec::new();
        };
        if let Some(key) = placement.image_id.zip(placement.placement_id) {
            self.named.remove(&key);
        }
        let children: Vec<_> = self
            .entries
            .values()
            .filter(|candidate| {
                candidate.relative.is_some_and(|relative| relative.parent == handle)
            })
            .map(|candidate| candidate.handle)
            .collect();
        let mut relative_images = Vec::with_capacity(children.len());
        for child in children {
            if let Some(placement) = self.entries.get(&child) {
                relative_images.push(placement.image);
            }
            relative_images.extend(self.remove(child));
        }
        relative_images
    }
}
