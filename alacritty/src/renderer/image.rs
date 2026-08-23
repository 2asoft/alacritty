use std::collections::HashMap;
use std::mem;

use alacritty_terminal::graphics::{ImageHandle, PixelBuffer};

use crate::gl;
use crate::gl::types::*;
use crate::renderer::Error;
use crate::renderer::shader::{ShaderProgram, ShaderVersion};

const IMAGE_SHADER_F: &str = include_str!("../../res/image.f.glsl");
const IMAGE_SHADER_V: &str = include_str!("../../res/image.v.glsl");
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TileRegion {
    core_x: u32,
    core_y: u32,
    core_width: u32,
    core_height: u32,
    upload_x: u32,
    upload_y: u32,
    upload_width: u32,
    upload_height: u32,
}

fn tile_regions(width: u32, height: u32, maximum: u32) -> Vec<TileRegion> {
    let border = u32::from(maximum > 2);
    let core_size = maximum.saturating_sub(border * 2).max(1);
    let mut regions = Vec::new();
    let mut core_y = 0;
    while core_y < height {
        let core_height = core_size.min(height - core_y);
        let mut core_x = 0;
        while core_x < width {
            let core_width = core_size.min(width - core_x);
            let upload_x = core_x.saturating_sub(border);
            let upload_y = core_y.saturating_sub(border);
            let upload_right = core_x.saturating_add(core_width).saturating_add(border).min(width);
            let upload_bottom =
                core_y.saturating_add(core_height).saturating_add(border).min(height);
            regions.push(TileRegion {
                core_x,
                core_y,
                core_width,
                core_height,
                upload_x,
                upload_y,
                upload_width: upload_right - upload_x,
                upload_height: upload_bottom - upload_y,
            });
            core_x += core_width;
        }
        core_y += core_height;
    }
    regions
}

fn intersect_scissors(left: [i32; 4], right: [i32; 4]) -> [i32; 4] {
    let x = left[0].max(right[0]);
    let y = left[1].max(right[1]);
    let right_edge = left[0].saturating_add(left[2]).min(right[0].saturating_add(right[2]));
    let top_edge = left[1].saturating_add(left[3]).min(right[1].saturating_add(right[3]));
    [x, y, right_edge.saturating_sub(x).max(0), top_edge.saturating_sub(y).max(0)]
}

