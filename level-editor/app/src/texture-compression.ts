import * as THREE from "three";

type MaterialWithMap = THREE.Material & { map: THREE.Texture | null };
type Encoded = {
  encoded: ArrayBuffer;
  levels: { width: number; height: number; offset: number; bytes: number }[];
};

function textureImage(texture: THREE.Texture): HTMLImageElement | ImageBitmap | null {
  const image = texture.image;
  if (typeof HTMLImageElement !== "undefined" && image instanceof HTMLImageElement) return image;
  if (typeof ImageBitmap !== "undefined" && image instanceof ImageBitmap) return image;
  return null;
}

/** Find base-color textures whose alpha and UV mapping can be preserved. */
function candidatesFor(root: THREE.Object3D) {
  const materials = new Set<MaterialWithMap>();
  root.traverse((node) => {
    const mesh = node as THREE.Mesh;
    for (const material of Array.isArray(mesh.material) ? mesh.material : [mesh.material]) {
      if (material) materials.add(material as MaterialWithMap);
    }
  });
  const candidates = new Map<THREE.Texture, MaterialWithMap[]>();
  const excluded = new Set<THREE.Texture>();
  for (const material of materials) {
    const texture = material.map;
    if (texture) {
      if (
        material.transparent ||
        material.alphaTest > 0 ||
        material.userData.foliage_physical_opacity
      )
        excluded.add(texture);
      const users = candidates.get(texture) ?? [];
      users.push(material);
      candidates.set(texture, users);
    }
    // A texture shared with a non-color slot must keep its existing representation.
    for (const [key, value] of Object.entries(material))
      if (key !== "map" && value instanceof THREE.Texture) excluded.add(value);
  }
  for (const texture of candidates.keys()) {
    const image = textureImage(texture);
    if (
      excluded.has(texture) ||
      texture instanceof THREE.CompressedTexture ||
      texture.colorSpace !== THREE.SRGBColorSpace ||
      !image ||
      !Number.isInteger(image.width) ||
      !Number.isInteger(image.height) ||
      image.width % 4 ||
      image.height % 4 ||
      image.width * image.height < 65536 ||
      image.width * image.height > 4096 * 4096 ||
      texture.flipY
    )
      candidates.delete(texture);
  }
  return candidates;
}

type Job = {
  texture: THREE.Texture;
  resolve: (texture: THREE.CompressedTexture | null) => void;
  reject: (reason: unknown) => void;
};

