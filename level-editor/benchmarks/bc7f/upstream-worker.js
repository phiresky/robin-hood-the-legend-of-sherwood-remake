/* Standalone benchmark: the upstream encoder is deliberately not bundled into the app. */
importScripts("/basis/webgl/encoder/build/basis_encoder.js");
let wasmBytes = 0;
const ready = BASIS({
  locateFile: (name) => "/basis/webgl/encoder/build/" + name,
  // Observe the actual linear-memory high-water mark without modifying upstream builds.
  instantiateWasm(imports, receive) {
    WebAssembly.instantiateStreaming(
      fetch("/basis/webgl/encoder/build/basis_encoder.wasm"),
      imports,
    )
      .then(({ instance, module }) => {
        self.wasmMemory = Object.values(instance.exports).find(
          (v) => v instanceof WebAssembly.Memory,
        );
        receive(instance, module);
      })
      .catch((error) => self.postMessage({ error: String(error) }));
    return {};
  },
  print: () => {},
  printErr: (message) => console.error(message),
}).then((module) => {
  module.initializeBasis();
  return module;
});
self.onmessage = async ({ data }) => {
  let encoder;
  try {
    const start = performance.now();
    const module = await ready;
    if (data.init) {
      self.postMessage({ initMs: performance.now() - start });
      return;
    }
    encoder = new module.BasisEncoder();
    encoder.setDebug(false);
    encoder.setComputeStats(false);
    encoder.setSliceSourceImage(0, new Uint8Array(data.pixels), data.width, data.height, 0);
    encoder.setSRGBOptions(true);
    encoder.setMipGen(data.mips);
    encoder.setMipWrapping(false);
    encoder.setDDSFormat("bc7");
    encoder.setDDSBC7Encoder(0);
    encoder.setDDSBC7FLevel(data.level);
    let bytes = 148,
      w = data.width,
      h = data.height;
    do {
      bytes += Math.ceil(w / 4) * Math.ceil(h / 4) * 16;
      if (!data.mips || (w === 1 && h === 1)) break;
      w = Math.max(1, w >> 1);
      h = Math.max(1, h >> 1);
    } while (true);
    const output = new Uint8Array(bytes);
    const setupMs = performance.now() - start;
    const encodingStart = performance.now();
    const length = encoder.encodeToDDS(output);
    if (!length) throw new Error("Basis encodeToDDS failed");
    const encodeMs = performance.now() - encodingStart;
    wasmBytes = self.wasmMemory.buffer.byteLength;
    self.postMessage({ dds: output.slice(0, length).buffer, encodeMs, setupMs, wasmBytes }, []);
  } catch (error) {
    self.postMessage({ error: String(error.stack ?? error) });
  } finally {
    encoder?.delete();
  }
};
