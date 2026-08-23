use std::sync::Arc;

use super::{GraphicsError, PixelBuffer};

pub const DEFAULT_FRAME_GAP_MS: i32 = 40;
pub const MAX_FRAMES_PER_IMAGE: usize = 4096;

#[derive(Clone, Copy, Debug)]
pub(crate) struct FrameComposition {
    pub source_x: u32,
    pub source_y: u32,
    pub destination_x: u32,
    pub destination_y: u32,
    pub width: u32,
    pub height: u32,
    pub overwrite: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct AnimationFrame {
    pub pixels: PixelBuffer,
    pub gap_ms: i32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum AnimationState {
    #[default]
    Stopped,
    Loading,
    Running,
}

pub(crate) fn blank_frame(
    width: u32,
    height: u32,
    rgba: u32,
) -> Result<PixelBuffer, GraphicsError> {
    let pixels = usize::try_from(width)
        .ok()
        .and_then(|width| usize::try_from(height).ok().and_then(|height| width.checked_mul(height)))
        .ok_or(GraphicsError::TooLarge)?;
    let color = rgba.to_be_bytes();
    let bytes = pixels.checked_mul(4).ok_or(GraphicsError::TooLarge)?;
    let mut data = Vec::with_capacity(bytes);
    for _ in 0..pixels {
        data.extend_from_slice(&color);
    }
    PixelBuffer::new_rgba(width, height, Arc::from(data))
}

pub(crate) fn compose(
    destination: &PixelBuffer,
    source: &PixelBuffer,
    composition: FrameComposition,
) -> Result<PixelBuffer, GraphicsError> {
    let FrameComposition {
        source_x,
        source_y,
        destination_x,
        destination_y,
        width,
        height,
        overwrite,
    } = composition;
    let source_right = source_x.checked_add(width).ok_or(GraphicsError::TooLarge)?;
    let source_bottom = source_y.checked_add(height).ok_or(GraphicsError::TooLarge)?;
    let destination_right = destination_x.checked_add(width).ok_or(GraphicsError::TooLarge)?;
    let destination_bottom = destination_y.checked_add(height).ok_or(GraphicsError::TooLarge)?;
    if source_right > source.width()
        || source_bottom > source.height()
        || destination_right > destination.width()
        || destination_bottom > destination.height()
    {
        return Err(GraphicsError::Invalid);
    }

    let source_stride = usize::try_from(source.width())
        .ok()
        .and_then(|width| width.checked_mul(4))
        .ok_or(GraphicsError::TooLarge)?;
    let destination_stride = usize::try_from(destination.width())
        .ok()
        .and_then(|width| width.checked_mul(4))
        .ok_or(GraphicsError::TooLarge)?;
    let mut output = destination.bytes().to_vec();
    for row in 0..height {
        for column in 0..width {
            let source_offset = usize::try_from(source_y + row)
                .ok()
                .and_then(|row| row.checked_mul(source_stride))
                .and_then(|offset| {
                    usize::try_from(source_x + column)
                        .ok()
                        .and_then(|column| column.checked_mul(4))
                        .and_then(|column| offset.checked_add(column))
                })
                .ok_or(GraphicsError::TooLarge)?;
            let destination_offset = usize::try_from(destination_y + row)
                .ok()
                .and_then(|row| row.checked_mul(destination_stride))
                .and_then(|offset| {
                    usize::try_from(destination_x + column)
                        .ok()
                        .and_then(|column| column.checked_mul(4))
                        .and_then(|column| offset.checked_add(column))
                })
                .ok_or(GraphicsError::TooLarge)?;
            let source_pixel = &source.bytes()[source_offset..source_offset + 4];
            let destination_pixel = &mut output[destination_offset..destination_offset + 4];
            if overwrite {
                destination_pixel.copy_from_slice(source_pixel);
            } else {
                alpha_blend(destination_pixel, source_pixel);
            }
        }
    }
    PixelBuffer::new_rgba(destination.width(), destination.height(), Arc::from(output))
}

fn alpha_blend(destination: &mut [u8], source: &[u8]) {
    let source_alpha = u32::from(source[3]);
    let destination_alpha = u32::from(destination[3]);
    let inverse_source = 255 - source_alpha;
    let alpha_numerator = source_alpha * 255 + destination_alpha * inverse_source;
    if alpha_numerator == 0 {
        destination.fill(0);
        return;
    }
    // Each channel is a weighted average of u8 values, so the final quotient is <= 255.
    for channel in 0..3 {
        let color_numerator = u32::from(source[channel]) * source_alpha * 255
            + u32::from(destination[channel]) * destination_alpha * inverse_source;
        destination[channel] = ((color_numerator + alpha_numerator / 2) / alpha_numerator) as u8;
    }
    destination[3] = ((alpha_numerator + 127) / 255) as u8;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composes_overwrite_and_alpha_pixels() {
        let destination = blank_frame(1, 1, 0xff0000ff).unwrap();
        let source = blank_frame(1, 1, 0x0000ff80).unwrap();
        let blended = compose(&destination, &source, FrameComposition {
            source_x: 0,
            source_y: 0,
            destination_x: 0,
            destination_y: 0,
            width: 1,
            height: 1,
            overwrite: false,
        })
        .unwrap();
        assert_eq!(blended.bytes(), &[127, 0, 128, 255]);
        let overwritten = compose(&destination, &source, FrameComposition {
            source_x: 0,
            source_y: 0,
            destination_x: 0,
            destination_y: 0,
            width: 1,
            height: 1,
            overwrite: true,
        })
        .unwrap();
        assert_eq!(overwritten.bytes(), source.bytes());
    }

    #[test]
    fn white_over_white_remains_white() {
        let mut destination = [255, 255, 255, 1];
        alpha_blend(&mut destination, &[255, 255, 255, 128]);
        assert_eq!(destination, [255, 255, 255, 128]);
    }

    #[test]
    fn white_over_white_remains_white_for_every_alpha() {
        for destination_alpha in 0..=255_u8 {
            for source_alpha in 0..=255_u8 {
                let mut destination = [255, 255, 255, destination_alpha];
                alpha_blend(&mut destination, &[255, 255, 255, source_alpha]);
                if destination[3] == 0 {
                    assert_eq!(destination, [0, 0, 0, 0]);
                } else {
                    assert_eq!(
                        destination,
                        [255, 255, 255, destination[3]],
                        "source alpha {source_alpha} over destination alpha {destination_alpha}"
                    );
                }
            }
        }
    }

    #[test]
    fn alpha_blend_representative_cases() {
        let mut unchanged = [10, 20, 30, 40];
        alpha_blend(&mut unchanged, &[9, 8, 7, 0]);
        assert_eq!(unchanged, [10, 20, 30, 40]);

        let mut replaced = [1, 2, 3, 4];
        alpha_blend(&mut replaced, &[9, 8, 7, 255]);
        assert_eq!(replaced, [9, 8, 7, 255]);

        let mut transparent = [255, 255, 255, 0];
        alpha_blend(&mut transparent, &[0, 0, 0, 0]);
        assert_eq!(transparent, [0, 0, 0, 0]);

        let mut black = [0, 0, 0, 255];
        alpha_blend(&mut black, &[255, 0, 0, 128]);
        assert_eq!(black, [128, 0, 0, 255]);
    }
}
