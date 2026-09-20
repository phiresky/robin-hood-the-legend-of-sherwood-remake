import type { SightObstacle, Vec3 } from "@rle/shared";

/** Directions run clockwise from north. View azimuth is measured from south. */
export function viewedDirection(direction: number, azimuth: number): number {
  return ((Math.round(direction + azimuth * 8 / Math.PI) % 16) + 16) % 16;
}

/** Intersect the map ray (x, mapY + z, z) with the support's top plane. */
export function placementHeight(x: number, mapY: number, obstacle: SightObstacle): number {
  const a = obstacle.points[0];
  if (!a) throw new Error("Entity support has no points");
  for (let i = 1; i + 1 < obstacle.points.length; i++) {
    const b = obstacle.points[i]!, c = obstacle.points[i + 1]!;
    const ux = b.x - a.x, uy = b.y - a.y, uz = b.z_top - a.z_top;
    const vx = c.x - a.x, vy = c.y - a.y, vz = c.z_top - a.z_top;
    const nx = uy * vz - uz * vy, ny = uz * vx - ux * vz, nz = ux * vy - uy * vx;
    if (Math.abs(nz) < 1e-7) continue;
    if (Math.abs(ny + nz) < 1e-7) throw new Error("Entity support is parallel to the map ray");
    return (nx * (a.x - x) + ny * (a.y - mapY) + nz * a.z_top) / (ny + nz);
  }
  throw new Error("Entity support has no valid top plane");
}

/** Lift a source pixel onto a front cylinder shell or its top cap. The source
 * camera projects it back to exactly (right, up), including protruding clothing.
 * TODO: Unseen underside/back detail needs additional elevation artwork. */
export function cylinderPixel(right: number, up: number, radius: number, height: number, elevation: number): Vec3 {
  const sin = Math.sin(elevation), cos = Math.cos(elevation);
  const shell = Math.sqrt(Math.max(0, radius * radius - right * right));
  const depth = Math.min(shell, (height * cos - up) / sin);
  return [right, (up + depth * sin) / cos, depth];
}

/** RGB565 key colors in converted sprites encode transparency and ground shadow.
 * RGBA-authored sprites keep their own colors and alpha. */
export function decodeSpritePixels(pixels: Uint8ClampedArray, legacy: boolean): void {
  if (!legacy) return;
  for (let i = 0; i < pixels.length; i += 4) {
    const packed = ((pixels[i]! >> 3) << 11) | ((pixels[i + 1]! >> 2) << 5) | (pixels[i + 2]! >> 3);
    if (packed === 0x07c0 || packed === 0x001f) pixels.fill(0, i, i + 4);
  }
}

/** Extract authored shadow coverage before removing the color keys. */
export function spriteShadowPixels(pixels: Uint8ClampedArray, legacy: boolean): Uint8ClampedArray | null {
  if (!legacy) return null;
  const mask = new Uint8ClampedArray(pixels.length);
  let found = false;
  for (let i = 0; i < pixels.length; i += 4) {
    const packed = ((pixels[i]! >> 3) << 11) | ((pixels[i + 1]! >> 2) << 5) | (pixels[i + 2]! >> 3);
    if (packed === 0x001f && pixels[i + 3]! > 0) {
      mask[i] = mask[i + 1] = mask[i + 2] = 255;
      mask[i + 3] = pixels[i + 3]!;
      found = true;
    }
  }
  return found ? mask : null;
}

export function spriteShadowStyle(ambiance: string): { color: number; opacity: number } {
  // The game darkens sRGB by 40% (10% in fog). This viewport writes sRGB to
  // the canvas before blending; the linear-target renderer converts that
  // same retention factor to linear alpha instead.
  return { color: 0x000000, opacity: ambiance === "Fog" ? 0.1 : 0.4 };
}
