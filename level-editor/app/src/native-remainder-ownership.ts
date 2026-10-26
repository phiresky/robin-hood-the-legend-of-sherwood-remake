import type { NativePixels } from "./native-state-presentation.ts";
import type { NativeShadowKey } from "../../shared/src/native-state-presentation.ts";

/** Tracks opaque kept artwork in draw order, never moving that artwork above later source pixels. */
export class NativeRemainderOwnership {
  private readonly retained: Uint8Array;
  private readonly width: number;
  private readonly height: number;
  constructor(width: number, height: number) {
    this.width = width;
    this.height = height;
    this.retained = new Uint8Array(width * height);
  }
  /** A shadow modifies its receiver; it does not establish a new opaque owner. */
  private painted(source: NativePixels, at: number, shadow?: NativeShadowKey) {
    return (
      source.data[at + 3] !== 0 &&
      !(shadow && shadow.rgb.every((value, c) => source.data[at + c] === value))
    );
  }
  native(source: NativePixels, x: number, y: number, kept: boolean, shadow?: NativeShadowKey) {
    x = Math.floor(x);
    y = Math.floor(y);
    for (let sy = Math.max(0, -y); sy < Math.min(source.height, this.height - y); sy++) {
      for (let sx = Math.max(0, -x); sx < Math.min(source.width, this.width - x); sx++) {
        const at = (sy * source.width + sx) * 4;
        if (!this.painted(source, at, shadow)) continue;
        const to = (sy + y) * this.width + sx + x;
        if (kept && source.data[at + 3] === 255) this.retained[to] = 1;
        else if (!kept && source.data[at + 3] === 255) this.retained[to] = 0;
      }
    }
  }
  /** Only inferred spill outside this element's source coverage yields to already visible kept art. */
  physical(
    layer: NativePixels,
    source: NativePixels,
    x: number,
    y: number,
    shadow?: NativeShadowKey,
  ): NativePixels {
    if (layer.width !== this.width || layer.height !== this.height)
      throw new Error("Native remainder ownership layer size differs");
    x = Math.floor(x);
    y = Math.floor(y);
    let data: Uint8Array | undefined;
    for (let p = 0; p < this.retained.length; p++) {
      const at = p * 4;
      if (layer.data[at + 3] === 0) continue;
      if (this.retained[p]) {
        const sx = (p % this.width) - x,
          sy = Math.floor(p / this.width) - y;
        const own =
          sx >= 0 &&
          sy >= 0 &&
          sx < source.width &&
          sy < source.height &&
          this.painted(source, (sy * source.width + sx) * 4, shadow);
        if (!own) {
          data ??= new Uint8Array(layer.data);
          data.fill(0, at, at + 4);
          continue;
        }
      }
      if (layer.data[at + 3] === 255) this.retained[p] = 0;
    }
    return data ? { width: layer.width, height: layer.height, data } : layer;
  }
}
