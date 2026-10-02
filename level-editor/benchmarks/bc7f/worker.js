/* One worker owns the encoder's memory; terminate it after a batch to release WASM pages. */
importScripts("/compiled/bc7f.js");
const ready = BC7F({ locateFile: (name) => "/compiled/" + name }).then((module) => {
  module._initialize();
  return module;
});
self.onmessage = async ({ data }) => {
  let module,
    source = 0,
    output = 0,
    next = 0;
  try {
    const start = performance.now();
    module = await ready;
    if (data.init) {
      self.postMessage({ initMs: performance.now() - start });
      return;
    }
    let bytes = 148,
      w = data.width,
      h = data.height,
      levels = 0;
    do {
      bytes += Math.ceil(w / 4) * Math.ceil(h / 4) * 16;
      levels++;
      if (!data.mips || (w === 1 && h === 1)) break;
      w = Math.max(1, w >> 1);
      h = Math.max(1, h >> 1);
    } while (true);
    source = module._malloc(data.pixels.byteLength);
    output = module._malloc(bytes);
    if (!source || !output) throw new Error("WASM allocation failed");
    module.HEAPU8.set(new Uint8Array(data.pixels), source);
    module.HEAPU8.fill(0, output, output + 148);
    const header = new DataView(module.HEAPU8.buffer, output, 148);
    // Minimal DX10 DDS envelope for the benchmark's WebGL uploader.
    for (const [offset, value] of [
      [0, 0x20534444],
      [4, 124],
      [8, 0xa1007],
      [12, data.height],
      [16, data.width],
      [20, Math.ceil(data.width / 4) * Math.ceil(data.height / 4) * 16],
      [28, levels],
      [76, 32],
      [80, 4],
      [84, 0x30315844],
      [108, 0x401008],
      [128, 99],
      [132, 3],
      [140, 1],
    ])
      header.setUint32(offset, value, true);
    const setupMs = performance.now() - start;
    let encodeMs = 0,
      mipMs = 0,
      offset = 148;
    w = data.width;
    h = data.height;
    for (let i = 0; i < levels; i++) {
      let t = performance.now();
      module._encode(source, w, h, output + offset, data.level);
      encodeMs += performance.now() - t;
      offset += Math.ceil(w / 4) * Math.ceil(h / 4) * 16;
      if (i === levels - 1) break;
      const nw = Math.max(1, w >> 1),
        nh = Math.max(1, h >> 1);
      next = module._malloc(nw * nh * 4);
      if (!next) throw new Error("Mipmap allocation failed");
      t = performance.now();
      module._mip(source, w, h, next);
      mipMs += performance.now() - t;
      module._free(source);
      source = next;
      next = 0;
      w = nw;
      h = nh;
    }
    const dds = module.HEAPU8.slice(output, output + bytes).buffer;
    const wasmBytes = module.HEAPU8.buffer.byteLength;
    self.postMessage({ dds, setupMs, encodeMs, mipMs, wasmBytes }, [dds]);
  } catch (error) {
    self.postMessage({ error: String(error.stack ?? error) });
  } finally {
    if (source) module._free(source);
    if (output) module._free(output);
    if (next) module._free(next);
  }
};
