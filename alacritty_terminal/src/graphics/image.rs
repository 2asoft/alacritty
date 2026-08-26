use std::io::{Cursor, Read};
use std::sync::Arc;

use base64::engine::general_purpose::STANDARD as Base64;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
use base64::read::DecoderReader;
use base64::{Engine, alphabet};
use miniz_oxide::inflate::decompress_to_vec_zlib_with_limit;
use png::{ColorType, Decoder, Limits, Transformations};

use super::{
    Action, Command, Compression, EncodedPayload, EncodedReader, Format, GraphicsError,
    ProcessedCommand, Transmission, load_transport,
};

const MAX_PNG_DECODER_OVERHEAD: usize = 1024 * 1024;
const DIRECT_BASE64: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PixelBuffer {
    width: u32,
    height: u32,
    // Share the owner without copying a complete pixel allocation into an Arc slice.
    // Completed buffers have capacity equal to length and expose immutable bytes only.
    bytes: Arc<Vec<u8>>,
}

impl PixelBuffer {
    pub(crate) fn new_rgba(width: u32, height: u32, bytes: Vec<u8>) -> Result<Self, GraphicsError> {
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
        let bytes = Arc::new(bytes.into_boxed_slice().into_vec());
        Ok(Self { width, height, bytes })
    }

    #[cfg(test)]
    pub(crate) fn from_rgba(width: u32, height: u32, bytes: Arc<[u8]>) -> Self {
        Self { width, height, bytes: Arc::new(bytes.to_vec()) }
    }

