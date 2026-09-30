import * as THREE from "three";
import { unzlibSync } from "fflate";

import { terrainMaterial, type CustomTerrainMaterial } from "../../shared/src/terrain-materials.ts";
import { TexturePixelCache } from "./texture-pixel-cache.ts";
import tiles from "./terrain-textures/tiles.json" with { type: "json" };

// Retain CPU pixels only; every caller owns its texture and mutable pixel array.
const pixelCache = new TexturePixelCache(64 * 1024 * 1024);
function textureFromPixels(data: Uint8Array, width: number, height: number) {
  const texture = new THREE.DataTexture(data, width, height);
  texture.wrapS = texture.wrapT = THREE.RepeatWrapping;
  texture.colorSpace = THREE.SRGBColorSpace;
  texture.magFilter = THREE.LinearFilter;
  texture.minFilter = THREE.LinearMipmapLinearFilter;
  texture.generateMipmaps = true;
  texture.needsUpdate = true;
  return texture;
}

/** Bundled indexed art is decoded synchronously, including for immediate map baking. */
export function terrainTexture(kind: "grass" | "dirt" | "water" | "paved", feather = false) {
  const key = `base:${kind}:${feather}`;
  const cached = pixelCache.get(key);
  if (cached) return textureFromPixels(cached.data, cached.width, cached.height);
  const tile = tiles[kind];
  const palette = atob(tile.palette),
    pixels = unzlibSync(Uint8Array.from(atob(tile.pixelsZlib), (c) => c.charCodeAt(0)));
  const width = feather ? 256 : tile.size;
  const data = new Uint8Array(width * tile.size * 4);
  for (let i = 0; i < width * tile.size; i++) {
    const color = pixels[Math.floor(i / width) * tile.size + (i % width)]! * 3;
    for (let c = 0; c < 3; c++) data[i * 4 + c] = palette.charCodeAt(color + c);
    const u = (i % width) / (width - 1);
    data[i * 4 + 3] = feather
      ? Math.min(255, Math.min(u, 1 - u) * (kind === "water" ? 12800 : 2200))
      : 255;
  }
  pixelCache.set(key, { data, width, height: tile.size });
  return textureFromPixels(data, width, tile.size);
}

/**
 * Synchronous color previews for named materials, using the existing base tile detail.
 * TODO: Refine procedural surface details with authored texture artwork.
 * Caller owns the texture and must dispose it when rebuilding geometry.
 */
export function terrainMaterialTexture(
  id: string,
  customMaterials: readonly CustomTerrainMaterial[] = [],
  feather = false,
) {
  const material = terrainMaterial(id, customMaterials);
  const key = JSON.stringify([
    "material",
    material.id,
    material.category,
    material.color,
    material.textureBase,
    feather,
  ]);
  const cached = pixelCache.get(key);
  if (cached) {
    const texture = textureFromPixels(cached.data, cached.width, cached.height);
    texture.name = `terrain-material:${id}`;
    return texture;
  }
  const texture = terrainTexture(material.textureBase, feather);
  const data = texture.image.data as Uint8Array;
  const rgb = [1, 3, 5].map((offset) => parseInt(material.color.slice(offset, offset + 2), 16));
  let total = 0;
  for (let i = 0; i < data.length; i += 4) {
    total += 0.2126 * data[i]! + 0.7152 * data[i + 1]! + 0.0722 * data[i + 2]!;
  }
  const mean = total / (data.length / 4);
  for (let i = 0; i < data.length; i += 4) {
    const luminance = 0.2126 * data[i]! + 0.7152 * data[i + 1]! + 0.0722 * data[i + 2]!;
    const detail = mean > 0 ? luminance / mean : 1;
    for (let channel = 0; channel < 3; channel++) {
      data[i + channel] = Math.min(255, Math.round(rgb[channel]! * detail));
    }
  }
  if (material.category !== "custom") {
    addMaterialDetails(data, texture.image.width, texture.image.height, id);
  }
  texture.name = `terrain-material:${id}`;
  pixelCache.set(key, { data, width: texture.image.width, height: texture.image.height });
  return texture;
}

