import createBC7F from "./vendor/bc7f/bc7f.js";
import wasmUrl from "./vendor/bc7f/bc7f.wasm?url";

let ready: ReturnType<typeof createBC7F> | undefined;
self.onmessage = async ({
  data,
}: MessageEvent<{ pixels: ArrayBuffer; width: number; height: number }>) => {
  let module: Awaited<typeof ready> | undefined;
  let source = 0,
    output = 0,
    next = 0;
  try {
    const { width, height, pixels } = data;
    if (
      !Number.isInteger(width) ||
      !Number.isInteger(height) ||
      width < 4 ||
      height < 4 ||
      width % 4 ||
      height % 4 ||
      width * height > 4096 * 4096 ||
      pixels.byteLength !== width * height * 4
    )
      throw new Error("Invalid BC7 texture dimensions");
    // Initialize inside the request so startup failures reach the caller.
    ready ??= createBC7F({ locateFile: () => wasmUrl }).then((module) => {
      module._initialize();
      return module;
    });
    module = await ready;
    const levels: { width: number; height: number; offset: number; bytes: number }[] = [];
    let bytes = 0,
      w = width,
      h = height;
    do {
      const size = Math.ceil(w / 4) * Math.ceil(h / 4) * 16;
      levels.push({ width: w, height: h, offset: bytes, bytes: size });
      bytes += size;
      if (w === 1 && h === 1) break;
      w = Math.max(1, w >> 1);
      h = Math.max(1, h >> 1);
    } while (w > 0 && h > 0);
    source = module._malloc(pixels.byteLength);
    output = module._malloc(bytes);
    if (!source || !output) throw new Error("BC7 texture allocation failed");
    module.HEAPU8.set(new Uint8Array(pixels), source);
    for (let i = 0; i < levels.length; i++) {
      const level = levels[i]!;
      module._encode(source, level.width, level.height, output + level.offset, 0);
      const following = levels[i + 1];
      if (!following) break;
      next = module._malloc(following.width * following.height * 4);
      if (!next) throw new Error("BC7 mipmap allocation failed");
      module._mip(source, level.width, level.height, next);
      module._free(source);
      source = next;
      next = 0;
    }
    const encoded = module.HEAPU8.slice(output, output + bytes).buffer;
    globalThis.postMessage({ encoded, levels }, { transfer: [encoded] });
  } catch (error) {
    globalThis.postMessage({ error: String(error) }, {});
  } finally {
    if (source) module!._free(source);
    if (output) module!._free(output);
    if (next) module!._free(next);
  }
};
