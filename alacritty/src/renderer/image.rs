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

fn premultiply_alpha(bytes: &mut [u8]) {
    for pixel in bytes.chunks_exact_mut(4) {
        let alpha = u16::from(pixel[3]);
        for component in &mut pixel[..3] {
            *component = ((u16::from(*component) * alpha + 127) / 255) as u8;
        }
    }
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

#[derive(Clone, Copy, Debug, PartialEq)]
struct ImageGeometry {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    source_x: f64,
    source_y: f64,
    source_width: f64,
    source_height: f64,
    clip_top: f32,
    clip_bottom: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct SampledTileQuad {
    destination_left: f32,
    destination_right: f32,
    destination_top: f32,
    destination_bottom: f32,
    texture_left: f32,
    texture_right: f32,
    texture_top: f32,
    texture_bottom: f32,
}

fn sample_tile(image: &ImageGeometry, region: TileRegion) -> Option<SampledTileQuad> {
    if image.source_width <= 0. || image.source_height <= 0. {
        return None;
    }
    // Intersect relative to the source origin so small extents survive large coordinates.
    let tile_left = f64::from(region.core_x) - image.source_x;
    let tile_top = f64::from(region.core_y) - image.source_y;
    let left = tile_left.max(0.);
    let right = (tile_left + f64::from(region.core_width)).min(image.source_width);
    let top = tile_top.max(0.);
    let bottom = (tile_top + f64::from(region.core_height)).min(image.source_height);
    if left >= right || top >= bottom {
        return None;
    }

    let destination_left = image.x + (left / image.source_width) as f32 * image.width;
    let destination_right = image.x + (right / image.source_width) as f32 * image.width;
    let mut destination_top = image.y + (top / image.source_height) as f32 * image.height;
    let mut destination_bottom = image.y + (bottom / image.source_height) as f32 * image.height;
    let upload_width = f64::from(region.upload_width);
    let upload_height = f64::from(region.upload_height);
    if upload_width <= 0. || upload_height <= 0. {
        return None;
    }
    let upload_x = image.source_x - f64::from(region.upload_x);
    let upload_y = image.source_y - f64::from(region.upload_y);
    let texture_left = ((upload_x + left) / upload_width) as f32;
    let texture_right = ((upload_x + right) / upload_width) as f32;
    let mut texture_top = ((upload_y + top) / upload_height) as f32;
    let mut texture_bottom = ((upload_y + bottom) / upload_height) as f32;
    if destination_top < image.clip_top {
        let span = destination_bottom - destination_top;
        if span <= 0. {
            return None;
        }
        let fraction = (image.clip_top - destination_top) / span;
        texture_top += fraction * (texture_bottom - texture_top);
        destination_top = image.clip_top;
    }
    if destination_bottom > image.clip_bottom {
        let span = destination_bottom - destination_top;
        if span <= 0. {
            return None;
        }
        let fraction = (destination_bottom - image.clip_bottom) / span;
        texture_bottom -= fraction * (texture_bottom - texture_top);
        destination_bottom = image.clip_bottom;
    }
    if destination_top >= destination_bottom {
        return None;
    }
    Some(SampledTileQuad {
        destination_left,
        destination_right,
        destination_top,
        destination_bottom,
        texture_left,
        texture_right,
        texture_top,
        texture_bottom,
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
    pub source_x: f64,
    pub source_y: f64,
    pub source_width: f64,
    pub source_height: f64,
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

        let mut previous_active_texture = 0;
        let mut previous_texture = 0;
        let mut previous_program = 0;
        let mut previous_vao = 0;
        let mut previous_array_buffer = 0;
        let mut previous_blend_source_rgb = 0;
        let mut previous_blend_destination_rgb = 0;
        let mut previous_blend_source_alpha = 0;
        let mut previous_blend_destination_alpha = 0;
        let mut previous_viewport = [0; 4];
        // SAFETY: All GL names are owned by this renderer and a current context is established by
        // `Display::draw`. Uploaded slices remain alive for each call consuming their pointers.
        let blend_was_enabled = unsafe {
            let enabled = gl::IsEnabled(gl::BLEND) == gl::TRUE;
            gl::GetIntegerv(gl::ACTIVE_TEXTURE, &mut previous_active_texture);
            gl::ActiveTexture(gl::TEXTURE0);
            gl::GetIntegerv(gl::TEXTURE_BINDING_2D, &mut previous_texture);
            gl::GetIntegerv(gl::CURRENT_PROGRAM, &mut previous_program);
            gl::GetIntegerv(gl::VERTEX_ARRAY_BINDING, &mut previous_vao);
            gl::GetIntegerv(gl::ARRAY_BUFFER_BINDING, &mut previous_array_buffer);
            gl::GetIntegerv(gl::BLEND_SRC_RGB, &mut previous_blend_source_rgb);
            gl::GetIntegerv(gl::BLEND_DST_RGB, &mut previous_blend_destination_rgb);
            gl::GetIntegerv(gl::BLEND_SRC_ALPHA, &mut previous_blend_source_alpha);
            gl::GetIntegerv(gl::BLEND_DST_ALPHA, &mut previous_blend_destination_alpha);
            gl::GetIntegerv(gl::VIEWPORT, previous_viewport.as_mut_ptr());
            enabled
        };
        unsafe {
            gl::Enable(gl::SCISSOR_TEST);
            gl::Scissor(scissor[0], scissor[1], scissor[2], scissor[3]);
            gl::Viewport(0, 0, viewport.width as i32, viewport.height as i32);
            gl::UseProgram(self.program.id());
            gl::Uniform1i(self.texture_uniform, 0);
            gl::ActiveTexture(gl::TEXTURE0);
            gl::BindVertexArray(self.vao);
            gl::BindBuffer(gl::ARRAY_BUFFER, self.vbo);
            gl::Enable(gl::BLEND);
            gl::BlendFunc(gl::ONE, gl::ONE_MINUS_SRC_ALPHA);
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

        // SAFETY: Restoring the captured state, including viewport, keeps other renderers' binding
        // caches coherent.
        unsafe {
            gl::BindTexture(gl::TEXTURE_2D, previous_texture as GLuint);
            gl::ActiveTexture(previous_active_texture as GLenum);
            gl::BindVertexArray(previous_vao as GLuint);
            gl::BindBuffer(gl::ARRAY_BUFFER, previous_array_buffer as GLuint);
            gl::UseProgram(previous_program as GLuint);
            gl::BlendFuncSeparate(
                previous_blend_source_rgb as GLenum,
                previous_blend_destination_rgb as GLenum,
                previous_blend_source_alpha as GLenum,
                previous_blend_destination_alpha as GLenum,
            );
            if !blend_was_enabled {
                gl::Disable(gl::BLEND);
            }
            gl::Scissor(
                previous_scissor[0],
                previous_scissor[1],
                previous_scissor[2],
                previous_scissor[3],
            );
            if !scissor_was_enabled {
                gl::Disable(gl::SCISSOR_TEST);
            }
            gl::Viewport(
                previous_viewport[0],
                previous_viewport[1],
                previous_viewport[2],
                previous_viewport[3],
            );
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
        let Some(sampled) = sample_tile(
            &ImageGeometry {
                x: image.x,
                y: image.y,
                width: image.width,
                height: image.height,
                source_x: image.source_x,
                source_y: image.source_y,
                source_width: image.source_width,
                source_height: image.source_height,
                clip_top: image.clip_top,
                clip_bottom: image.clip_bottom,
            },
            region,
        ) else {
            return;
        };
        let left = sampled.destination_left / (viewport_width / 2.) - 1.;
        let right = sampled.destination_right / (viewport_width / 2.) - 1.;
        let top = 1. - sampled.destination_top / (viewport_height / 2.);
        let bottom = 1. - sampled.destination_bottom / (viewport_height / 2.);
        let vertices = [
            Vertex {
                x: left,
                y: top,
                texture_x: sampled.texture_left,
                texture_y: sampled.texture_top,
            },
            Vertex {
                x: left,
                y: bottom,
                texture_x: sampled.texture_left,
                texture_y: sampled.texture_bottom,
            },
            Vertex {
                x: right,
                y: top,
                texture_x: sampled.texture_right,
                texture_y: sampled.texture_top,
            },
            Vertex {
                x: right,
                y: top,
                texture_x: sampled.texture_right,
                texture_y: sampled.texture_top,
            },
            Vertex {
                x: right,
                y: bottom,
                texture_x: sampled.texture_right,
                texture_y: sampled.texture_bottom,
            },
            Vertex {
                x: left,
                y: bottom,
                texture_x: sampled.texture_left,
                texture_y: sampled.texture_bottom,
            },
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
        premultiply_alpha(&mut data);
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
    fn premultiplies_texture_pixels_for_linear_alpha_filtering() {
        let mut pixels = [255, 128, 64, 128, 20, 40, 60, 0, 1, 2, 3, 255];
        premultiply_alpha(&mut pixels);
        assert_eq!(pixels, [128, 64, 32, 128, 0, 0, 0, 0, 1, 2, 3, 255]);
    }

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

    fn pixel_tile() -> TileRegion {
        TileRegion {
            core_x: 0,
            core_y: 0,
            core_width: 1,
            core_height: 1,
            upload_x: 0,
            upload_y: 0,
            upload_width: 1,
            upload_height: 1,
        }
    }

    #[test]
    fn tiny_source_extents_preserve_visible_destination_at_large_coordinates() {
        for (x, y) in [(0, 0), (1 << 24, 0), (0, 1 << 24), (1 << 24, 1 << 24)] {
            let region = TileRegion {
                core_x: x,
                core_y: y,
                core_width: 1,
                core_height: 1,
                upload_x: x,
                upload_y: y,
                upload_width: 1,
                upload_height: 1,
            };
            let extent = 1. / f64::from(u32::MAX);
            let image = ImageGeometry {
                x: 0.,
                y: 0.,
                width: 10.,
                height: 10.,
                source_x: f64::from(x),
                source_y: f64::from(y),
                source_width: extent,
                source_height: extent,
                clip_top: f32::NEG_INFINITY,
                clip_bottom: f32::INFINITY,
            };
            let sampled =
                sample_tile(&image, region).expect("a visible cell retains its source extent");
            assert_eq!((sampled.destination_left, sampled.destination_right), (0., 10.));
            assert_eq!((sampled.destination_top, sampled.destination_bottom), (0., 10.));
        }
    }

    #[test]
    fn source_crops_preserve_subpixels_at_large_coordinates() {
        let coordinate = 1 << 24;
        let region = TileRegion { core_x: coordinate, upload_x: coordinate, ..pixel_tile() };
        for (offset, width) in [(0., 1.), (0.25, 0.5)] {
            let quad = sample_tile(
                &ImageGeometry {
                    x: 0.,
                    y: 0.,
                    width: 10.,
                    height: 10.,
                    source_x: f64::from(coordinate) + offset,
                    source_y: 0.,
                    source_width: width,
                    source_height: 1.,
                    clip_top: 0.,
                    clip_bottom: 10.,
                },
                region,
            )
            .expect("a crop within the tile remains visible");
            assert_eq!(quad.destination_right - quad.destination_left, 10.);
            assert_eq!(quad.texture_left, offset as f32);
            assert_eq!(quad.texture_right, (offset + width) as f32);
        }
    }

    #[test]
    fn fractional_source_rect_keeps_enlarged_placeholder_tile() {
        let right_half = sample_tile(
            &ImageGeometry {
                x: 10.,
                y: 0.,
                width: 10.,
                height: 10.,
                source_x: 0.5,
                source_y: 0.,
                source_width: 0.5,
                source_height: 1.,
                clip_top: f32::NEG_INFINITY,
                clip_bottom: f32::INFINITY,
            },
            pixel_tile(),
        )
        .expect("right-half source remains sampleable");
        assert_eq!(right_half.destination_left, 10.);
        assert_eq!(right_half.destination_right, 20.);
        assert_eq!(right_half.texture_left, 0.5);
        assert_eq!(right_half.texture_right, 1.);

        let integer_source = sample_tile(
            &ImageGeometry {
                x: 0.,
                y: 0.,
                width: 10.,
                height: 20.,
                source_x: 0.,
                source_y: 0.,
                source_width: 1.,
                source_height: 1.,
                clip_top: f32::NEG_INFINITY,
                clip_bottom: f32::INFINITY,
            },
            pixel_tile(),
        )
        .expect("integer source still fills the destination");
        assert_eq!(integer_source.destination_left, 0.);
        assert_eq!(integer_source.destination_right, 10.);
        assert_eq!(integer_source.texture_left, 0.);
        assert_eq!(integer_source.texture_right, 1.);
    }
}
