/** Publish a refined GLB and refresh its document fingerprint together, retaining a backup. */
import { readFile, writeFile, rename, mkdir } from "node:fs/promises";
import { resolve, dirname, basename, join } from "node:path";
import { createHash } from "node:crypto";
import { parseLevel3D, parseSceneDoc } from "@rle/shared";

const [stagedArg, documentArg] = process.argv.slice(2);
if (!stagedArg || !documentArg) throw new Error("Usage: node publish-refined-map.ts <staged.glb> <map.level3d.json>");
const documentPath = resolve(documentArg);
const previousText = await readFile(documentPath, "utf8");
const document = parseLevel3D(JSON.parse(previousText));
const glbPath = join(dirname(documentPath), document.glb);
const oldGlb = await readFile(glbPath);
const newGlb = await readFile(resolve(stagedArg));
const hash = (data: Buffer) => createHash("sha256").update(data).digest("hex");
parseLevel3D(document, { glbSha256: hash(oldGlb) });
interface Gltf { nodes: { name?: string; children?: number[]; extras?: { part_name?: string; asset_group?: string } }[] }
const gltf = JSON.parse(newGlb.toString("utf8", 20, 20 + newGlb.readUInt32LE(12))) as Gltf;
const previousGltf = JSON.parse(oldGlb.toString("utf8", 20, 20 + oldGlb.readUInt32LE(12))) as Gltf;
const previousNodes = new Map(previousGltf.nodes.map(node => [node.name, node]));
const refinedNodes = new Map(gltf.nodes.map(node => [node.name, node]));
// Follow revised authored labels only when the saved label still matches its
// previous model. Names the map author changed remain theirs.
for (const part of document.objects) {
  const before = previousNodes.get(part.node)?.extras?.part_name;
  const after = refinedNodes.get(part.node)?.extras?.part_name;
  if (before && after && part.name === before) part.name = after;
}
for (const group of document.groups) {
  const before = previousGltf.nodes.find(node => node.extras?.asset_group === group.id)?.name;
  const after = gltf.nodes.find(node => node.extras?.asset_group === group.id)?.name;
  if (before && after && group.name === before) group.name = after;
}
const root = gltf.nodes.find(node => node.name === "map");
if (!root) throw new Error("Refined map lacks editor root");
const parts = (root.children ?? []).flatMap(i => {
  const group = gltf.nodes[i]!;
  return group.name === "ground" ? [] : (group.children ?? []).map(j => gltf.nodes[j]!.name!);
});
if (new Set(parts).size !== parts.length) throw new Error("Duplicate refined part IDs");
const nodes = new Set(parts);
document.provenance = { ...document.provenance, glb_sha256: hash(newGlb) };
const scene = parseSceneDoc(JSON.parse(await readFile(glbPath.replace(/\.glb$/, ".json"), "utf8")));
parseLevel3D(document, { scene, nodes, glbSha256: hash(newGlb) });
const backup = join(dirname(documentPath), "backups", hash(oldGlb).slice(0, 16));
await mkdir(backup, { recursive: true });
await writeFile(join(backup, basename(glbPath)), oldGlb);
await writeFile(join(backup, basename(documentPath)), previousText);
await writeFile(glbPath + ".pending", newGlb, { flag: "wx" });
await writeFile(documentPath + ".pending", JSON.stringify(document, null, 2) + "\n", { flag: "wx" });
await rename(glbPath + ".pending", glbPath);
try { await rename(documentPath + ".pending", documentPath); }
catch (error) { await writeFile(glbPath, oldGlb); throw error; }
console.log(JSON.stringify({ file: glbPath, document: documentPath, backup, parts: parts.length, groups: document.groups.length }));
