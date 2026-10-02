const status = document.querySelector("#status");
const worker = new Worker("/worker.js");
function call(data, transfer = []) {
  return new Promise((resolve, reject) => {
    worker.onmessage = ({ data }) => (data.error ? reject(new Error(data.error)) : resolve(data));
    worker.onerror = reject;
    worker.postMessage(data, transfer);
  });
}
const canvas = document.createElement("canvas");
const gl = canvas.getContext("webgl2", { premultipliedAlpha: false, antialias: false });
if (!gl) throw new Error("WebGL2 unavailable");
const bptc = gl.getExtension("EXT_texture_compression_bptc");
if (!bptc) throw new Error("BC7 unsupported");
function shader(type, code) {
  const s = gl.createShader(type);
  gl.shaderSource(s, code);
  gl.compileShader(s);
  if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(s));
  return s;
}
const program = gl.createProgram();
gl.attachShader(
  program,
  shader(
    gl.VERTEX_SHADER,
    `#version 300 es
void main(){ vec2 p=vec2((gl_VertexID<<1)&2, gl_VertexID&2); gl_Position=vec4(p*2.0-1.0,0,1); }`,
  ),
);
gl.attachShader(
  program,
  shader(
    gl.FRAGMENT_SHADER,
    `#version 300 es
precision highp float; uniform sampler2D tex; out vec4 color;
void main(){color=texelFetch(tex,ivec2(gl_FragCoord.xy),0);}`,
  ),
);
gl.linkProgram(program);
if (!gl.getProgramParameter(program, gl.LINK_STATUS))
  throw new Error(gl.getProgramInfoLog(program));