/** Owned by one map load. Two workers overlap encoding while bounding pixel-buffer memory. */
export class MapTextureCompressor {
  private gl: WebGL2RenderingContext | null | undefined;
  private workers: Worker[] = [];
  private idle: Worker[] = [];
  private queue: Job[] = [];
  private cache = new Map<THREE.Texture, Promise<THREE.CompressedTexture | null>>();
  private running = new Map<Worker, Job>();
  private disposed = false;
  private awaiting = new Map<Worker, (reason: unknown) => void>();
  private abort = () => this.dispose(this.signal?.reason);
  private progress?: (active: number) => void;
  private signal?: AbortSignal;
  constructor(progress?: (active: number) => void, signal?: AbortSignal) {
    this.progress = progress;
    this.signal = signal;
    signal?.addEventListener("abort", this.abort, { once: true });
  }
  private initialize() {
    if (this.gl !== undefined) return !!this.gl;
    this.gl = null;
    if (typeof Worker === "undefined" || typeof OffscreenCanvas === "undefined") return false;
    const gl = new OffscreenCanvas(1, 1).getContext("webgl2", {
      antialias: false,
      premultipliedAlpha: false,
    });
    if (!gl) return false;
    if (!gl.getExtension("EXT_texture_compression_bptc")) {
      gl.getExtension("WEBGL_lose_context")?.loseContext();
      return false;
    }
    this.gl = gl;
    for (let i = 0; i < Math.min(2, navigator.hardwareConcurrency || 2); i++) {
      const worker = new Worker(new URL("./bc7f-worker.ts", import.meta.url), { type: "module" });
      worker.onerror = (event) => this.dispose(new Error(event.message));
      this.workers.push(worker);
      this.idle.push(worker);
    }
    return true;
  }
  async compress(root: THREE.Object3D) {
    const candidates = candidatesFor(root);
    if (!candidates.size) return;
    this.signal?.throwIfAborted();
    if (this.disposed) throw new Error("Texture encoder is disposed");
    if (!this.initialize()) return;
    await Promise.all(
      [...candidates].map(async ([texture, users]) => {
        let pending = this.cache.get(texture);
        if (!pending) {
          pending = new Promise((resolve, reject) => this.queue.push({ texture, resolve, reject }));
          this.cache.set(texture, pending);
        }
        this.pump();
        const compressed = await pending;
        if (compressed)
          for (const material of users) {
            material.map = compressed;
            material.needsUpdate = true;
          }
      }),
    );
  }
  private pump() {
    while (!this.disposed && this.idle.length && this.queue.length) {
      const worker = this.idle.pop()!,
        job = this.queue.shift()!;
      this.running.set(worker, job);
      this.progress?.(this.running.size);
      void this.run(worker, job.texture)
        .then(job.resolve, job.reject)
        .finally(() => {
          this.running.delete(worker);
          if (!this.disposed) {
            this.idle.push(worker);
            this.pump();
            this.progress?.(this.running.size);
          }
        });
    }
  }
  private async run(
    worker: Worker,
    texture: THREE.Texture,
  ): Promise<THREE.CompressedTexture | null> {
    // Let the loading dialog paint before doing synchronous pixel extraction.
    await new Promise((resolve) => setTimeout(resolve, 0));
    this.signal?.throwIfAborted();
    if (this.disposed) throw new Error("Texture encoder is disposed");
    const gl = this.gl!;
    const image = textureImage(texture)!;
    const { width, height } = image;
    const gpu = gl.createTexture(),
      framebuffer = gl.createFramebuffer();
    let pixels: Uint8Array<ArrayBuffer>;
    try {
      gl.bindTexture(gl.TEXTURE_2D, gpu);
      gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, false);
      gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, false);
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, gl.RGBA, gl.UNSIGNED_BYTE, image);
      gl.bindFramebuffer(gl.FRAMEBUFFER, framebuffer);
      gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, gpu, 0);
      if (gl.checkFramebufferStatus(gl.FRAMEBUFFER) !== gl.FRAMEBUFFER_COMPLETE)
        throw new Error("Texture readback framebuffer is incomplete");
      pixels = new Uint8Array(width * height * 4);
      gl.readPixels(0, 0, width, height, gl.RGBA, gl.UNSIGNED_BYTE, pixels);
      if (gl.getError() !== gl.NO_ERROR) throw new Error("Texture readback failed");
    } finally {
      gl.bindFramebuffer(gl.FRAMEBUFFER, null);
      gl.deleteFramebuffer(framebuffer);
      gl.deleteTexture(gpu);
    }
    // Opaque materials can still use alpha as ownership data: leave those untouched.
    for (let i = 3; i < pixels.length; i += 4) if (pixels[i] !== 255) return null;
    const result = await new Promise<Encoded>((resolve, reject) => {
      this.awaiting.set(worker, reject);
      worker.onmessage = ({ data }: MessageEvent<Encoded & { error?: string }>) => {
        this.awaiting.delete(worker);
        if (data.error) reject(new Error(data.error));
        else resolve(data);
      };
      worker.postMessage({ pixels: pixels.buffer, width, height }, [pixels.buffer]);
    });
    this.signal?.throwIfAborted();
    if (this.disposed) throw new Error("Texture encoder is disposed");
    const mipmaps = result.levels.map((level) => ({
      width: level.width,
      height: level.height,
      data: new Uint8Array(result.encoded, level.offset, level.bytes),
    }));
    const compressed = new THREE.CompressedTexture(mipmaps, width, height, THREE.RGBA_BPTC_Format);
    THREE.Texture.prototype.copy.call(compressed, texture);
    compressed.source = new THREE.Source({ width, height });
    compressed.mipmaps = mipmaps;
    compressed.format = THREE.RGBA_BPTC_Format;
    compressed.type = THREE.UnsignedByteType;
    compressed.generateMipmaps = false;
    compressed.flipY = false;
    compressed.needsUpdate = true;
    texture.dispose();
    return compressed;
  }
  dispose(reason: unknown = new DOMException("Texture encoding cancelled", "AbortError")) {
    if (this.disposed) return;
    this.disposed = true;
    this.signal?.removeEventListener("abort", this.abort);
    for (const job of [...this.queue, ...this.running.values()]) job.reject(reason);
    for (const reject of this.awaiting.values()) reject(reason);
    this.awaiting.clear();
    this.queue = [];
    this.running.clear();
    this.cache.clear();
    for (const worker of this.workers) worker.terminate();
    this.workers = [];
    this.idle = [];
    this.gl?.getExtension("WEBGL_lose_context")?.loseContext();
    this.gl = null;
  }
}
