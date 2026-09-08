// Bake a 3D level document back into game files.
//
//   node src/bake.ts --map york [--doc library/scenes/york.level3d.json] [--out work/york-bake]
//       [--fill proc|synth] [--ambiance Day]
//
// Reconstructs the map's volumes and textures exactly like volumes.ts (the
// textures are always taken at the original positions), then places every
// object of the document with its transform (duplicates share their
// source's faces, deleted and hidden ones are left out), renders the map
// from the game camera with the software rasterizer at full resolution and
// writes what the game reads into <out>/Data/Levels/:
//   <Ambiance>/<map>.map.png     the rendered map
//   <Ambiance>/<map>.min.png     the minimap (same size as the original)
//   <map>.rhp.json               the level with the objects' obstacles moved
// Masks and everything else in the level are carried through unchanged
// (todo: masks from the id buffer, patch states, other ambiances).
// Without --doc (or with an unmodified document) the output reproduces
// the original map, which is the round-trip test.
import fs from "node:fs/promises";
import { pathToFileURL } from "node:url";
import crypto from "node:crypto";
import { readDocument, pathComponent } from "./inputs.ts";
import path from "node:path";
import sharp from "sharp";
import {
  applyAffineMatrix,
  parseLevel3D,
  groupObstacles,
  partMatrix,
  transformedObstacle,
  type Level3D,
  type MapCamera,
  type SightObstacle,
} from "@rle/shared";
import { libraryDir, workDir, datadirPath, loadEnvironment } from "./env.ts";
import type { MeshData } from "./mesh.ts";
import { mapView, render } from "./render.ts";
import { groundToScene } from "@rle/shared";
import {
  reconstruct,
  type Fill,
  type Geometry,
  type ReconstructOptions,
} from "./volumes.ts";

/** the document's objects as one placed mesh (positions transformed, tiles shared) */
function placeObjects(
  g: Geometry,
  uvs: Float32Array,
  cam: MapCamera,
  doc: Level3D,
): { positions: Float32Array; indices: Uint32Array; uvs: Float32Array } {
  const facesOf = new Map<number, number[]>();
  g.faces.forEach((f, i) => {
    const list = facesOf.get(f.obstacle) ?? [];
    list.push(i);
    facesOf.set(f.obstacle, list);
  });
  const pos: number[] = [];
  const uv: number[] = [];
  const idx: number[] = [];
  const hiddenGroups = new Set(
    doc.groups.filter((g) => g.hidden).map((g) => g.id),
  );
  for (const o of doc.objects) {
    if (o.hidden || (o.group && hiddenGroups.has(o.group))) continue;
    const faces = facesOf.get(o.source.obstacle);
    if (!faces)
      throw new Error(
        `document object ${o.id} has no reconstructed geometry for source obstacle ${o.source.obstacle}`,
      );
    const m = partMatrix(cam, doc, o);
    const remap = new Map<number, number>();
    for (const f of faces) {
      for (const t of g.faces[f]!.tris) {
        for (let k = 0; k < 3; k++) {
          const v = g.tris[t * 3 + k]!;
          let local = remap.get(v);
          if (local === undefined) {
            local = pos.length / 3;
            remap.set(v, local);
            const p = applyAffineMatrix(m, [
              g.positions[v * 3]!,
              g.positions[v * 3 + 1]!,
              g.positions[v * 3 + 2]!,
            ]);
            pos.push(p[0], p[1], p[2]);
            uv.push(uvs[v * 2]!, uvs[v * 2 + 1]!);
          }
          idx.push(local);
        }
      }
    }
  }
  return {
    positions: Float32Array.from(pos),
    indices: Uint32Array.from(idx),
    uvs: Float32Array.from(uv),
  };
}

