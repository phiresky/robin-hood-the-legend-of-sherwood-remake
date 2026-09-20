// Decode authored character-occlusion bitmaps as geometry-review evidence.
import fs from 'node:fs/promises';
import path from 'node:path';
import sharp from 'sharp';

export function decodeMask(bytes: number[], width: number, height: number): Uint8Array {
  const out = new Uint8Array(width * height);
  let offset = 0;
  for (let y = 0; y < height; y++) {
    if (offset >= bytes.length) throw new Error(`Missing mask row ${y}`);
    const end = offset + 1 + bytes[offset]!;
    offset++;
    if (end > bytes.length) throw new Error(`Truncated mask row ${y}`);
    let x = 0;
    while (offset < end) {
      const control = bytes[offset++]!;
      const count = control & 127;
      if (!count) continue;
      const compressed = (control & 128) !== 0;
      if (offset + (compressed ? 1 : count) > end) throw new Error(`Truncated mask run ${y}`);
      for (let block = 0; block < count; block++) {
        const bits = bytes[offset + (compressed ? 0 : block)]!;
        for (let bit = 0; bit < 8; bit++, x++)
          if (x < width) out[y * width + x] = bits & (128 >> bit) ? 255 : 0;
      }
      offset += compressed ? 1 : count;
    }
    offset = end;
  }
  if (offset !== bytes.length) throw new Error('Trailing mask data');
  return out;
}

const [levelPath, sourcePath, outputPath, indices] = process.argv.slice(2);
if (!levelPath || !sourcePath || !outputPath)
  throw new Error('Usage: export-occlusion-masks.ts <level.rhp.json> <source.png> <fresh-output-dir> [mask-indices-comma-separated]');
const level = JSON.parse(await fs.readFile(levelPath, 'utf8')) as {
  masks: { box_top_left: [number, number]; box_size: [number, number]; mask_data: number[]; [key: string]: unknown }[];
};
const selected = indices ? indices.split(',').map(Number) : level.masks.map((_, i) => i);
const output = path.resolve(outputPath);
await fs.mkdir(output, { recursive: false });
const records = [];
for (const index of selected) {
  const mask = level.masks[index];
  if (!mask) throw new Error(`Unknown mask ${index}`);
  const [left, top] = mask.box_top_left, [width, height] = mask.box_size;
  const decoded = decodeMask(mask.mask_data, width, height);
  const folder = path.join(output, `mask-${String(index).padStart(3, '0')}`);
  await fs.mkdir(folder);
  await sharp(decoded, { raw: { width, height, channels: 1 } }).png().toFile(path.join(folder, 'mask.png'));
  const context = await sharp(sourcePath).extract({ left, top, width, height }).removeAlpha().png().toBuffer();
  await fs.writeFile(path.join(folder, 'context.png'), context);
  await sharp(context).joinChannel(Buffer.from(decoded), { raw: { width, height, channels: 1 } }).png().toFile(path.join(folder, 'cutout.png'));
  const { mask_data: _packed, ...metadata } = mask;
  records.push({ index, ...metadata, opaque_pixels: decoded.reduce((n, value) => n + Number(value !== 0), 0), folder: path.basename(folder) });
}
await fs.writeFile(path.join(output, 'masks.json'), JSON.stringify({
  level: path.resolve(levelPath), source: path.resolve(sourcePath), masks: records,
  meaning: 'White pixels occlude characters when this mask is active and its position/layer test passes. Masks are not automatically complete object segmentation or depth maps.',
}, null, 2) + '\n');
console.log(JSON.stringify({ output, masks: records.length }));