fn texture_bytes(regions: &[TileRegion]) -> Option<usize> {
    regions.iter().try_fold(0usize, |total, region| {
        usize::try_from(region.upload_width)
            .ok()
            .and_then(|width| {
                usize::try_from(region.upload_height)
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .and_then(|pixels| pixels.checked_mul(4))
            .and_then(|bytes| total.checked_add(bytes))
    })
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ImageViewport {
    pub width: f32,
    pub height: f32,
    pub clip_x: f32,
    pub clip_y: f32,
    pub clip_width: f32,
    pub clip_height: f32,
}

#[derive(Clone, Debug)]
pub struct RenderableImage {
    pub image: ImageHandle,
    pub content_generation: u64,
    pub pixels: PixelBuffer,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub source_x: u32,
    pub source_y: u32,
    pub source_width: u32,
    pub source_height: u32,
    pub z_index: i32,
    pub image_id: u32,
    pub creation_serial: u64,
    pub clip_top: f32,
    pub clip_bottom: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct Vertex {
    x: f32,
    y: f32,
    texture_x: f32,
    texture_y: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct TextureKey {
    image: ImageHandle,
    generation: u64,
}

#[derive(Debug)]
struct TextureTile {
    texture: GLuint,
    region: TileRegion,
}

impl Drop for TextureTile {
    fn drop(&mut self) {
        // SAFETY: The texture name was created by this renderer in the current shared GL context.
        unsafe { gl::DeleteTextures(1, &self.texture) }
    }
}

#[derive(Debug)]
struct CachedImage {
    tiles: Vec<TextureTile>,
    bytes: usize,
    last_used: u64,
}

#[derive(Debug)]
pub(super) struct ImageRenderer {
    program: ShaderProgram,
    texture_uniform: GLint,
    vao: GLuint,
    vbo: GLuint,
    textures: HashMap<TextureKey, CachedImage>,
    texture_bytes: usize,
    usage_clock: u64,
    maximum_texture_size: u32,
}

impl ImageRenderer {
    pub(super) fn new(shader_version: ShaderVersion) -> Result<Self, Error> {
        let program = ShaderProgram::new(shader_version, None, IMAGE_SHADER_V, IMAGE_SHADER_F)?;
        let texture_uniform = program.get_uniform_location(c"imageTexture")?;
        let mut vao = 0;
        let mut vbo = 0;

        // SAFETY: A current GL context exists during renderer construction. Vertex offsets match
        // the `Vertex` C representation and remain valid while the VAO references `vbo`.
        unsafe {
            gl::GenVertexArrays(1, &mut vao);
            gl::GenBuffers(1, &mut vbo);
            gl::BindVertexArray(vao);
            gl::BindBuffer(gl::ARRAY_BUFFER, vbo);
            gl::VertexAttribPointer(
                0,
                2,
                gl::FLOAT,
                gl::FALSE,
                mem::size_of::<Vertex>() as i32,
                std::ptr::null(),
            );
            gl::EnableVertexAttribArray(0);
            gl::VertexAttribPointer(
                1,
                2,
                gl::FLOAT,
                gl::FALSE,
                mem::size_of::<Vertex>() as i32,
                (mem::size_of::<f32>() * 2) as *const _,
            );
            gl::EnableVertexAttribArray(1);
            gl::BindBuffer(gl::ARRAY_BUFFER, 0);
            gl::BindVertexArray(0);
        }

        let mut maximum_texture_size = 0;
        // SAFETY: A current GL context exists and the output points to valid writable storage.
        unsafe { gl::GetIntegerv(gl::MAX_TEXTURE_SIZE, &mut maximum_texture_size) };
        let maximum_texture_size = u32::try_from(maximum_texture_size).unwrap_or(1).clamp(1, 8192);

        Ok(Self {
            program,
            texture_uniform,
            vao,
            vbo,
            textures: HashMap::new(),
            texture_bytes: 0,
            usage_clock: 0,
            maximum_texture_size,
        })
    }

    pub(super) fn draw(
        &mut self,
        viewport: ImageViewport,
        images: &[RenderableImage],
        cache_limit: usize,
    ) {
        #[cfg(debug_assertions)]
        if std::env::var_os("ALACRITTY_TEST_DISCARD_IMAGE_TEXTURES").is_some() {
            self.textures.clear();
            self.texture_bytes = 0;
        }
        self.evict_for(0, cache_limit);
        if images.is_empty() || viewport.clip_width <= 0. || viewport.clip_height <= 0. {
            return;
        }

        let mut previous_scissor = [0; 4];
        // SAFETY: A current context exists and `previous_scissor` is writable state storage.
        let scissor_was_enabled = unsafe {
            let enabled = gl::IsEnabled(gl::SCISSOR_TEST) == gl::TRUE;
            gl::GetIntegerv(gl::SCISSOR_BOX, previous_scissor.as_mut_ptr());
            enabled
        };
        let content_scissor = [
            viewport.clip_x.floor() as i32,
            (viewport.height - viewport.clip_y - viewport.clip_height).floor() as i32,
            viewport.clip_width.ceil() as i32,
            viewport.clip_height.ceil() as i32,
        ];
        let scissor = if scissor_was_enabled {
            intersect_scissors(previous_scissor, content_scissor)
        } else {
            content_scissor
        };
        if scissor[2] <= 0 || scissor[3] <= 0 {
            return;
        }

        // SAFETY: All GL names are owned by this renderer and a current context is established by
        // `Display::draw`. Uploaded slices remain alive for each call consuming their pointers.
        unsafe {
            gl::Enable(gl::SCISSOR_TEST);
            gl::Scissor(scissor[0], scissor[1], scissor[2], scissor[3]);
            gl::UseProgram(self.program.id());
            gl::Uniform1i(self.texture_uniform, 0);
            gl::ActiveTexture(gl::TEXTURE0);
            gl::BindVertexArray(self.vao);
            gl::BindBuffer(gl::ARRAY_BUFFER, self.vbo);
            gl::Enable(gl::BLEND);
            gl::BlendFunc(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA);
        }

        for image in images {
            let key = TextureKey { image: image.image, generation: image.content_generation };
            if !self.textures.contains_key(&key) {
                let regions = tile_regions(
                    image.pixels.width(),
                    image.pixels.height(),
                    self.maximum_texture_size,
                );
                if texture_bytes(&regions).is_some_and(|bytes| bytes > cache_limit) {
                    for region in regions {
                        let Ok(tile) = Self::upload_tile(&image.pixels, region) else {
                            break;
                        };
                        self.draw_tile(
                            viewport.width,
                            viewport.height,
                            image,
                            tile.texture,
                            tile.region,
                        );
                    }
                    continue;
                }
                if self.upload(key, &image.pixels, cache_limit).is_err() {
                    continue;
                }
            }
            self.usage_clock = self.usage_clock.saturating_add(1);
            let Some(cached) = self.textures.get_mut(&key) else {
                continue;
            };
            cached.last_used = self.usage_clock;
            let tiles: Vec<_> =
                cached.tiles.iter().map(|tile| (tile.texture, tile.region)).collect();
            for (texture, region) in tiles {
                self.draw_tile(viewport.width, viewport.height, image, texture, region);
            }
        }

        // SAFETY: Resetting bindings does not invalidate renderer-owned objects.
        unsafe {
            gl::BindTexture(gl::TEXTURE_2D, 0);
            gl::BindBuffer(gl::ARRAY_BUFFER, 0);
            gl::BindVertexArray(0);
            gl::UseProgram(0);
            // Text rendering relies on dual-source blending and does not reset this state before
            // every batch. Restore the renderer's regular blend function after image composition.
            gl::BlendFunc(gl::SRC1_COLOR, gl::ONE_MINUS_SRC1_COLOR);
            gl::Scissor(
                previous_scissor[0],
                previous_scissor[1],
                previous_scissor[2],
                previous_scissor[3],
            );
            if !scissor_was_enabled {
                gl::Disable(gl::SCISSOR_TEST);
            }
        }
    }

    fn draw_tile(
        &mut self,
        viewport_width: f32,
        viewport_height: f32,
        image: &RenderableImage,
        texture: GLuint,
        region: TileRegion,
    ) {
        let source_right = image.source_x + image.source_width;
        let source_bottom = image.source_y + image.source_height;
        let tile_right = region.core_x + region.core_width;
        let tile_bottom = region.core_y + region.core_height;
        let left = image.source_x.max(region.core_x);
        let right = source_right.min(tile_right);
        let top = image.source_y.max(region.core_y);
        let bottom = source_bottom.min(tile_bottom);
        if left >= right || top >= bottom {
            return;
        }

        let destination_left =
            image.x + (left - image.source_x) as f32 / image.source_width as f32 * image.width;
        let destination_right =
            image.x + (right - image.source_x) as f32 / image.source_width as f32 * image.width;
        let mut destination_top =
            image.y + (top - image.source_y) as f32 / image.source_height as f32 * image.height;
        let mut destination_bottom =
            image.y + (bottom - image.source_y) as f32 / image.source_height as f32 * image.height;
        let texture_left = (left - region.upload_x) as f32 / region.upload_width as f32;
        let texture_right = (right - region.upload_x) as f32 / region.upload_width as f32;
        let mut texture_top = (top - region.upload_y) as f32 / region.upload_height as f32;
        let mut texture_bottom = (bottom - region.upload_y) as f32 / region.upload_height as f32;
        if destination_top < image.clip_top {
            let fraction =
                (image.clip_top - destination_top) / (destination_bottom - destination_top);
            texture_top += fraction * (texture_bottom - texture_top);
            destination_top = image.clip_top;
        }
        if destination_bottom > image.clip_bottom {
            let fraction =
                (destination_bottom - image.clip_bottom) / (destination_bottom - destination_top);
            texture_bottom -= fraction * (texture_bottom - texture_top);
            destination_bottom = image.clip_bottom;
        }
        if destination_top >= destination_bottom {
            return;
        }
        let left = destination_left / (viewport_width / 2.) - 1.;
        let right = destination_right / (viewport_width / 2.) - 1.;
        let top = 1. - destination_top / (viewport_height / 2.);
        let bottom = 1. - destination_bottom / (viewport_height / 2.);
        let vertices = [
            Vertex { x: left, y: top, texture_x: texture_left, texture_y: texture_top },
            Vertex { x: left, y: bottom, texture_x: texture_left, texture_y: texture_bottom },
            Vertex { x: right, y: top, texture_x: texture_right, texture_y: texture_top },
            Vertex { x: right, y: top, texture_x: texture_right, texture_y: texture_top },
            Vertex { x: right, y: bottom, texture_x: texture_right, texture_y: texture_bottom },
            Vertex { x: left, y: bottom, texture_x: texture_left, texture_y: texture_bottom },
        ];

        // SAFETY: `texture` is cached and `vertices` covers the uploaded buffer range.
        unsafe {
            gl::BindTexture(gl::TEXTURE_2D, texture);
            gl::BufferData(
                gl::ARRAY_BUFFER,
                mem::size_of_val(&vertices) as isize,
                vertices.as_ptr().cast(),
                gl::STREAM_DRAW,
            );
            gl::DrawArrays(gl::TRIANGLES, 0, vertices.len() as i32);
        }
    }

    fn remove_image_textures(&mut self, image: ImageHandle) {
        let keys: Vec<_> = self.textures.keys().filter(|key| key.image == image).copied().collect();
        for key in keys {
            if let Some(cached) = self.textures.remove(&key) {
                self.texture_bytes = self.texture_bytes.saturating_sub(cached.bytes);
            }
        }
    }

    fn evict_for(&mut self, incoming: usize, cache_limit: usize) {
        while self.texture_bytes.saturating_add(incoming) > cache_limit {
            let candidate = self
                .textures
                .iter()
                .min_by_key(|(key, cached)| (cached.last_used, key.generation))
                .map(|(key, _)| *key);
            let Some(candidate) = candidate else {
                break;
            };
            if let Some(cached) = self.textures.remove(&candidate) {
                self.texture_bytes = self.texture_bytes.saturating_sub(cached.bytes);
            }
        }
    }

    fn upload_tile(pixels: &PixelBuffer, region: TileRegion) -> Result<TextureTile, Error> {
        let source_stride = usize::try_from(pixels.width())
            .ok()
            .and_then(|width| width.checked_mul(4))
            .ok_or_else(|| Error::Other("image row too wide".into()))?;
        let row_bytes = usize::try_from(region.upload_width)
            .ok()
            .and_then(|width| width.checked_mul(4))
            .ok_or_else(|| Error::Other("image tile too wide".into()))?;
        let tile_bytes = row_bytes
            .checked_mul(region.upload_height as usize)
            .ok_or_else(|| Error::Other("image tile too large".into()))?;
        let mut data = Vec::with_capacity(tile_bytes);
        for row in region.upload_y..region.upload_y + region.upload_height {
            let start = row as usize * source_stride + region.upload_x as usize * 4;
            data.extend_from_slice(&pixels.bytes()[start..start + row_bytes]);
        }
        let width = i32::try_from(region.upload_width)
            .map_err(|_| Error::Other("image tile too wide".into()))?;
        let height = i32::try_from(region.upload_height)
            .map_err(|_| Error::Other("image tile too tall".into()))?;
        let mut texture = 0;
        // SAFETY: The tile buffer matches the declared RGBA dimensions and GL copies it.
        unsafe {
            gl::GenTextures(1, &mut texture);
            gl::BindTexture(gl::TEXTURE_2D, texture);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::LINEAR as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::LINEAR as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_S, gl::CLAMP_TO_EDGE as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_T, gl::CLAMP_TO_EDGE as i32);
            gl::PixelStorei(gl::UNPACK_ALIGNMENT, 1);
            gl::TexImage2D(
                gl::TEXTURE_2D,
                0,
                gl::RGBA as i32,
                width,
                height,
                0,
                gl::RGBA,
                gl::UNSIGNED_BYTE,
                data.as_ptr().cast(),
            );
        }
        Ok(TextureTile { texture, region })
    }

    fn upload(
        &mut self,
        key: TextureKey,
        pixels: &PixelBuffer,
        cache_limit: usize,
    ) -> Result<(), Error> {
        let regions = tile_regions(pixels.width(), pixels.height(), self.maximum_texture_size);
        let bytes = texture_bytes(&regions)
            .ok_or_else(|| Error::Other("image texture cache size overflow".into()))?;
        self.remove_image_textures(key.image);
        self.evict_for(bytes, cache_limit);

        let mut tiles = Vec::with_capacity(regions.len());
        for region in regions {
            tiles.push(Self::upload_tile(pixels, region)?);
        }
        unsafe { gl::BindTexture(gl::TEXTURE_2D, 0) };
        self.texture_bytes = self.texture_bytes.saturating_add(bytes);
        self.textures.insert(key, CachedImage { tiles, bytes, last_used: self.usage_clock });
        Ok(())
    }
}

impl Drop for ImageRenderer {
    fn drop(&mut self) {
        // Drop textures before deleting shared renderer buffers.
        self.textures.clear();
        // SAFETY: These GL names are uniquely owned by this renderer.
        unsafe {
            gl::DeleteBuffers(1, &self.vbo);
            gl::DeleteVertexArrays(1, &self.vao);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texture_tiles_cover_image_with_bounded_overlapping_uploads() {
        let regions = tile_regions(8, 5, 4);
        assert!(regions.iter().all(|region| region.upload_width <= 4 && region.upload_height <= 4));
        let core_pixels: u32 =
            regions.iter().map(|region| region.core_width * region.core_height).sum();
        assert_eq!(core_pixels, 40);
        assert!(regions.iter().any(|region| region.upload_x < region.core_x));
        assert!(regions.iter().any(|region| region.upload_y < region.core_y));

        let single_pixel_tiles = tile_regions(3, 2, 1);
        assert_eq!(single_pixel_tiles.len(), 6);
        assert!(
            single_pixel_tiles
                .iter()
                .all(|region| region.upload_width == 1 && region.upload_height == 1)
        );
        assert_eq!(texture_bytes(&single_pixel_tiles), Some(24));
        assert_eq!(intersect_scissors([0, 0, 10, 10], [5, -5, 10, 10]), [5, 0, 5, 5]);
    }
}