/** Tile-local deterministic patterns keep previews and exported bakes identical. */
function addMaterialDetails(data: Uint8Array, width: number, height: number, id: string) {
  const hash = (x: number, y: number, salt = 0) => {
    let n = Math.imul(x + 317, 374761393) ^ Math.imul(y + 811, 668265263) ^ salt;
    n = Math.imul(n ^ (n >>> 13), 1274126177);
    return ((n ^ (n >>> 16)) >>> 0) / 4294967295;
  };
  const paint = (x: number, y: number, rgb: readonly number[], opacity = 1) => {
    // Repeat decorations crossing tile boundaries; preserve any edge feather alpha.
    const offset = (((y + height) % height) * width + ((x + width) % width)) * 4;
    for (let c = 0; c < 3; c++)
      data[offset + c] = Math.round(data[offset + c]! * (1 - opacity) + rgb[c]! * opacity);
  };
  const ellipse = (
    cx: number,
    cy: number,
    rx: number,
    ry: number,
    color: readonly number[],
    opacity = 1,
  ) => {
    for (let y = Math.floor(cy - ry); y <= Math.ceil(cy + ry); y++) {
      for (let x = Math.floor(cx - rx); x <= Math.ceil(cx + rx); x++) {
        const d = ((x - cx) / rx) ** 2 + ((y - cy) / ry) ** 2;
        if (d < 1) {
          const shade = 0.8 + 0.24 * (1 - (y - cy) / ry) + 0.05 * hash(x, y);
          paint(
            x,
            y,
            color.map((value) => Math.min(255, value * shade)),
            opacity * Math.min(1, (1 - d) * 5),
          );
        }
      }
    }
  };
  const line = (
    x: number,
    y: number,
    dx: number,
    dy: number,
    color: readonly number[],
    opacity = 1,
  ) => {
    const steps = Math.max(Math.abs(dx), Math.abs(dy), 1);
    for (let i = 0; i <= steps; i++)
      paint(Math.round(x + (dx * i) / steps), Math.round(y + (dy * i) / steps), color, opacity);
  };
  const cells = (spacing: number, draw: (x: number, y: number, random: number) => void) => {
    const nx = Math.max(1, Math.round(width / spacing)),
      ny = Math.max(1, Math.round(height / spacing));
    for (let y = 0; y < ny; y++)
      for (let x = 0; x < nx; x++) {
        draw(
          Math.floor(((x + 0.2 + hash(x, y, 1) * 0.6) * width) / nx),
          Math.floor(((y + 0.2 + hash(x, y, 2) * 0.6) * height) / ny),
          hash(x, y, 3),
        );
      }
  };
  if (/cobblestone|flagstone|floor_paving/.test(id)) {
    const cellWidth = id === "path_flagstone" ? 48 : 24,
      cellHeight = id === "path_flagstone" ? 32 : 16;
    for (let y = 0; y < height; y++)
      for (let x = 0; x < width; x++) {
        const row = Math.floor(y / cellHeight),
          xx = x + ((row % 2) * cellWidth) / 2;
        const variation = hash(Math.floor(xx / cellWidth), row);
        if (id.endsWith("broken") && variation < 0.23) continue;
        const edge = Math.min(
          xx % cellWidth,
          cellWidth - (xx % cellWidth),
          y % cellHeight,
          cellHeight - (y % cellHeight),
        );
        if (edge < 2) paint(x, y, id.endsWith("mossy") ? [62, 77, 36] : [66, 61, 49], 0.8);
        else if (edge < 3) paint(x, y, [190, 185, 166], 0.4);
        else paint(x, y, [110 + variation * 70, 106 + variation * 66, 92 + variation * 60], 0.3);
      }
  }
  if (/stone|gravel|pebbles|rocky|water_ford/.test(id) && !/cobblestone|flagstone/.test(id)) {
    const large = /large|white_stone|rocky/.test(id),
      single = /single|white_stone/.test(id);
    const drawStone = (x: number, y: number, r: number) => {
      const radius = (large ? 11 : 3) * (0.7 + r);
      if (id === "water_white_stone")
        ellipse(x, y, radius * 1.8, radius * 1.2, [220, 238, 229], 0.7);
      ellipse(x + 2, y + 3, radius * 1.15, radius * 0.75, [35, 43, 36], 0.6);
      ellipse(x, y, radius, radius * 0.7, [130 + r * 45, 127 + r * 40, 112 + r * 35]);
    };
    if (single) drawStone(width / 2, height / 2, 0.6);
    else cells(large ? 44 : /gravel|pebbles/.test(id) ? 9 : 24, drawStone);
  }
  if (/water_white/.test(id)) {
    cells(20, (x, y, r) => {
      for (let dx = -7; dx <= 7; dx++) {
        const yy = y + Math.round(Math.sin(dx * 0.4 + r * 6) * 2);
        paint(x + dx, yy, [225, 243, 233], 0.6 + 0.2 * r);
      }
    });
  }
  if (id === "water_reeds" || id === "grass_tall" || id === "field_crops") {
    cells(id === "field_crops" ? 12 : 24, (x, y, r) => {
      for (let j = -2; j <= 2; j++) {
        line(x, y, j * 3, -7 - Math.round(r * 10), [95 + r * 30, 108 + r * 35, 40], 0.85);
        if (id === "water_reeds") ellipse(x + j * 3, y - 9 - r * 10, 1.3, 3, [92, 62, 34]);
      }
    });
  }
  if (id === "water_lily_pads")
    cells(30, (x, y, r) => {
      ellipse(x, y, 5 + r * 4, 3 + r * 3, [66, 105, 49]);
      line(x, y, 5, -2, [45, 73, 50]);
    });
  if (/leaves|forest_floor|straw|flowers|clover/.test(id))
    cells(14, (x, y, r) => {
      if (id === "grass_flowers") {
        for (const [dx, dy] of [
          [-2, 0],
          [2, 0],
          [0, -2],
          [0, 2],
        ])
          ellipse(x + dx!, y + dy!, 1.8, 1.8, r > 0.5 ? [227, 214, 171] : [182, 149, 185]);
        paint(x, y, [220, 177, 47]);
      } else if (id === "grass_clover") {
        for (const [dx, dy] of [
          [-2, 0],
          [2, 0],
          [0, -2],
        ])
          ellipse(x + dx!, y + dy!, 2, 2, [73, 113, 53]);
      } else if (id === "ground_straw") line(x, y, 9, Math.round(r * 8) - 4, [195, 167, 97], 0.8);
      else ellipse(x, y, 2 + r * 2, 1.5, [118 + r * 55, 79 + r * 36, 33], 0.8);
    });
  if (/mud|puddles|trampled|swamp/.test(id))
    cells(48, (x, y, r) => {
      if (r < 0.65)
        ellipse(
          x,
          y,
          9 + r * 15,
          5 + r * 9,
          id === "path_puddles" ? [123, 146, 140] : [77, 62, 40],
          0.5,
        );
    });
  if (id === "path_forest_roots")
    cells(60, (x, y, r) => {
      line(x, y, 25, Math.round(r * 25), [78, 57, 33], 0.9);
      line(x, y + 1, 25, Math.round(r * 25), [161, 127, 81], 0.7);
      line(x + 12, y + Math.round(r * 12), 6, -10, [85, 61, 37], 0.8);
    });
  if (/ploughed|cart_tracks|floor_planks/.test(id)) {
    for (let y = 0; y < height; y++)
      for (let x = 0; x < width; x++) {
        const groove =
          id === "path_cart_tracks"
            ? Math.abs(x - width * 0.28) < 2 || Math.abs(x - width * 0.72) < 2
            : x % (id === "floor_planks" ? 32 : 16) < 2;
        if (groove) paint(x, y, [65, 46, 28], 0.7);
        if (id === "floor_planks" && (y + Math.floor(x / 32) * 64) % 128 < 2)
          paint(x, y, [65, 46, 28], 0.6);
      }
  }
}
