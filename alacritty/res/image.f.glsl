#if defined(GLES2_RENDERER)
#define FRAG_COLOR gl_FragColor
#define SAMPLE_TEXTURE texture2D
varying mediump vec2 texCoord;
#else
out vec4 FragColor;
#define FRAG_COLOR FragColor
#define SAMPLE_TEXTURE texture
in vec2 texCoord;
#endif

uniform sampler2D imageTexture;

void main() {
    FRAG_COLOR = SAMPLE_TEXTURE(imageTexture, texCoord);
}
