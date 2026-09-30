/** Tint a small library thumbnail using the same luminance mapping as terrain textures. */
export function tintMaterialPreview(data: Uint8ClampedArray, color: string): void {
  if (!/^#[0-9a-f]{6}$/i.test(color) || !data.length || data.length % 4 !== 0)
    throw new Error("Material preview needs RGBA pixels and an RGB color");
  const rgb = [1, 3, 5].map((offset) => parseInt(color.slice(offset, offset + 2), 16));
  let total = 0;
  for (let i = 0; i < data.length; i += 4)
    total += 0.2126 * data[i]! + 0.7152 * data[i + 1]! + 0.0722 * data[i + 2]!;
  const mean = total / (data.length / 4);
  for (let i = 0; i < data.length; i += 4) {
    const luminance = 0.2126 * data[i]! + 0.7152 * data[i + 1]! + 0.0722 * data[i + 2]!;
    const detail = mean > 0 ? luminance / mean : 1;
    for (let c = 0; c < 3; c++) data[i + c] = Math.min(255, Math.round(rgb[c]! * detail));
  }
}
