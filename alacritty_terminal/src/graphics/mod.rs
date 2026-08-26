mod animation;
mod deferred;
mod image;
mod parser;
mod placeholder;
mod placement;
mod rowcolumn_diacritics;
mod storage;
mod transaction;
mod transport;

pub(crate) use animation::{
    AnimationFrame, AnimationState, DEFAULT_FRAME_GAP_MS, FrameComposition, MAX_FRAMES_PER_BUFFER,
    MAX_FRAMES_PER_IMAGE, blank_frame, compose,
};
pub(crate) use deferred::{
    DeferredGraphics, PreparedGraphics, ProcessedCommand, ProcessingOptions,
};
pub use image::PixelBuffer;
pub(crate) use image::process_command;
#[cfg(test)]
pub(crate) use parser::DeleteTarget;
pub(crate) use parser::{
    Action, Command, Compression, Format, GraphicsApcParser, GraphicsError, ParsedCommand,
    Transmission,
};
#[cfg(test)]
pub(crate) use placeholder::PLACEHOLDER;
pub(crate) use placeholder::decode_placeholder;
pub use placement::RenderableGraphic;
pub(crate) use placement::{
    Placement, PlacementHandle, PlacementInsert, PlacementSpec, Placements,
};
pub use storage::ImageHandle;
pub(crate) use storage::{FrameCommit, FrameWork, GraphicsState, PreparedFrameMutation};
pub(crate) use transaction::{
    EncodedPayload, EncodedReader, GraphicsRequest, PendingResult, PendingTransmission,
    process_request,
};
pub(crate) use transport::load_transport;

use crate::index::Point;

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
