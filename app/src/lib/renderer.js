// Draws I420 frames with WebGL2: the three planes go up as single-channel
// textures and the shader converts to RGB, so the CPU never touches a pixel
// after the socket hands it over.

const VERTEX = `#version 300 es
in vec2 position;
out vec2 uv;
void main() {
  uv = vec2((position.x + 1.0) * 0.5, (1.0 - position.y) * 0.5);
  gl_Position = vec4(position, 0.0, 1.0);
}`;

const FRAGMENT = `#version 300 es
precision mediump float;
in vec2 uv;
uniform sampler2D planeY;
uniform sampler2D planeU;
uniform sampler2D planeV;
uniform mat3 matrix;
uniform vec3 offset;
uniform vec3 scale;
out vec4 color;
void main() {
  vec3 yuv = vec3(
    texture(planeY, uv).r,
    texture(planeU, uv).r,
    texture(planeV, uv).r
  );
  yuv = (yuv - offset) * scale;
  color = vec4(clamp(matrix * yuv, 0.0, 1.0), 1.0);
}`;

// Column-major YUV to RGB matrices, by colour space byte from the frame header
// (0 and 1 are BT.601, 2 and 3 are BT.709; odd means full range).
const BT601 = [1, 1, 1, 0, -0.344136, 1.772, 1.402, -0.714136, 0];
const BT709 = [1, 1, 1, 0, -0.187324, 1.8556, 1.5748, -0.468124, 0];

function spaceFor(colorByte) {
  const full = colorByte % 2 === 1;
  return {
    matrix: colorByte >= 2 ? BT709 : BT601,
    offset: full ? [0, 0.5, 0.5] : [16 / 255, 0.5, 0.5],
    scale: full ? [1, 1, 1] : [255 / 219, 255 / 224, 255 / 224],
  };
}

function compile(gl, type, source) {
  const shader = gl.createShader(type);
  gl.shaderSource(shader, source);
  gl.compileShader(shader);
  if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
    throw new Error(gl.getShaderInfoLog(shader));
  }
  return shader;
}

export class I420Renderer {
  constructor(canvas) {
    const gl = canvas.getContext("webgl2", { antialias: false, alpha: false });
    if (!gl) throw new Error("WebGL2 is not available");
    this.gl = gl;
    this.canvas = canvas;

    const program = gl.createProgram();
    gl.attachShader(program, compile(gl, gl.VERTEX_SHADER, VERTEX));
    gl.attachShader(program, compile(gl, gl.FRAGMENT_SHADER, FRAGMENT));
    gl.linkProgram(program);
    if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
      throw new Error(gl.getProgramInfoLog(program));
    }
    gl.useProgram(program);
    this.program = program;

    const quad = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER, quad);
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 1, -1, -1, 1, 1, 1]), gl.STATIC_DRAW);
    const position = gl.getAttribLocation(program, "position");
    gl.enableVertexAttribArray(position);
    gl.vertexAttribPointer(position, 2, gl.FLOAT, false, 0, 0);

    this.textures = ["planeY", "planeU", "planeV"].map((name, unit) => {
      const texture = gl.createTexture();
      gl.activeTexture(gl.TEXTURE0 + unit);
      gl.bindTexture(gl.TEXTURE_2D, texture);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
      gl.uniform1i(gl.getUniformLocation(program, name), unit);
      return texture;
    });
    gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
    this.uniforms = {
      matrix: gl.getUniformLocation(program, "matrix"),
      offset: gl.getUniformLocation(program, "offset"),
      scale: gl.getUniformLocation(program, "scale"),
    };
    this.colorByte = -1;
  }

  draw(frame) {
    const { gl, canvas } = this;
    const { width, height, colorByte, data } = frame;
    if (canvas.width !== width || canvas.height !== height) {
      canvas.width = width;
      canvas.height = height;
    }
    gl.viewport(0, 0, width, height);

    if (colorByte !== this.colorByte) {
      const space = spaceFor(colorByte);
      gl.uniformMatrix3fv(this.uniforms.matrix, false, space.matrix);
      gl.uniform3fv(this.uniforms.offset, space.offset);
      gl.uniform3fv(this.uniforms.scale, space.scale);
      this.colorByte = colorByte;
    }

    const lumaSize = width * height;
    const chromaWidth = width / 2;
    const chromaHeight = height / 2;
    const chromaSize = chromaWidth * chromaHeight;
    const planes = [
      [data.subarray(0, lumaSize), width, height],
      [data.subarray(lumaSize, lumaSize + chromaSize), chromaWidth, chromaHeight],
      [data.subarray(lumaSize + chromaSize, lumaSize + 2 * chromaSize), chromaWidth, chromaHeight],
    ];
    planes.forEach(([plane, w, h], unit) => {
      gl.activeTexture(gl.TEXTURE0 + unit);
      gl.bindTexture(gl.TEXTURE_2D, this.textures[unit]);
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.R8, w, h, 0, gl.RED, gl.UNSIGNED_BYTE, plane);
    });
    gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
  }

  dispose() {
    const { gl } = this;
    this.textures.forEach((t) => gl.deleteTexture(t));
    gl.deleteProgram(this.program);
  }
}
