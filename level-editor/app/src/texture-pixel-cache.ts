export interface TexturePixels {
  data: Uint8Array;
  width: number;
  height: number;
}

/** Byte-bounded LRU of CPU pixels; callers own copies and can dispose textures independently. */
export class TexturePixelCache {
  private entries = new Map<string, TexturePixels>();
  private bytes = 0;
  readonly maxBytes: number;
  constructor(maxBytes: number) {
    this.maxBytes = maxBytes;
  }
  get(key: string): TexturePixels | undefined {
    const value = this.entries.get(key);
    if (!value) return undefined;
    this.entries.delete(key);
    this.entries.set(key, value);
    return { ...value, data: value.data.slice() };
  }
  set(key: string, value: TexturePixels) {
    const previous = this.entries.get(key);
    if (previous) this.bytes -= previous.data.byteLength;
    this.entries.delete(key);
    if (value.data.byteLength > this.maxBytes) return;
    while (this.bytes + value.data.byteLength > this.maxBytes) {
      const oldest = this.entries.keys().next().value!;
      this.bytes -= this.entries.get(oldest)!.data.byteLength;
      this.entries.delete(oldest);
    }
    this.entries.set(key, { ...value, data: value.data.slice() });
    this.bytes += value.data.byteLength;
  }
}
