// Assemble every reconstructed asset of a map into one 3D scene.
//
//   node src/scene.ts --map york [--out library/scenes] [--exclude id,id]
//       [--min-iou 0.5] [--ground-scale 0.5] [--texture-size 1024] [--render]
//
// Assets tagged "exclude" (e.g. wall segments the sweep caught) and ids
// passed with --exclude are left out.
// Writes <out>/<map>.scene.json (SceneDoc: camera + placements, our own
// format), <out>/<map>-ground.jpg (downscaled map used as the ground
// texture) and <out>/<map>.scene.glb (all models under one Y-up root, plus
// the textured ground quad). --render also produces a litmus test in
// work/<map>-scene/: the assembled scene rendered from the map's own camera
// next to the original map.
import fs from "node:fs/promises";
import path from "node:path";
import sharp from "sharp";
import { Document, NodeIO, type Node } from "@gltf-transform/core";
import { ALL_EXTENSIONS, KHRMaterialsUnlit } from "@gltf-transform/extensions";
import { mergeDocuments, textureCompress, unpartition } from "@gltf-transform/functions";
import type {
  AssetDescriptor,
  LibraryIndexEntry,
  MapCamera,
  SceneDoc,
  ScenePlacement,
} from "@rle/shared";
import { groundToScene } from "@rle/shared";
import { libraryDir, workDir } from "./env.ts";
import { findMapPng, loadProtoLevel } from "./asset-writer.ts";
import { fitMapCamera } from "./map-camera.ts";
import { loadGlb, transformPositions, type MeshData } from "./mesh.ts";
import { mapView, orbitView, render, type RenderInstance } from "./render.ts";

/** Z-up scene frame -> glTF Y-up: (x, y, z) -> (x, z, -y) = rotation of -90° about X */
const ZUP_TO_YUP: [number, number, number, number] = [-Math.SQRT1_2, 0, 0, Math.SQRT1_2];

async function collectAssets(map: string, exclude: Set<string>, minIou: number) {
  const index: LibraryIndexEntry[] = JSON.parse(
    await fs.readFile(path.join(libraryDir, "index.json"), "utf8"),
  );
  const assets: AssetDescriptor[] = [];
  const skipped: string[] = [];
  const descs: AssetDescriptor[] = [];
  for (const e of index) {
    if (e.source_map.toLowerCase() !== map.toLowerCase()) continue;
    const desc: AssetDescriptor = JSON.parse(
      await fs.readFile(path.join(libraryDir, e.id, "asset.json"), "utf8"),
    );
    if (desc.model) descs.push(desc);
  }
  // a block (merged detection) replaces its members when it fits at least
  // as well as they do on average: silhouette IoU plus colour agreement
  const byId = new Map(descs.map((d) => [d.id, d]));
  const fitScore = (d: AssetDescriptor) =>
    d.model!.fit_iou + (d.model!.fit_appearance ?? 0);
  const inBlock = new Map<string, string>();
  const losingBlocks = new Set<string>();
  for (const d of descs) {
    if (!d.merged_from) continue;
    if (d.model!.fit_iou < minIou || exclude.has(d.id) || d.tags.includes("exclude")) {
      losingBlocks.add(d.id);
      continue;
    }
    const members = d.merged_from.map((m) => byId.get(m)).filter((m): m is AssetDescriptor => !!m);
    const memberScore = members.length
      ? members.reduce((a, m) => a + fitScore(m), 0) / members.length
      : 0;
    if (fitScore(d) >= memberScore) {
      for (const m of d.merged_from) inBlock.set(m, d.id);
    } else {
      losingBlocks.add(d.id);
      skipped.push(`${d.id} (block ${fitScore(d).toFixed(2)} < members ${memberScore.toFixed(2)})`);
    }
  }
  for (const desc of descs) {
    const e = desc;
    if (losingBlocks.has(e.id)) continue;
    if (exclude.has(e.id) || desc.tags.includes("exclude")) {
      skipped.push(`${e.id} (excluded)`);
      continue;
    }
    const block = inBlock.get(e.id);
    if (block) {
      skipped.push(`${e.id} (in block ${block})`);
      continue;
    }
    if (desc.model!.fit_iou < minIou) {
      skipped.push(`${e.id} (IoU ${desc.model!.fit_iou.toFixed(2)})`);
      continue;
    }
    assets.push(desc);
  }
  return { assets, skipped };
}

