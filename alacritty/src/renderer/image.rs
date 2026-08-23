use std::collections::HashMap;
use std::mem;

use alacritty_terminal::graphics::{ImageHandle, PixelBuffer};

use crate::gl;
use crate::gl::types::*;
use crate::renderer::Error;
use crate::renderer::shader::{ShaderProgram, ShaderVersion};

const IMAGE_SHADER_F: &str = include_str!("../../res/image.f.glsl");
const IMAGE_SHADER_V: &str = include_str!("../../res/image.v.glsl");

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
struct Texture(GLuint);

impl Drop for Texture {
    fn drop(&mut self) {
        // SAFETY: The texture name was created by this renderer in the current shared GL context.
        unsafe { gl::DeleteTextures(1, &self.0) }
    }
}

#[derive(Debug)]
pub(super) struct ImageRenderer {
    program: ShaderProgram,
    texture_uniform: GLint,
    vao: GLuint,
    vbo: GLuint,
    textures: HashMap<TextureKey, Texture>,
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

        Ok(Self { program, texture_uniform, vao, vbo, textures: HashMap::new() })
    }

    pub(super) fn draw(
        &mut self,
        viewport_width: f32,
        viewport_height: f32,
        images: &[RenderableImage],
    ) {
        if images.is_empty() {
            return;
        }

        // SAFETY: All GL names are owned by this renderer and a current context is established by
        // `Display::draw`. Uploaded slices remain alive for each call consuming their pointers.
        unsafe {
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
            if !self.textures.contains_key(&key) && self.upload(key, &image.pixels).is_err() {
                continue;
            }
            let Some(texture) = self.textures.get(&key) else {
                continue;
            };

            let texture_width = image.pixels.width() as f32;
            let texture_height = image.pixels.height() as f32;
            let left = image.source_x as f32 / texture_width;
            let top = image.source_y as f32 / texture_height;
            let right = (image.source_x + image.source_width) as f32 / texture_width;
            let bottom = (image.source_y + image.source_height) as f32 / texture_height;
            let x = image.x / (viewport_width / 2.) - 1.;
            let y = 1. - image.y / (viewport_height / 2.);
            let width = image.width / (viewport_width / 2.);
            let height = image.height / (viewport_height / 2.);
            let vertices = [
                Vertex { x, y, texture_x: left, texture_y: top },
                Vertex { x, y: y - height, texture_x: left, texture_y: bottom },
                Vertex { x: x + width, y, texture_x: right, texture_y: top },
                Vertex { x: x + width, y, texture_x: right, texture_y: top },
                Vertex { x: x + width, y: y - height, texture_x: right, texture_y: bottom },
                Vertex { x, y: y - height, texture_x: left, texture_y: bottom },
            ];

            // SAFETY: `texture` is live in the cache and `vertices` covers the uploaded byte range.
            unsafe {
                gl::BindTexture(gl::TEXTURE_2D, texture.0);
                gl::BufferData(
                    gl::ARRAY_BUFFER,
                    mem::size_of_val(&vertices) as isize,
                    vertices.as_ptr().cast(),
                    gl::STREAM_DRAW,
                );
                gl::DrawArrays(gl::TRIANGLES, 0, vertices.len() as i32);
            }
        }

        // SAFETY: Resetting bindings does not invalidate renderer-owned objects.
        unsafe {
            gl::BindTexture(gl::TEXTURE_2D, 0);
            gl::BindBuffer(gl::ARRAY_BUFFER, 0);
            gl::BindVertexArray(0);
            gl::UseProgram(0);
        }
    }

    fn upload(&mut self, key: TextureKey, pixels: &PixelBuffer) -> Result<(), Error> {
        let width =
            i32::try_from(pixels.width()).map_err(|_| Error::Other("image too wide".into()))?;
        let height =
            i32::try_from(pixels.height()).map_err(|_| Error::Other("image too tall".into()))?;
        self.textures.retain(|cached, _| cached.image != key.image);
        let mut texture = 0;
        // SAFETY: The immutable RGBA slice has exactly four bytes per declared image pixel. GL
        // copies it before this call returns.
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
                pixels.bytes().as_ptr().cast(),
            );
            gl::BindTexture(gl::TEXTURE_2D, 0);
        }
        self.textures.insert(key, Texture(texture));
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
