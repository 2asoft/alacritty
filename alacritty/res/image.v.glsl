#if defined(GLES2_RENDERER)
attribute vec2 aPos;
attribute vec2 aTexCoord;
varying mediump vec2 texCoord;
#else
layout (location = 0) in vec2 aPos;
layout (location = 1) in vec2 aTexCoord;
out vec2 texCoord;
#endif

void main() {
    texCoord = aTexCoord;
    gl_Position = vec4(aPos, 0.0, 1.0);
}