function addGround(doc: Document, root: Node, cam: MapCamera, w: number, h: number, jpeg: Buffer) {
  const c0 = groundToScene(cam, 0, 0);
  const c1 = groundToScene(cam, w, 0);
  const c2 = groundToScene(cam, w, h);
  const c3 = groundToScene(cam, 0, h);
  const buffer = doc.getRoot().listBuffers()[0] ?? doc.createBuffer();
  const position = doc
    .createAccessor("ground-position")
    .setType("VEC3")
    .setArray(new Float32Array([...c0, ...c1, ...c2, ...c3]))
    .setBuffer(buffer);
  const uv = doc
    .createAccessor("ground-uv")
    .setType("VEC2")
    .setArray(new Float32Array([0, 0, 1, 0, 1, 1, 0, 1]))
    .setBuffer(buffer);
  const indices = doc
    .createAccessor("ground-indices")
    .setType("SCALAR")
    .setArray(new Uint16Array([0, 2, 1, 0, 3, 2]))
    .setBuffer(buffer);
  const texture = doc.createTexture("ground").setImage(jpeg).setMimeType("image/jpeg");
  const material = doc
    .createMaterial("ground")
    .setBaseColorTexture(texture)
    .setMetallicFactor(0)
    .setRoughnessFactor(1)
    .setDoubleSided(true);
  const prim = doc
    .createPrimitive()
    .setAttribute("POSITION", position)
    .setAttribute("TEXCOORD_0", uv)
    .setIndices(indices)
    .setMaterial(material);
  const mesh = doc.createMesh("ground").addPrimitive(prim);
  root.addChild(doc.createNode("ground").setMesh(mesh));
}

async function exportGlb(
  file: string,
  assets: AssetDescriptor[],
  cam: MapCamera,
  size: [number, number],
  groundJpeg: Buffer,
  textureSize: number | null,
) {
  const io = new NodeIO().registerExtensions(ALL_EXTENSIONS);
  const doc = new Document();
  const scene = doc.createScene("scene");
  const root = doc.createNode("map").setRotation(ZUP_TO_YUP);
  scene.addChild(root);
  addGround(doc, root, cam, size[0], size[1], groundJpeg);

  for (const a of assets) {
    const src = await io.read(path.join(libraryDir, a.id, a.model!.glb));
    if (textureSize) {
      await src.transform(
        textureCompress({ encoder: sharp, resize: [textureSize, textureSize], targetFormat: "jpeg" }),
      );
    }
    const map = mergeDocuments(doc, src);
    const p = a.model!.placement;
    const holder = doc
      .createNode(a.id)
      .setTranslation(p.position)
      .setRotation(p.rotation)
      .setScale([p.scale, p.scale, p.scale]);
    root.addChild(holder);
    for (const srcScene of src.getRoot().listScenes()) {
      const merged = map.get(srcScene) as ReturnType<Document["createScene"]>;
      for (const child of merged.listChildren()) holder.addChild(child);
      merged.dispose();
    }
  }
  // the textures carry the map's baked lighting: mark every material unlit
  // so viewers don't shade them a second time
  const unlit = doc.createExtension(KHRMaterialsUnlit);
  for (const m of doc.getRoot().listMaterials()) {
    m.setExtension("KHR_materials_unlit", unlit.createUnlit());
  }
  // merged documents carry one buffer per source; GLB allows a single one
  await doc.transform(unpartition());
  await io.write(file, doc);
}

