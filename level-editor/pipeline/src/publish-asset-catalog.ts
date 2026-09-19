/** Publish named ownership beside an existing reconstruction without changing its mesh IDs. */
import { readFile, writeFile } from "node:fs/promises";
import { resolve, basename } from "node:path";
import {
  authoredAssetGroups, documentProvenance, IDENTITY_TRANSFORM,
  parseProtoLevel, parseSceneDoc, parseLevel3D, upgradeGeneratedAssetGroups,
  type Level3D, type Level3DObject,
} from "@rle/shared";

const [sceneArg, levelArg] = process.argv.slice(2);
if (!sceneArg || !levelArg) throw new Error("Usage: node publish-asset-catalog.ts <map-volumes.scene.json> <Map.rhp.json>");
const scenePath = resolve(sceneArg);
const scene = parseSceneDoc(JSON.parse(await readFile(scenePath, "utf8")));
const level = parseProtoLevel(JSON.parse(await readFile(resolve(levelArg), "utf8")));
const glbPath = scenePath.replace(/\.json$/, ".glb");
const bytes = await readFile(glbPath);
if (bytes.toString("ascii", 0, 4) !== "glTF" || bytes.readUInt32LE(16) !== 0x4e4f534a) throw new Error("Expected GLB with a JSON first chunk");
const gltf = JSON.parse(bytes.toString("utf8", 20, 20 + bytes.readUInt32LE(12))) as { nodes: { name?: string; mesh?: number }[] };
const nodes = new Set(gltf.nodes.filter(node => /^(building|terrace)-\d+$/.test(node.name ?? "")).map(node => node.name!));
const output = scenePath.replace(/-volumes\.scene\.json$/, ".level3d.json");
if (output === scenePath) throw new Error("Expected a -volumes.scene.json filename");
let previous: string | undefined;
try { previous = await readFile(output, "utf8"); }
catch (error) { if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error; }
let document: Level3D;
const provenance = await documentProvenance(level, bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) as ArrayBuffer);
if (previous) {
  document = parseLevel3D(JSON.parse(previous), { scene, level, nodes,
    sourceSha256: provenance.source_sha256, glbSha256: provenance.glb_sha256 });
  if (!upgradeGeneratedAssetGroups(document)) throw new Error("Existing document has custom ownership or edits; refusing to replace it");
} else {
  const objects: Level3DObject[] = [...nodes].sort().map(node => {
    const [kind, index] = node.split("-");
    const obstacle = Number(index);
    if (!level.sight_obstacles[obstacle]) throw new Error(`Missing obstacle ${obstacle}`);
    return { id: node, node, kind: kind as "building" | "terrace", source: { map: scene.map, obstacle },
      obstacle: level.sight_obstacles[obstacle]!, transform: { ...IDENTITY_TRANSFORM } };
  });
  const groups = authoredAssetGroups(scene.map, objects);
  if (!groups) throw new Error(`No authored catalog for ${scene.map}`);
  document = { version: 1, map: scene.map, size: scene.size, camera: scene.camera,
    glb: basename(glbPath), objects, groups };
}
document.provenance = provenance;
parseLevel3D(document, { scene, level, nodes, sourceSha256: provenance.source_sha256, glbSha256: provenance.glb_sha256 });
if (previous) await writeFile(output + ".before-asset-catalog", previous, { flag: "wx" });
await writeFile(output, JSON.stringify(document, null, 2) + "\n", { flag: previous ? "w" : "wx" });
console.log(JSON.stringify({ file: output, assets: document.groups.length, parts: document.objects.length }));