export interface BakeOptions {
  textures?: ReconstructOptions["textures"];
  map: string;
  document?: string;
  output?: string;
  fill?: Fill;
  ambiance?: string;
}
export async function bake(options: BakeOptions): Promise<void> {
  const map = pathComponent(options.map, "map");
  const ambiance = pathComponent(options.ambiance ?? "Day", "ambiance");
  const outDir =
    options.output ?? path.join(workDir, `${map.toLowerCase()}-bake`);
  const docPath =
    options.document ??
    path.join(libraryDir, "scenes", `${map.toLowerCase()}.level3d.json`);
  const input = await readDocument(docPath, options.document !== undefined);
  // Structural validation precedes expensive reconstruction; source-index
  // validation follows once the source level is available.
  const parsed = input === undefined ? undefined : parseLevel3D(input, { map });
  const r = await reconstruct(map, {
    textures: options.textures,
    fill: options.fill,
    fallbackFill: options.fill === undefined,
  });
  const { g, tex, cam, size, level } = r;

  let doc: Level3D;
  if (parsed !== undefined) {
    const sourceSha256 = crypto
      .createHash("sha256")
      .update(JSON.stringify(level))
      .digest("hex");
    let glbSha256: string | undefined;
    if (parsed.provenance?.glb_sha256) {
      const glbFile = path.resolve(path.dirname(docPath), parsed.glb);
      glbSha256 = crypto
        .createHash("sha256")
        .update(await fs.readFile(glbFile))
        .digest("hex");
    }
    doc = parseLevel3D(parsed, {
      map,
      level,
      sourceSha256,
      glbSha256,
      scene: { size, camera: cam },
    });
    console.log(`document ${docPath}: ${doc.objects.length} objects`);
  } else {
    // Absent optional default: every obstacle where it is (round-trip)
    const ids = new Set(g.faces.map((f) => f.obstacle));
    const groupOf = groupObstacles(level.sight_obstacles, g.terraceIds);
    doc = {
      version: 1,
      map,
      size,
      camera: cam,
      glb: `${map.toLowerCase()}-volumes.scene.glb`,
      objects: [...ids]
        .sort((a, b) => a - b)
        .map((i) => ({
          id: `${g.terraceIds.has(i) ? "terrace" : "building"}-${String(i).padStart(3, "0")}`,
          kind: g.terraceIds.has(i)
            ? ("terrace" as const)
            : ("building" as const),
          node: `${g.terraceIds.has(i) ? "terrace" : "building"}-${String(i).padStart(3, "0")}`,
          source: { map, obstacle: i },
          obstacle: level.sight_obstacles[i]!,
          transform: { dx: 0, dy: 0, dz: 0, rot_deg: 0 },
          group: groupOf.has(i)
            ? `group-${String(groupOf.get(i)!).padStart(3, "0")}`
            : undefined,
        })),
      groups: [...new Set(groupOf.values())].map((r) => ({
        id: `group-${String(r).padStart(3, "0")}`,
        transform: { dx: 0, dy: 0, dz: 0, rot_deg: 0 },
      })),
    };
    console.log(
      `no document at ${docPath}: baking the unmodified reconstruction (${doc.objects.length} objects)`,
    );
  }

  // ── render the map from the game camera at full resolution ──
  const placed = placeObjects(g, tex.uvs, cam, doc);
  const atlasTex = {
    width: tex.atlas.width,
    height: tex.atlas.height,
    data: tex.atlas.rgba,
  };
  const volumes: MeshData = {
    positions: placed.positions,
    indices: placed.indices,
    colors: null,
    uvs: placed.uvs,
    triTexture: Int32Array.from({ length: placed.indices.length / 3 }, () => 0),
    textures: [atlasTex],
  };
  const [w, h] = size;
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
    textures: [
      {
        width: tex.ground.width,
        height: tex.ground.height,
        data: tex.ground.rgba,
      },
    ],
  };
  const view = { ...mapView(cam, [0, 0, w, h], 1), unlit: true };
  const t0 = Date.now();
  const rgba = render(
    [
      { mesh: ground, positions: ground.positions },
      { mesh: volumes, positions: volumes.positions },
    ],
    view,
  );
  console.log(
    `rendered ${view.width}x${view.height} in ${((Date.now() - t0) / 1000).toFixed(1)} s`,
  );

  const levelsDir = path.join(outDir, "Data", "Levels");
  const ambDir = path.join(levelsDir, ambiance);
  await fs.mkdir(ambDir, { recursive: true });
  const mapPngOut = path.join(ambDir, `${map.toLowerCase()}.map.png`);
  await sharp(rgba, {
    raw: { width: view.width, height: view.height, channels: 4 },
    limitInputPixels: false,
  })
    .flatten({ background: "#000" })
    .png({ compressionLevel: 6 })
    .toFile(mapPngOut);
  // minimap: the original's size if we can find it, else 1/14
  let minSize: [number, number] = [Math.round(w / 14), Math.round(h / 14)];
  try {
    const orig = path.join(
      datadirPath(),
      "Data",
      "Levels",
      ambiance,
      `${map.toLowerCase()}.min.png`,
    );
    const meta = await sharp(orig).metadata();
    if (meta.width && meta.height) minSize = [meta.width, meta.height];
  } catch (error) {
    console.warn(
      `original minimap unavailable; using ${minSize.join("x")}: ${String(error)}`,
    );
  }
  await sharp(mapPngOut)
    .resize(minSize[0], minSize[1], { fit: "fill" })
    .png()
    .toFile(path.join(ambDir, `${map.toLowerCase()}.min.png`));

  // ── level: the objects' obstacles moved, everything else carried through ──
  const obstacles: SightObstacle[] = level.sight_obstacles.map((o) => ({
    ...o,
  }));
  const hiddenGroups = new Set(
    doc.groups.filter((g) => g.hidden).map((g) => g.id),
  );
  const seen = new Set<number>();
  const extra: SightObstacle[] = [];
  for (const o of doc.objects) {
    if (o.hidden || (o.group && hiddenGroups.has(o.group))) continue;
    const t = transformedObstacle(doc, o);
    if (!seen.has(o.source.obstacle)) {
      seen.add(o.source.obstacle);
      obstacles[o.source.obstacle] = t;
    } else extra.push(t);
  }
  // deleted objects: their obstacle stays in place by index but empty (indices are referenced by masks, patches and elevation lines)
  for (let i = 0; i < obstacles.length; i++) {
    if (!seen.has(i) && g.faces.some((f) => f.obstacle === i))
      obstacles[i] = { ...obstacles[i]!, points: [] };
  }
  const out = { ...level, sight_obstacles: [...obstacles, ...extra] };
  await fs.writeFile(
    path.join(levelsDir, `${map.toLowerCase()}.rhp.json`),
    JSON.stringify(out),
  );
  console.log(
    `wrote ${mapPngOut}, ${map.toLowerCase()}.min.png (${minSize.join("x")}) and ${map.toLowerCase()}.rhp.json (${obstacles.length} + ${extra.length} obstacles) under ${outDir}`,
  );
}

async function main() {
  loadEnvironment();
  const { parseArgs } = await import("node:util");
  const { values } = parseArgs({
    options: {
      map: { type: "string" },
      doc: { type: "string" },
      out: { type: "string" },
      fill: { type: "string" },
      ambiance: { type: "string" },
    },
  });
  if (!values.map)
    throw new Error(
      "usage: --map <name> [--doc file] [--out dir] [--fill proc|synth|smear|none]",
    );
  if (values.fill && !["proc", "synth", "smear", "none"].includes(values.fill))
    throw new Error(`unknown fill ${values.fill}`);
  await bake({
    textures: process.env.TEXTURE_SYNTHESIS
      ? { synthBinary: process.env.TEXTURE_SYNTHESIS }
      : undefined,
    map: values.map,
    document: values.doc,
    output: values.out,
    fill: values.fill as Fill | undefined,
    ambiance: values.ambiance,
  });
}
if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(process.argv[1]).href
) {
  main().catch((error) => {
    console.error(error);
    process.exitCode = 1;
  });
}