async function litmusRender(
  map: string,
  assets: AssetDescriptor[],
  cam: MapCamera,
  size: [number, number],
  mapPng: Buffer,
  scale: number,
) {
  const [w, h] = size;
  const instances: RenderInstance[] = [];
  for (const a of assets) {
    const mesh: MeshData = await loadGlb(path.join(libraryDir, a.id, a.model!.glb));
    const p = a.model!.placement;
    instances.push({
      mesh,
      positions: transformPositions(mesh.positions, p.rotation, p.scale, p.position),
    });
  }
  // ground as a textured quad
  const { data, info } = await sharp(mapPng)
    .resize(Math.round(w * scale), Math.round(h * scale))
    .ensureAlpha()
    .raw()
    .toBuffer({ resolveWithObject: true });
  const c = [groundToScene(cam, 0, 0), groundToScene(cam, w, 0), groundToScene(cam, w, h), groundToScene(cam, 0, h)];
  const ground: MeshData = {
    positions: Float32Array.from(c.flat()),
    indices: Uint32Array.from([0, 2, 1, 0, 3, 2]),
    colors: null,
    uvs: Float32Array.from([0, 0, 1, 0, 1, 1, 0, 1]),
    triTexture: Int32Array.from([0, 0]),
    textures: [{ width: info.width, height: info.height, data }],
  };
  // ground first so buildings win depth ties
  instances.unshift({ mesh: ground, positions: ground.positions });

  const view = mapView(cam, [0, 0, w, h], scale);
  view.unlit = true;
  console.log(`rendering ${instances.length - 1} models + ground at ${view.width}x${view.height}`);
  const rgba = render(instances, view);
  const rendered = await sharp(rgba, { raw: { width: view.width, height: view.height, channels: 4 } })
    .flatten({ background: "#000" })
    .png()
    .toBuffer();
  const original = await sharp(mapPng).resize(view.width, view.height).png().toBuffer();
  const outDir = path.join(workDir, `${map.toLowerCase()}-scene`);
  await fs.mkdir(outDir, { recursive: true });
  await fs.writeFile(path.join(outDir, "render.png"), rendered);
  await sharp({
    create: { width: view.width * 2, height: view.height, channels: 3, background: "#000" },
  })
    .composite([
      { input: original, left: 0, top: 0 },
      { input: rendered, left: view.width, top: 0 },
    ])
    .png()
    .toFile(path.join(outDir, "compare.png"));
  console.log(`wrote ${outDir}/render.png and compare.png`);

  // two orbit views of the whole scene to judge the 3D result
  const c0 = groundToScene(cam, w / 2, h / 2);
  const extent = Math.max(w, h / Math.sin((cam.elevation_deg * Math.PI) / 180));
  const views: [number, number][] = [
    [35, 40],
    [215, 30],
  ];
  for (const [i, [yaw, pitch]] of views.entries()) {
    const vw = 1600;
    const vh = 1000;
    const view = orbitView(c0, yaw, pitch, vw, vh, (vw * 0.9) / extent);
    view.unlit = true;
    const img = render(instances, view);
    const file = path.join(outDir, `view-${i + 1}.png`);
    await sharp(img, { raw: { width: vw, height: vh, channels: 4 } })
      .flatten({ background: "#000" })
      .png()
      .toFile(file);
    console.log(`wrote ${file} (yaw ${yaw}°, pitch ${pitch}°)`);
  }
}

async function main() {
  const argv = process.argv.slice(2);
  const get = (flag: string): string | undefined => {
    const i = argv.indexOf(`--${flag}`);
    return i >= 0 ? argv[i + 1] : undefined;
  };
  const map = get("map");
  if (!map) throw new Error("usage: --map <name> [--out dir] [--exclude a,b] [--min-iou x] [--render]");
  const outDir = get("out") ?? path.join(libraryDir, "scenes");
  const exclude = new Set((get("exclude") ?? "").split(",").filter(Boolean));
  const minIou = Number(get("min-iou") ?? 0.5);
  const groundScale = Number(get("ground-scale") ?? 0.5);
  const textureSize = get("texture-size") ? Number(get("texture-size")) : null;

  const level = await loadProtoLevel(map);
  const fit = fitMapCamera(level);
  const cam: MapCamera = { kind: fit.kind, elevation_deg: fit.elevation_deg };
  const dayPath = await findMapPng("Day", map);
  if (!dayPath) throw new Error(`no Day map for ${map}`);
  const mapPng = await sharp(dayPath).png().toBuffer();
  const meta = await sharp(mapPng).metadata();
  const size: [number, number] = [meta.width!, meta.height!];

  const { assets, skipped } = await collectAssets(map, exclude, minIou);
  console.log(`${map}: ${assets.length} reconstructed assets` + (skipped.length ? `; skipped ${skipped.join(", ")}` : ""));
  if (assets.length === 0) throw new Error(`no reconstructed assets for ${map} in the library`);

  await fs.mkdir(outDir, { recursive: true });
  const groundFile = `${map.toLowerCase()}-ground.jpg`;
  const groundJpeg = await sharp(mapPng)
    .resize(Math.round(size[0] * groundScale), Math.round(size[1] * groundScale))
    .jpeg({ quality: 85 })
    .toBuffer();
  await fs.writeFile(path.join(outDir, groundFile), groundJpeg);

  const doc: SceneDoc = {
    version: 1,
    map,
    size,
    camera: cam,
    ground: { texture: groundFile, rect: [0, 0, size[0], size[1]] },
    placements: assets.map((a): ScenePlacement => a.model!.placement),
  };
  const jsonFile = path.join(outDir, `${map.toLowerCase()}.scene.json`);
  await fs.writeFile(jsonFile, JSON.stringify(doc, null, 2));
  console.log(`wrote ${jsonFile}`);

  const glbFile = path.join(outDir, `${map.toLowerCase()}.scene.glb`);
  await exportGlb(glbFile, assets, cam, size, groundJpeg, textureSize);
  const st = await fs.stat(glbFile);
  console.log(`wrote ${glbFile} (${(st.size / 1e6).toFixed(1)} MB)`);

  if (argv.includes("--render")) await litmusRender(map, assets, cam, size, mapPng, 0.5);
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
