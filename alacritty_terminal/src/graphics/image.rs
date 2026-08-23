use std::io::Cursor;
use std::sync::Arc;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as Base64;
use miniz_oxide::inflate::decompress_to_vec_zlib_with_limit;
use png::{ColorType, Decoder, Limits, Transformations};

use super::{Action, Command, Compression, Format, GraphicsError, Transmission, load_transport};

const MAX_PNG_DECODER_OVERHEAD: usize = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PixelBuffer {
    width: u32,
    height: u32,
    bytes: Arc<[u8]>,
}

impl PixelBuffer {
    pub(crate) fn new_rgba(
        width: u32,
        height: u32,
        bytes: Arc<[u8]>,
    ) -> Result<Self, GraphicsError> {
        let expected = usize::try_from(width)
            .ok()
            .and_then(|width| {
                usize::try_from(height).ok().and_then(|height| width.checked_mul(height))
            })
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or(GraphicsError::TooLarge)?;
        if bytes.len() != expected {
            return Err(GraphicsError::Invalid);
        }
        Ok(Self { width, height, bytes })
    }

    #[cfg(test)]
    pub(crate) fn from_rgba(width: u32, height: u32, bytes: Arc<[u8]>) -> Self {
        Self { width, height, bytes }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn storage_bytes(&self) -> usize {
        self.bytes.len()
    }
}

#[derive(Debug)]
pub enum ProcessedCommand {
    Decoded { command: Command, image: PixelBuffer },
    Metadata(Command),
    Error { command: Option<Command>, error: GraphicsError },
}

impl ProcessedCommand {
    pub fn set_anchor(&mut self, anchor: Option<crate::index::Point>) {
        let command = match self {
            Self::Decoded { command, .. } | Self::Metadata(command) => Some(command),
            Self::Error { command, .. } => command.as_mut(),
        };
        if let Some(command) = command {
            command.anchor = anchor.or(command.anchor);
        }
    }
}

pub fn process_command(
    command: Result<Command, GraphicsError>,
    storage_limit: usize,
    local_transmission: bool,
) -> ProcessedCommand {
    let command = match command {
        Ok(command)
            if command.image_id.is_some() && command.image_number.is_some()
                || command.image_id == Some(0)
                || command.image_number == Some(0) =>
        {
            return ProcessedCommand::Error {
                command: Some(command),
                error: GraphicsError::Invalid,
            };
        },
        Ok(command) => command,
        Err(error) => return ProcessedCommand::Error { command: None, error },
    };

    let action = command.action.unwrap_or_default();
    if !matches!(
        action,
        Action::Transmit | Action::TransmitAndPlace | Action::Query | Action::TransmitFrame
    ) || command.more == Some(true)
    {
        return ProcessedCommand::Metadata(command);
    }

    let source = match command.transmission.unwrap_or_default() {
        Transmission::Direct => Base64.decode(&command.payload).map_err(|_| GraphicsError::Decode),
        _ if !local_transmission => Err(GraphicsError::LocalTransmissionDisabled),
        transmission => load_transport(transmission, &command, storage_limit),
    };
    let source = match source {
        Ok(source) => source,
        Err(error) => return ProcessedCommand::Error { command: Some(command), error },
    };

    match decode_source(&command, source, storage_limit) {
        Ok(image) => ProcessedCommand::Decoded { command, image },
        Err(error) => ProcessedCommand::Error { command: Some(command), error },
    }
}

fn decode_source(
    command: &Command,
    mut source: Vec<u8>,
    storage_limit: usize,
) -> Result<PixelBuffer, GraphicsError> {
    if command.compression == Some(Compression::Zlib) {
        let decompressed_limit = match command.format.unwrap_or_default() {
            Format::Rgb => raw_size(command, 3)?,
            Format::Rgba => raw_size(command, 4)?,
            Format::Png => usize::try_from(command.data_size.ok_or(GraphicsError::Invalid)?)
                .map_err(|_| GraphicsError::TooLarge)?,
            Format::Unknown(_) => return Err(GraphicsError::Invalid),
        };
        let source_limit = match command.format.unwrap_or_default() {
            Format::Png => storage_limit.saturating_add(MAX_PNG_DECODER_OVERHEAD),
            _ => storage_limit,
        };
        if decompressed_limit > source_limit {
            return Err(GraphicsError::NoSpace);
        }
        source = decompress_to_vec_zlib_with_limit(&source, decompressed_limit)
            .map_err(|_| GraphicsError::Decode)?;
        if source.len() != decompressed_limit {
            return Err(GraphicsError::Invalid);
        }
    }

    match command.format.unwrap_or_default() {
        Format::Rgb => decode_rgb(command, source, storage_limit),
        Format::Rgba => decode_rgba(command, source, storage_limit),
        Format::Png => decode_png(&source, storage_limit),
        Format::Unknown(_) => Err(GraphicsError::Invalid),
    }
}

fn dimensions(command: &Command) -> Result<(u32, u32), GraphicsError> {
    let width = command.width.filter(|width| *width != 0).ok_or(GraphicsError::Invalid)?;
    let height = command.height.filter(|height| *height != 0).ok_or(GraphicsError::Invalid)?;
    Ok((width, height))
}

fn raw_size(command: &Command, channels: usize) -> Result<usize, GraphicsError> {
    let (width, height) = dimensions(command)?;
    usize::try_from(width)
        .ok()
        .and_then(|width| usize::try_from(height).ok().and_then(|height| width.checked_mul(height)))
        .and_then(|pixels| pixels.checked_mul(channels))
        .ok_or(GraphicsError::TooLarge)
}

fn decode_rgb(
    command: &Command,
    source: Vec<u8>,
    storage_limit: usize,
) -> Result<PixelBuffer, GraphicsError> {
    let (width, height) = dimensions(command)?;
    let source_size = raw_size(command, 3)?;
    let canonical_size = raw_size(command, 4)?;
    if source.len() < source_size {
        return Err(GraphicsError::NoData);
    }
    if source.len() > source_size && command.action != Some(Action::TransmitFrame) {
        return Err(GraphicsError::Invalid);
    }
    if canonical_size > storage_limit {
        return Err(GraphicsError::NoSpace);
    }

    let mut bytes = Vec::with_capacity(canonical_size);
    for pixel in source[..source_size].chunks_exact(3) {
        bytes.extend_from_slice(pixel);
        bytes.push(255);
    }
    Ok(PixelBuffer { width, height, bytes: bytes.into() })
}

fn decode_rgba(
    command: &Command,
    mut source: Vec<u8>,
    storage_limit: usize,
) -> Result<PixelBuffer, GraphicsError> {
    let (width, height) = dimensions(command)?;
    let canonical_size = raw_size(command, 4)?;
    if source.len() < canonical_size {
        return Err(GraphicsError::NoData);
    }
    if source.len() > canonical_size && command.action != Some(Action::TransmitFrame) {
        return Err(GraphicsError::Invalid);
    }
    if canonical_size > storage_limit {
        return Err(GraphicsError::NoSpace);
    }
    source.truncate(canonical_size);
    Ok(PixelBuffer { width, height, bytes: source.into() })
}

fn decode_png(source: &[u8], storage_limit: usize) -> Result<PixelBuffer, GraphicsError> {
    let limits = Limits { bytes: storage_limit.saturating_add(MAX_PNG_DECODER_OVERHEAD) };
    let mut decoder = Decoder::new_with_limits(Cursor::new(source), limits);
    decoder.set_ignore_text_chunk(true);
    decoder.set_transformations(Transformations::EXPAND | Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|_| GraphicsError::Decode)?;
    let (width, height) = reader.info().size();
    let canonical_size = usize::try_from(width)
        .ok()
        .and_then(|width| usize::try_from(height).ok().and_then(|height| width.checked_mul(height)))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(GraphicsError::TooLarge)?;
    if canonical_size > storage_limit {
        return Err(GraphicsError::NoSpace);
    }

    let output_size = reader.output_buffer_size();
    if output_size > storage_limit {
        return Err(GraphicsError::NoSpace);
    }
    let mut output = vec![0; output_size];
    let info = reader.next_frame(&mut output).map_err(|_| GraphicsError::Decode)?;
    output.truncate(info.buffer_size());

    let mut bytes = Vec::with_capacity(canonical_size);
    match info.color_type {
        ColorType::Grayscale => {
            for gray in output {
                bytes.extend_from_slice(&[gray, gray, gray, 255]);
            }
        },
        ColorType::GrayscaleAlpha => {
            for pixel in output.chunks_exact(2) {
                bytes.extend_from_slice(&[pixel[0], pixel[0], pixel[0], pixel[1]]);
            }
        },
        ColorType::Rgb => {
            for pixel in output.chunks_exact(3) {
                bytes.extend_from_slice(pixel);
                bytes.push(255);
            }
        },
        ColorType::Rgba => bytes = output,
        ColorType::Indexed => return Err(GraphicsError::Decode),
    }

    if bytes.len() != canonical_size {
        return Err(GraphicsError::Decode);
    }
    Ok(PixelBuffer { width, height, bytes: bytes.into() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn direct(format: Format, width: u32, height: u32, bytes: &[u8]) -> Command {
        Command {
            format: Some(format),
            width: Some(width),
            height: Some(height),
            payload: Base64.encode(bytes).into_bytes(),
            ..Default::default()
        }
    }

    fn png(color: ColorType, depth: png::BitDepth, data: &[u8]) -> Vec<u8> {
        let mut output = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut output, 1, 1);
            encoder.set_color(color);
            encoder.set_depth(depth);
            if color == ColorType::Indexed {
                encoder.set_palette(vec![10, 20, 30]);
                encoder.set_trns(vec![40]);
            }
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(data).unwrap();
        }
        output
    }

    fn decoded(command: Command, limit: usize) -> Result<PixelBuffer, GraphicsError> {
        match process_command(Ok(command), limit, true) {
            ProcessedCommand::Decoded { image, .. } => Ok(image),
            ProcessedCommand::Error { error, .. } => Err(error),
            ProcessedCommand::Metadata(_) => panic!("expected decoded image"),
        }
    }

    #[test]
    fn applies_default_transmit_rgba_and_requires_raw_dimensions() {
        let command = Command {
            payload: Base64.encode([1, 2, 3, 4]).into_bytes(),
            width: Some(1),
            height: Some(1),
            ..Default::default()
        };
        assert_eq!(decoded(command, 4).unwrap().bytes(), &[1, 2, 3, 4]);
        assert_eq!(
            decoded(
                Command { payload: Base64.encode([1, 2, 3, 4]).into_bytes(), ..Default::default() },
                4
            ),
            Err(GraphicsError::Invalid)
        );
    }

    #[test]
    fn converts_rgb_to_canonical_rgba() {
        let image = decoded(direct(Format::Rgb, 2, 1, &[1, 2, 3, 4, 5, 6]), 8).unwrap();
        assert_eq!(image.bytes(), &[1, 2, 3, 255, 4, 5, 6, 255]);
    }

    #[test]
    fn preserves_rgba() {
        let image = decoded(direct(Format::Rgba, 1, 1, &[1, 2, 3, 4]), 4).unwrap();
        assert_eq!(image.bytes(), &[1, 2, 3, 4]);
    }

    #[test]
    fn rejects_conflicting_and_zero_image_identifiers_for_metadata_commands() {
        for command in [
            Command {
                action: Some(Action::Delete),
                image_id: Some(1),
                image_number: Some(2),
                ..Default::default()
            },
            Command { action: Some(Action::Place), image_id: Some(0), ..Default::default() },
            Command { action: Some(Action::Animate), image_number: Some(0), ..Default::default() },
        ] {
            assert!(matches!(process_command(Ok(command), 4, true), ProcessedCommand::Error {
                error: GraphicsError::Invalid,
                ..
            }));
        }
    }

    #[test]
    fn rejects_wrong_raw_size_and_quota_overflow() {
        assert_eq!(decoded(direct(Format::Rgb, 1, 1, &[1, 2]), 4), Err(GraphicsError::NoData));
        assert_eq!(
            decoded(direct(Format::Rgb, 1, 1, &[1, 2, 3, 4]), 4),
            Err(GraphicsError::Invalid)
        );
        assert_eq!(
            decoded(direct(Format::Rgba, 1, 1, &[1, 2, 3, 4]), 3),
            Err(GraphicsError::NoSpace)
        );
    }

    #[test]
    fn truncates_excess_animation_frame_data() {
        let mut command = direct(Format::Rgb, 1, 1, &[1, 2, 3, 4, 5, 6]);
        command.action = Some(Action::TransmitFrame);
        assert_eq!(decoded(command, 4).unwrap().bytes(), &[1, 2, 3, 255]);
    }

    #[test]
    fn decompresses_bounded_zlib() {
        let compressed = miniz_oxide::deflate::compress_to_vec_zlib(&[1, 2, 3, 4], 6);
        let mut command = direct(Format::Rgba, 1, 1, &compressed);
        command.compression = Some(Compression::Zlib);
        let image = decoded(command, 4).unwrap();
        assert_eq!(image.bytes(), &[1, 2, 3, 4]);
    }

    #[test]
    fn decompresses_rgb_and_png_with_declared_png_size() {
        let rgb = miniz_oxide::deflate::compress_to_vec_zlib(&[1, 2, 3], 6);
        let mut command = direct(Format::Rgb, 1, 1, &rgb);
        command.compression = Some(Compression::Zlib);
        assert_eq!(decoded(command, 4).unwrap().bytes(), &[1, 2, 3, 255]);

        let source = png(ColorType::Rgba, png::BitDepth::Eight, &[1, 2, 3, 4]);
        let compressed = miniz_oxide::deflate::compress_to_vec_zlib(&source, 6);
        let mut command = direct(Format::Png, 0, 0, &compressed);
        command.compression = Some(Compression::Zlib);
        assert_eq!(decoded(command.clone(), 4), Err(GraphicsError::Invalid));
        command.data_size = Some(source.len() as u32);
        assert_eq!(decoded(command, 4).unwrap().bytes(), &[1, 2, 3, 4]);
    }

    #[test]
    fn normalizes_ordinary_png_color_types_and_depths() {
        for (color, depth, source, expected) in [
            (ColorType::Grayscale, png::BitDepth::Eight, vec![12], vec![12, 12, 12, 255]),
            (ColorType::GrayscaleAlpha, png::BitDepth::Eight, vec![12, 34], vec![12, 12, 12, 34]),
            (ColorType::Rgb, png::BitDepth::Eight, vec![1, 2, 3], vec![1, 2, 3, 255]),
            (ColorType::Rgba, png::BitDepth::Eight, vec![1, 2, 3, 4], vec![1, 2, 3, 4]),
            (ColorType::Indexed, png::BitDepth::Eight, vec![0], vec![10, 20, 30, 40]),
            (ColorType::Grayscale, png::BitDepth::One, vec![0], vec![0, 0, 0, 255]),
            (ColorType::Grayscale, png::BitDepth::Two, vec![0], vec![0, 0, 0, 255]),
            (ColorType::Grayscale, png::BitDepth::Four, vec![0], vec![0, 0, 0, 255]),
            (ColorType::Indexed, png::BitDepth::One, vec![0], vec![10, 20, 30, 40]),
            (ColorType::Indexed, png::BitDepth::Two, vec![0], vec![10, 20, 30, 40]),
            (ColorType::Indexed, png::BitDepth::Four, vec![0], vec![10, 20, 30, 40]),
            (ColorType::Grayscale, png::BitDepth::Sixteen, vec![128, 0], vec![128, 128, 128, 255]),
            (ColorType::GrayscaleAlpha, png::BitDepth::Sixteen, vec![128, 0, 64, 0], vec![
                128, 128, 128, 64,
            ]),
            (ColorType::Rgb, png::BitDepth::Sixteen, vec![1, 0, 2, 0, 3, 0], vec![1, 2, 3, 255]),
            (ColorType::Rgba, png::BitDepth::Sixteen, vec![1, 0, 2, 0, 3, 0, 4, 0], vec![
                1, 2, 3, 4,
            ]),
        ] {
            let decoded = decode_png(&png(color, depth, &source), 4)
                .unwrap_or_else(|error| panic!("failed {color:?} {depth:?}: {error:?}"));
            assert_eq!(decoded.bytes(), expected);
        }
    }

    #[test]
    fn decodes_interlaced_palette_png_fixture() {
        let source = include_bytes!("../../tests/fixtures/kitty/interlaced.png");
        let decoded = decode_png(source, 16).unwrap();
        assert_eq!((decoded.width(), decoded.height(), decoded.storage_bytes()), (2, 2, 16));
    }

    #[test]
    fn normalizes_png_to_rgba() {
        let mut png = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png, 1, 1);
            encoder.set_color(ColorType::GrayscaleAlpha);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&[42, 128]).unwrap();
        }
        let image = decoded(direct(Format::Png, 0, 0, &png), 4).unwrap();
        assert_eq!(image.bytes(), &[42, 42, 42, 128]);
    }
}