gl.useProgram(program);
gl.disable(gl.DITHER);
function read(texture, w, h) {
  canvas.width = w;
  canvas.height = h;
  const target = gl.createTexture();
  gl.bindTexture(gl.TEXTURE_2D, target);
  gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, w, h, 0, gl.RGBA, gl.UNSIGNED_BYTE, null);
  const fb = gl.createFramebuffer();
  gl.bindFramebuffer(gl.FRAMEBUFFER, fb);
  gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, target, 0);
  gl.bindTexture(gl.TEXTURE_2D, texture);
  gl.viewport(0, 0, w, h);
  gl.drawArrays(gl.TRIANGLES, 0, 3);
  const pixels = new Uint8Array(w * h * 4);
  gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, pixels);
  const error = gl.getError();
  if (error) throw new Error("WebGL error " + error);
  gl.bindFramebuffer(gl.FRAMEBUFFER, null);
  gl.deleteFramebuffer(fb);
  gl.deleteTexture(target);
  return pixels;
}
function texture() {
  const t = gl.createTexture();
  gl.bindTexture(gl.TEXTURE_2D, t);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
  return t;
}
function uploadDDS(dds) {
  const v = new DataView(dds);
  if (v.getUint32(0, true) !== 0x20534444 || v.getUint32(84, true) !== 0x30315844)
    throw new Error("Expected DX10 DDS");
  const format = v.getUint32(128, true);
  if (![98, 99].includes(format)) throw new Error("Expected BC7 DDS: " + format);
  let w = v.getUint32(16, true),
    h = v.getUint32(12, true),
    offset = 148;
  const t = texture(),
    levels = v.getUint32(28, true) || 1;
  for (let i = 0; i < levels; i++) {
    const bytes = Math.ceil(w / 4) * Math.ceil(h / 4) * 16;
    // UNORM intentionally used for byte-domain error measurement of encoded sRGB values.
    gl.compressedTexImage2D(
      gl.TEXTURE_2D,
      i,
      bptc.COMPRESSED_RGBA_BPTC_UNORM_EXT,
      w,
      h,
      0,
      new Uint8Array(dds, offset, bytes),
    );
    offset += bytes;
    w = Math.max(1, w >> 1);
    h = Math.max(1, h >> 1);
  }
  if (offset !== dds.byteLength) throw new Error("DDS mip sizes do not match payload");
  return { texture: t, levels };
}
function quality(a, b) {
  let rgb = 0,
    visible = 0,
    visibleCount = 0,
    alpha = 0,
    flips = 0,
    nonOpaque = 0,
    max = 0;
  for (let i = 0; i < a.length; i += 4) {
    for (let k = 0; k < 3; k++) {
      const d = a[i + k] - b[i + k];
      rgb += d * d;
      max = Math.max(max, Math.abs(d));
      if (a[i + 3] >= 128) {
        visible += d * d;
        visibleCount++;
      }
    }
    const d = a[i + 3] - b[i + 3];
    alpha += d * d;
    if (a[i + 3] >= 128 !== b[i + 3] >= 128) flips++;
    if (a[i + 3] === 255 && b[i + 3] !== 255) nonOpaque++;
  }
  const psnr = (sum, n) => (sum ? 10 * Math.log10((255 * 255 * n) / sum) : null);
  return {
    rgbPSNR: psnr(rgb, (a.length / 4) * 3),
    visiblePSNR: psnr(visible, visibleCount),
    alphaPSNR: psnr(alpha, a.length / 4),
    alphaCutoffFlips: flips,
    opaqueAlphaChanged: nonOpaque,
    maxRGBError: max,
  };
}
function crop(a, b, w, h) {
  // Pick the 256px crop with greatest source color variation, avoiding empty atlas padding.
  let best = -1,
    bx = 0,
    by = 0;
  for (let y = 0; y < h; y += 128)
    for (let x = 0; x < w; x += 128) {
      let sum = 0,
        sq = 0,
        n = 0;
      for (let yy = y; yy < Math.min(y + 256, h); yy += 8)
        for (let xx = x; xx < Math.min(x + 256, w); xx += 8) {
          const p = (yy * w + xx) * 4;
          if (a[p + 3] < 128) continue;
          const v = a[p] + a[p + 1] + a[p + 2];
          sum += v;
          sq += v * v;
          n++;
        }
      const score = n ? sq - (sum * sum) / n : 0;
      if (score > best) {
        best = score;
        bx = x;
        by = y;
      }
    }
  const cw = Math.min(256, w - bx),
    ch = Math.min(256, h - by),
    c = document.createElement("canvas");
  c.width = cw * 3;
  c.height = ch;
  const ctx = c.getContext("2d"),
    im = ctx.createImageData(c.width, c.height);
  for (let y = 0; y < ch; y++)
    for (let x = 0; x < cw; x++)
      for (let k = 0; k < 3; k++) {
        const src = ((by + y) * w + bx + x) * 4 + k,
          dst = (y * c.width + x) * 4 + k;
        im.data[dst] = a[src];
        im.data[dst + cw * 4] = b[src];
        im.data[dst + cw * 8] = Math.min(255, Math.abs(a[src] - b[src]) * 8);
      }
  for (let i = 3; i < im.data.length; i += 4) im.data[i] = 255;
  ctx.putImageData(im, 0, 0);
  return c.toDataURL();
}
window.results = [];
window.phase = "init";
window.done = false;
try {
  const init = await call({ init: true });
  window.init = init;
  const mapRun = !!new URLSearchParams(location.search).get("map");
  const inputs = await (await fetch("/inputs/manifest.json")).json();
  // Warm the encoder with an actual small image before recording the main samples.
  const batchStart = performance.now();
  for (const [iteration, spec] of [inputs[0], ...inputs, ...(mapRun ? [] : inputs)].entries()) {
    const run = iteration === 0 ? "warmup" : iteration <= inputs.length ? "first" : "repeat";
    window.phase = spec.name + " " + run;
    status.textContent = window.phase;
    const fetchStart = performance.now();
    const response = await fetch("/inputs/" + spec.name);
    const blob = new Blob([await response.arrayBuffer()], { type: spec.mime });
    const fetchMs = performance.now() - fetchStart;
    const decodeStart = performance.now();
    const image = await createImageBitmap(blob, {
      premultiplyAlpha: "none",
      colorSpaceConversion: "none",
    });
    const decodeMs = performance.now() - decodeStart;
    const t = texture();
    gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, false);
    gl.pixelStorei(gl.UNPACK_COLORSPACE_CONVERSION_WEBGL, gl.NONE);
    const extractionStart = performance.now();
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, gl.RGBA, gl.UNSIGNED_BYTE, image);
    image.close();
    const original = read(t, spec.width, spec.height);
    gl.deleteTexture(t);
    const extractionMs = performance.now() - extractionStart;
    const width = Math.ceil(spec.width / 4) * 4,
      height = Math.ceil(spec.height / 4) * 4;
    const paddingStart = performance.now();
    const pixels = new Uint8Array(width * height * 4);
    for (let y = 0; y < height; y++) {
      const src = Math.min(y, spec.height - 1) * spec.width * 4;
      pixels.set(original.subarray(src, src + spec.width * 4), y * width * 4);
      for (let x = spec.width; x < width; x++)
        pixels.set(
          original.subarray(src + (spec.width - 1) * 4, src + spec.width * 4),
          (y * width + x) * 4,
        );
    }
    const paddingMs = performance.now() - paddingStart;
    const start = performance.now();
    const encoded = await call({ width, height, pixels: pixels.buffer, mips: true, level: 0 }, [
      pixels.buffer,
    ]);
    const workerMs = performance.now() - start;
    const uploadStart = performance.now();
    const uploaded = uploadDDS(encoded.dds);
    gl.finish();
    const uploadMs = performance.now() - uploadStart;
    const decoded = mapRun ? null : read(uploaded.texture, spec.width, spec.height);
    gl.deleteTexture(uploaded.texture);
    const row = {
      ...spec,
      encodedWidth: width,
      encodedHeight: height,
      run,
      fetchMs,
      decodeMs,
      extractionMs,
      paddingMs,
      mipMs: encoded.mipMs,
      setupMs: encoded.setupMs,
      encodeMs: encoded.encodeMs,
      workerMs,
      uploadMs,
      wasmBytes: encoded.wasmBytes,
      bc7Bytes: encoded.dds.byteLength - 148,
      levels: uploaded.levels,
      ...(decoded ? quality(original, decoded) : {}),
    };
    window.results.push(row);
    if (run === "first" && decoded)
      await fetch("/artifact/" + spec.name + ".png", {
        method: "POST",
        body: await (await fetch(crop(original, decoded, spec.width, spec.height))).blob(),
      });
    await fetch("/row", { method: "POST", body: JSON.stringify(row) });
  }
  window.batchMs = performance.now() - batchStart;
  worker.terminate();
  window.phase = "done";
  window.done = true;
  status.textContent = JSON.stringify(window.results, null, 2);
} catch (error) {
  worker.terminate();
  window.error = String(error.stack ?? error);
  window.done = true;
  status.textContent = window.error;
}
