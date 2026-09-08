// Render a GLB as-is (Y-up assumed) from four orbit views, with its bounds.
//
//   node src/glb-preview.ts <file.glb> [--out preview.png] [--size 400]
import fs from "node:fs/promises";
import sharp from "sharp";
import { bounds, loadGlb } from "./mesh.ts";
import { orbitView, render } from "./render.ts";

async function main() {
  const argv = process.argv.slice(2);
  const file = argv[0];
  if (!file || file.startsWith("--")) throw new Error("usage: <file.glb> [--out png] [--size px]");
  const get = (flag: string) => {
    const i = argv.indexOf(`--${flag}`);
    return i >= 0 ? argv[i + 1] : undefined;
  };
  const size = Number(get("size") ?? 400);
  const out = get("out") ?? file.replace(/\.glb$/i, "") + "-preview.png";
  const mesh = await loadGlb(file);
  // glTF Y-up -> scene Z-up: (x, y, z) -> (x, -z, y)
  const p = mesh.positions;
  const zup = new Float32Array(p.length);
  for (let i = 0; i < p.length; i += 3) {
    zup[i] = p[i]!;
    zup[i + 1] = -p[i + 2]!;
    zup[i + 2] = p[i + 1]!;
  }
  const b = bounds(p);
  console.log(
    `${file}: ${p.length / 3} verts, ${mesh.indices.length / 3} tris, ${mesh.textures.length} texture(s); bounds x ${b.min[0].toFixed(2)}..${b.max[0].toFixed(2)} y ${b.min[1].toFixed(2)}..${b.max[1].toFixed(2)} z ${b.min[2].toFixed(2)}..${b.max[2].toFixed(2)}`,
  );
  const bz = bounds(zup);
  const center: [number, number, number] = [
    (bz.min[0] + bz.max[0]) / 2,
    (bz.min[1] + bz.max[1]) / 2,
    (bz.min[2] + bz.max[2]) / 2,
  ];
  const extent = Math.max(bz.max[0] - bz.min[0], bz.max[1] - bz.min[1], bz.max[2] - bz.min[2]);
  const tiles: sharp.OverlayOptions[] = [];
  for (const [i, yaw] of [0, 90, 180, 270].entries()) {
    const img = render([{ mesh, positions: zup }], orbitView(center, yaw, 30, size, size, (size * 0.8) / extent));
    tiles.push({
      input: await sharp(img, { raw: { width: size, height: size, channels: 4 } })
        .flatten({ background: "#303030" })
        .png()
        .toBuffer(),
      left: i * size,
      top: 0,
    });
  }
  await sharp({ create: { width: 4 * size, height: size, channels: 3, background: "#202020" } })
    .composite(tiles)
    .png()
    .toFile(out);
  await fs.access(out);
  console.log(`wrote ${out}`);
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
