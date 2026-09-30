import type { TexturePixels } from "./texture-pixel-cache.ts";

/** Separable area filter: average columns once, then integrate arbitrary row footprints. */
export class TextureAreaFilter {
  readonly width: number;
  readonly height: number;
  private readonly sums: Float32Array;

  constructor(source: TexturePixels, width: number) {
    if (
      !Number.isInteger(width) ||
      width < 1 ||
      source.width < 1 ||
      source.height < 1 ||
      source.data.length !== source.width * source.height * 4
    )
      throw new Error("Invalid texture filter dimensions");
    this.width = width;
    this.height = source.height;
    this.sums = new Float32Array(width * (source.height + 1) * 4);
    const span = source.width / width;
    for (let y = 0; y < source.height; y++) {
      for (let x = 0; x < width; x++) {
        const start = x * span,
          end = (x + 1) * span;
        const index = ((y + 1) * width + x) * 4;
        for (let c = 0; c < 4; c++) this.sums[index + c] = this.sums[index + c - width * 4]!;
        for (let sx = Math.floor(start); sx < Math.ceil(end); sx++) {
          const weight = (Math.min(end, sx + 1) - Math.max(start, sx)) / span;
          const pixel = (y * source.width + sx) * 4;
          const alpha = source.data[pixel + 3]! / 255;
          for (let c = 0; c < 3; c++)
            this.sums[index + c] = this.sums[index + c]! + source.data[pixel + c]! * alpha * weight;
          this.sums[index + 3] = this.sums[index + 3]! + source.data[pixel + 3]! * weight;
        }
      }
    }
  }

  get byteLength() {
    return this.sums.byteLength;
  }

  private integral(y: number, component: number, repeat: boolean): number {
    const stride = this.width * 4;
    const total = this.sums[this.height * stride + component]!;
    let offset = 0;
    if (repeat) {
      const cycles = Math.floor(y / this.height);
      offset = cycles * total;
      y -= cycles * this.height;
    } else {
      if (y <= 0) return y * this.sums[stride + component]!;
      if (y >= this.height)
        return (
          total + (y - this.height) * (total - this.sums[(this.height - 1) * stride + component]!)
        );
    }
    const row = Math.floor(y),
      fraction = y - row;
    const base = this.sums[row * stride + component]!;
    return offset + base + fraction * (this.sums[(row + 1) * stride + component]! - base);
  }

  /** Output is premultiplied RGBA, allowing alpha-safe material blending before unpremultiplying. */
  sampleRow(center: number, footprint: number, repeat: boolean, target: Float32Array): void {
    if (
      !Number.isFinite(center) ||
      !Number.isFinite(footprint) ||
      footprint <= 0 ||
      target.length !== this.width * 4
    )
      throw new Error("Invalid texture filter footprint");
    const start = center - footprint / 2,
      end = center + footprint / 2;
    for (let i = 0; i < target.length; i++)
      target[i] = (this.integral(end, i, repeat) - this.integral(start, i, repeat)) / footprint;
  }
}
