import fs from "node:fs/promises";
import path from "node:path";
import sharp from "sharp";
import { groundToScene, type MapCamera } from "@rle/shared";
import { workDir } from "./env.ts";
import type { Geometry } from "./volume-geometry.ts";
import type { Textured } from "./volume-fill.ts";
import type { MeshData, TextureData } from "./mesh.ts";
import { mapView, orbitView, render } from "./render.ts";
async function downscale(
  tex: { width: number; height: number; rgba: Buffer },
  maxSide: number,
): Promise<TextureData> {
  const s = Math.min(1, maxSide / Math.max(tex.width, tex.height));
  if (s >= 1) return { width: tex.width, height: tex.height, data: tex.rgba };
  const w = Math.round(tex.width * s);
  const h = Math.round(tex.height * s);
  const data = await sharp(tex.rgba, {
    raw: { width: tex.width, height: tex.height, channels: 4 },
    limitInputPixels: false,
  })
    .resize(w, h, { kernel: "nearest" })
    .raw()
    .toBuffer();
  return { width: w, height: h, data };
}

export async function renders(
  map: string,
  g: Geometry,
  tex: Textured,
  cam: MapCamera,
  size: [number, number],
  mapPng: Buffer,
  stem: string,
  closeups: [number, number][],
  debugFill: boolean,
  options: {
    workDirectory?: string;
    closeupSpan?: number;
    closeupYaws?: number[];
  } = {},
) {
  const [w, h] = size;
  const scale = 0.5;
  const atlasTex = await downscale(
    debugFill ? tex.debugAtlas : tex.atlas,
    8192,
  );
  const groundTex = await downscale(tex.ground, 4096);
  const volumes: MeshData = {
    positions: g.positions,
    indices: g.tris,
    colors: null,
    uvs: tex.uvs,
    triTexture: Int32Array.from({ length: g.tris.length / 3 }, () => 0),
    textures: [atlasTex],
  };
  const c = [
    groundToScene(cam, 0, 0),
    groundToScene(cam, w, 0),
    groundToScene(cam, w, h),
    groundToScene(cam, 0, h),
  ];
  const ground: MeshData = {
    positions: Float32Array.from(c.flat()),
    indices: Uint32Array.from([0, 2, 1, 0, 3, 2]),
    colors: null,
    uvs: Float32Array.from([0, 0, 1, 0, 1, 1, 0, 1]),
    triTexture: Int32Array.from([0, 0]),
    textures: [groundTex],
  };
  const instances = [
    { mesh: ground, positions: ground.positions },
    { mesh: volumes, positions: volumes.positions },
  ];
  const outDir = path.join(
    options.workDirectory ?? workDir,
    `${map.toLowerCase()}-scene`,
  );
  await fs.mkdir(outDir, { recursive: true });

  const view = { ...mapView(cam, [0, 0, w, h], scale), unlit: true };
  const rendered = await sharp(render(instances, view), {
    raw: { width: view.width, height: view.height, channels: 4 },
  })
    .flatten({ background: "#000" })
    .png()
    .toBuffer();
  const original = await sharp(mapPng)
    .resize(view.width, view.height)
    .png()
    .toBuffer();
  await sharp({
    create: {
      width: view.width * 2,
      height: view.height,
      channels: 3,
      background: "#000",
    },
  })
    .composite([
      { input: original, left: 0, top: 0 },
      { input: rendered, left: view.width, top: 0 },
    ])
    .png()
    .toFile(path.join(outDir, `${stem}-compare.png`));

  const c0 = groundToScene(cam, w / 2, h / 2);
  const extent = Math.max(w, h / Math.sin((cam.elevation_deg * Math.PI) / 180));
  for (const [i, [yaw, pitch]] of (
    [
      [35, 40],
      [215, 30],
    ] as [number, number][]
  ).entries()) {
    const vw = 1600;
    const vh = 1000;
    const v = {
      ...orbitView(c0, yaw, pitch, vw, vh, (vw * 0.9) / extent),
      unlit: true,
    };
    await sharp(render(instances, v), {
      raw: { width: vw, height: vh, channels: 4 },
    })
      .flatten({ background: "#000" })
      .png()
      .toFile(path.join(outDir, `${stem}-view-${i + 1}.png`));
  }
  console.log(`wrote ${outDir}/${stem}-compare.png and ${stem}-view-{1,2}.png`);

  // close-ups: one row per map point, one column per yaw, 500 map px across
  if (closeups.length > 0) {
    const vw = 640;
    const vh = 480;
    // --closeup-span px: map pixels across a close-up (default 500)
    const span = options.closeupSpan ?? 500;
    // --closeup-yaws a,b,… (default 35,125,215,305); yaw 0 with the map elevation is the map camera
    const yaws = options.closeupYaws ?? [35, 125, 215, 305];
    const composites: sharp.OverlayOptions[] = [];
    let svg = `<svg width="${yaws.length * vw}" height="${closeups.length * vh}" xmlns="http://www.w3.org/2000/svg">`;
    for (const [i, [x, y]] of closeups.entries()) {
      const c = groundToScene(cam, x, y);
      for (const [j, yaw] of yaws.entries()) {
        const v = {
          ...orbitView(
            c,
            yaw,
            yaw === 0 ? cam.elevation_deg : 30,
            vw,
            vh,
            vw / span,
          ),
          unlit: true,
        };
        composites.push({
          input: await sharp(render(instances, v), {
            raw: { width: vw, height: vh, channels: 4 },
          })
            .flatten({ background: "#000" })
            .png()
            .toBuffer(),
          left: j * vw,
          top: i * vh,
        });
        svg += `<text x="${j * vw + 6}" y="${i * vh + 16}" font-size="13" font-family="sans-serif" fill="#0f0">${x},${y} yaw ${yaw}</text>`;
      }
    }
    svg += "</svg>";
    composites.push({ input: Buffer.from(svg), left: 0, top: 0 });
    await sharp({
      create: {
        width: yaws.length * vw,
        height: closeups.length * vh,
        channels: 3,
        background: "#000",
      },
    })
      .composite(composites)
      .png()
      .toFile(
        path.join(outDir, `${stem}-closeups${debugFill ? "-fill" : ""}.png`),
      );
    console.log(
      `wrote ${outDir}/${stem}-closeups${debugFill ? "-fill" : ""}.png`,
    );
  }
}