    pub(crate) fn into_rgba(self) -> Vec<u8> {
        Arc::try_unwrap(self.bytes).unwrap_or_else(|shared| shared.as_ref().clone())
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

pub(crate) fn process_command(
    command: Command,
    payload: impl Into<EncodedPayload>,
    storage_limit: usize,
    local_transmission: bool,
) -> ProcessedCommand {
    if command.image_id.is_some() && command.image_number.is_some() {
        return ProcessedCommand::Error { command: Some(command), error: GraphicsError::Invalid };
    }

    let payload = payload.into();
    let action = command.action.unwrap_or_default();
    if !matches!(
        action,
        Action::Transmit | Action::TransmitAndPlace | Action::Query | Action::TransmitFrame
    ) || command.more == Some(true)
        && command.transmission.unwrap_or_default() == Transmission::Direct
    {
        return ProcessedCommand::Metadata(command);
    }

    let source = match command.transmission.unwrap_or_default() {
        Transmission::Direct => direct_decode_policy(&command, storage_limit).and_then(|policy| {
            decode_base64(payload, &DIRECT_BASE64, policy, GraphicsError::Decode)
        }),
        _ if !local_transmission => Err(GraphicsError::LocalTransmissionDisabled),
        transmission => decode_local_name(payload)
            .and_then(|name| load_transport(transmission, name, &command, storage_limit)),
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

#[derive(Clone, Copy)]
enum DecodeOverflow {
    Error(GraphicsError),
    Truncate,
}

#[derive(Clone, Copy)]
struct DecodePolicy {
    limit: usize,
    overflow: DecodeOverflow,
}

fn direct_decode_policy(
    command: &Command,
    storage_limit: usize,
) -> Result<DecodePolicy, GraphicsError> {
    if command.compression.is_some() || command.format == Some(Format::Png) {
        let limit =
            storage_limit.checked_add(MAX_PNG_DECODER_OVERHEAD).ok_or(GraphicsError::TooLarge)?;
        return Ok(DecodePolicy { limit, overflow: DecodeOverflow::Error(GraphicsError::NoSpace) });
    }

    let channels = match command.format.unwrap_or_default() {
        Format::Rgb => 3,
        Format::Rgba => 4,
        Format::Png => return Err(GraphicsError::Invalid),
        Format::Unknown(_) => {
            return Ok(DecodePolicy {
                limit: storage_limit,
                overflow: DecodeOverflow::Error(GraphicsError::Invalid),
            });
        },
    };
    let source_size = raw_size(command, channels)?;
    let canonical_size = raw_size(command, 4)?;
    if canonical_size > storage_limit {
        return Ok(DecodePolicy {
            limit: storage_limit,
            overflow: DecodeOverflow::Error(GraphicsError::NoSpace),
        });
    }
    let overflow = if command.action == Some(Action::TransmitFrame) {
        DecodeOverflow::Truncate
    } else {
        DecodeOverflow::Error(GraphicsError::Invalid)
    };
    Ok(DecodePolicy { limit: source_size, overflow })
}

fn decode_local_name(payload: EncodedPayload) -> Result<Vec<u8>, GraphicsError> {
    let EncodedPayload::Single(payload) = payload else {
        return Err(GraphicsError::Invalid);
    };
    Base64.decode(payload).map_err(|_| GraphicsError::Invalid)
}

fn decode_base64<E: Engine>(
    payload: EncodedPayload,
    engine: &E,
    policy: DecodePolicy,
    invalid: GraphicsError,
) -> Result<Vec<u8>, GraphicsError> {
    let read_limit = policy.limit.checked_add(1).ok_or(GraphicsError::TooLarge)?;
    // Base64 cannot emit more than three bytes per encoded quartet. Grow as encoded
    // blocks are consumed, with a capacity ceiling derived from the actual input.
    let capacity_bound = payload
        .encoded_len()?
        .div_ceil(4)
        .checked_mul(3)
        .and_then(|size| size.checked_add(1))
        .ok_or(GraphicsError::TooLarge)?
        .min(read_limit);
    let reader = EncodedReader::from(payload);
    let mut decoder = DecoderReader::new(reader, engine);
    let mut decoded = Vec::new();
    while decoded.len() < capacity_bound {
        if decoded.len() == decoded.capacity() {
            let capacity = decoded.capacity().saturating_mul(2).max(8192).min(capacity_bound);
            decoded.reserve_exact(capacity - decoded.len());
        }
        let start = decoded.len();
        let end = start.saturating_add(8192).min(decoded.capacity()).min(capacity_bound);
        decoded.resize(end, 0);
        let count = decoder.read(&mut decoded[start..]).map_err(|_| invalid)?;
        decoded.truncate(start + count);
        if count == 0 {
            break;
        }
    }
    if decoded.len() <= policy.limit {
        return Ok(decoded);
    }

    std::io::copy(&mut decoder, &mut std::io::sink()).map_err(|_| invalid)?;
    match policy.overflow {
        DecodeOverflow::Error(error) => Err(error),
        DecodeOverflow::Truncate => {
            decoded.truncate(policy.limit);
            Ok(decoded)
        },
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
            .map_err(|_| GraphicsError::Decode)?
            .into_boxed_slice()
            .into_vec();
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
    PixelBuffer::new_rgba(width, height, bytes)
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
    PixelBuffer::new_rgba(width, height, source)
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

    let mut bytes = if info.color_type == ColorType::Rgba {
        Vec::new()
    } else {
        Vec::with_capacity(canonical_size)
    };
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
    PixelBuffer::new_rgba(width, height, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn freezing_pixels_transfers_allocation_and_discards_spare_capacity() {
        let bytes = vec![255; 4];
        let pointer = bytes.as_ptr();
        let pixels = PixelBuffer::new_rgba(1, 1, bytes).unwrap();
        assert_eq!(pixels.bytes().as_ptr(), pointer);
        assert_eq!(pixels.bytes.capacity(), pixels.storage_bytes());
        let bytes = pixels.into_rgba();
        assert_eq!(bytes.as_ptr(), pointer);

        let mut spare = Vec::with_capacity(1024);
        spare.extend_from_slice(&[1, 2, 3, 4]);
        let pixels = PixelBuffer::new_rgba(1, 1, spare).unwrap();
        assert_eq!(pixels.bytes.capacity(), 4);
        assert_eq!(pixels.bytes(), &[1, 2, 3, 4]);
    }

    #[derive(Clone)]
    struct TestCommand {
        command: Command,
        payload: Vec<u8>,
    }

    impl std::ops::Deref for TestCommand {
        type Target = Command;

        fn deref(&self) -> &Self::Target {
            &self.command
        }
    }

    impl std::ops::DerefMut for TestCommand {
        fn deref_mut(&mut self) -> &mut Self::Target {
            &mut self.command
        }
    }

    fn direct(format: Format, width: u32, height: u32, bytes: &[u8]) -> TestCommand {
        TestCommand {
            command: Command {
                format: Some(format),
                width: Some(width),
                height: Some(height),
                ..Default::default()
            },
            payload: Base64.encode(bytes).into_bytes(),
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

    fn decoded(input: TestCommand, limit: usize) -> Result<PixelBuffer, GraphicsError> {
        match process_command(input.command, input.payload, limit, true) {
            ProcessedCommand::Decoded { image, .. } => Ok(image),
            ProcessedCommand::Error { error, .. } => Err(error),
            ProcessedCommand::Metadata(_) => panic!("expected decoded image"),
        }
    }

    #[test]
    fn applies_default_transmit_rgba_and_requires_raw_dimensions() {
        let command = direct(Format::Rgba, 1, 1, &[1, 2, 3, 4]);
        assert_eq!(decoded(command, 4).unwrap().bytes(), &[1, 2, 3, 4]);
        assert_eq!(
            decoded(
                TestCommand {
                    command: Command::default(),
                    payload: Base64.encode([1, 2, 3, 4]).into_bytes(),
                },
                4
            ),
            Err(GraphicsError::Invalid)
        );
    }

    #[test]
    fn local_transport_gate_never_blocks_direct_payloads() {
        let direct = direct(Format::Rgba, 1, 1, &[1, 2, 3, 4]);
        assert!(matches!(
            process_command(direct.command, direct.payload, 4, false),
            ProcessedCommand::Decoded { .. }
        ));
        let file = Command {
            transmission: Some(Transmission::File),
            format: Some(Format::Rgba),
            width: Some(1),
            height: Some(1),
            ..Default::default()
        };
        assert!(matches!(
            process_command(file, Base64.encode("/tmp/image").into_bytes(), 4, false),
            ProcessedCommand::Error { error: GraphicsError::LocalTransmissionDisabled, .. }
        ));
    }

    #[test]
    fn malformed_local_transport_name_retains_einval() {
        let command = Command {
            transmission: Some(Transmission::File),
            format: Some(Format::Rgba),
            width: Some(1),
            height: Some(1),
            ..Default::default()
        };
        for payload in [b"%%%".as_slice(), b"L3RtcC9pbWFnZQ".as_slice()] {
            assert!(matches!(
                process_command(command.clone(), payload.to_vec(), 4, true),
                ProcessedCommand::Error { error: GraphicsError::Invalid, .. }
            ));
        }
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
    fn accepts_unpadded_base64_from_kitty_clients() {
        let mut command = direct(Format::Rgba, 1, 1, &[1, 2, 3, 4]);
        command.payload.truncate(command.payload.len() - 2);
        let image = decoded(command, 4).unwrap();
        assert_eq!(image.bytes(), &[1, 2, 3, 4]);
    }

    #[test]
    fn rejects_conflicting_image_identifiers() {
        let command = Command {
            action: Some(Action::Delete),
            image_id: Some(1),
            image_number: Some(2),
            ..Default::default()
        };

        assert!(matches!(process_command(command, Vec::new(), 4, true), ProcessedCommand::Error {
            error: GraphicsError::Invalid,
            ..
        }));
    }

    #[test]
    fn zero_image_identifiers_retain_anonymous_defaults() {
        for (image_id, image_number) in [(Some(0), None), (None, Some(0))] {
            let mut command = direct(Format::Rgba, 1, 1, &[1, 2, 3, 4]);
            command.image_id = image_id;
            command.image_number = image_number;
            assert!(matches!(
                process_command(command.command, command.payload, 4, true),
                ProcessedCommand::Decoded { .. }
            ));
        }
    }

    #[test]
    fn non_direct_transports_ignore_chunk_flag_and_attempt_loading() {
        for transmission in
            [Transmission::File, Transmission::TemporaryFile, Transmission::SharedMemory]
        {
            let command = Command {
                transmission: Some(transmission),
                format: Some(Format::Rgba),
                width: Some(1),
                height: Some(1),
                more: Some(true),
                ..Default::default()
            };
            assert!(matches!(
                process_command(
                    command,
                    Base64.encode("/alacritty-kitty-missing-object").into_bytes(),
                    4,
                    true,
                ),
                ProcessedCommand::Error { .. }
            ));
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
    fn classifies_excess_raw_data_without_retaining_it() {
        let excess = vec![7; 1024 * 1024];
        assert_eq!(decoded(direct(Format::Rgba, 1, 1, &excess), 4), Err(GraphicsError::Invalid));

        let mut frame = direct(Format::Rgba, 1, 1, &excess);
        frame.action = Some(Action::TransmitFrame);
        assert_eq!(decoded(frame, 4).unwrap().bytes(), &[7, 7, 7, 7]);
    }

    #[test]
    fn invalid_base64_after_excess_data_keeps_decode_precedence() {
        let input = TestCommand {
            command: Command {
                format: Some(Format::Rgba),
                width: Some(1),
                height: Some(1),
                ..Default::default()
            },
            payload: b"AQIDBAUG%%%".to_vec(),
        };
        assert_eq!(decoded(input, 4), Err(GraphicsError::Decode));
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
    fn decodes_interlaced_png_fixtures() {
        for source in [
            include_bytes!("../../tests/fixtures/kitty/interlaced.png").as_slice(),
            include_bytes!("../../tests/fixtures/kitty/interlaced-rgb.png").as_slice(),
        ] {
            let decoded = decode_png(source, 16).unwrap();
            assert_eq!((decoded.width(), decoded.height(), decoded.storage_bytes()), (2, 2, 16));
        }
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
