/** Validate a frame crop before decoding or extracting its pixels. */
export function spriteAtlasRect(
  value: unknown,
  width: number,
  height: number,
): [number, number, number, number] {
  if (!Array.isArray(value) || value.length !== 4 || value.some((n) => !Number.isSafeInteger(n)))
    throw new Error("Invalid sprite atlas rectangle");
  const [x, y, w, h] = value as [number, number, number, number];
  if (x < 0 || y < 0 || w <= 0 || h <= 0 || x + w > width || y + h > height)
    throw new Error("Sprite atlas rectangle is outside the image");
  return [x, y, w, h];
}
