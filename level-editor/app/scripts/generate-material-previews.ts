import { writeFile } from "node:fs/promises";
import { encode } from "fast-png";
import type { DataTexture } from "three";
import { terrainMaterials } from "../../shared/src/terrain-materials.ts";
import { terrainMaterialTexture, terrainTexture } from "../src/terrain-texture.ts";

// Bake full-resolution designs once so browsing never generates them on the UI thread.
const tileSize = 128;
const columns = 8;
const materials = terrainMaterials.map((material) => material.id);
const bases = ["grass", "dirt", "water", "paved"] as const;
const rows = Math.ceil((materials.length + bases.length) / columns);
const width = columns * tileSize;
const height = rows * tileSize;
const atlas = new Uint8Array(width * height * 4);

function addTile(texture: DataTexture, index: number) {
  try {
    const source = texture.image.data;
    const sourceWidth = texture.image.width;
    const sourceHeight = texture.image.height;
    if (!(source instanceof Uint8Array) || source.length !== sourceWidth * sourceHeight * 4)
      throw new Error(`Unexpected RGBA pixels for ${texture.name}`);
    if (sourceWidth % tileSize || sourceHeight % tileSize)
      throw new Error(`Texture dimensions must be multiples of ${tileSize}`);
    const scaleX = sourceWidth / tileSize;
    const scaleY = sourceHeight / tileSize;
    const samples = scaleX * scaleY;
    const tileX = (index % columns) * tileSize;
    const tileY = Math.floor(index / columns) * tileSize;
    // Box-filter the exact sRGB preview pixels, including the procedural decorations.
    for (let y = 0; y < tileSize; y++) {
      for (let x = 0; x < tileSize; x++) {
        const destination = ((tileY + y) * width + tileX + x) * 4;
        for (let channel = 0; channel < 4; channel++) {
          let sum = 0;
          for (let dy = 0; dy < scaleY; dy++)
            for (let dx = 0; dx < scaleX; dx++)
              sum += source[((y * scaleY + dy) * sourceWidth + x * scaleX + dx) * 4 + channel];
          atlas[destination + channel] = Math.round(sum / samples);
        }
      }
    }
  } finally {
    texture.dispose();
  }
}

for (const [index, id] of materials.entries()) addTile(terrainMaterialTexture(id), index);
for (const [index, base] of bases.entries())
  addTile(terrainTexture(base), materials.length + index);

const directory = new URL("../src/terrain-textures/", import.meta.url);
await writeFile(
  new URL("material-previews.png", directory),
  encode({ width, height, data: atlas, channels: 4 }),
);
await writeFile(
  new URL("material-previews.json", directory),
  JSON.stringify({ tileSize, columns, materials, bases }, null, 2) + "\n",
);
console.log(
  `Wrote ${materials.length} material previews and ${bases.length} base tiles (${width} × ${height}).`,
);
