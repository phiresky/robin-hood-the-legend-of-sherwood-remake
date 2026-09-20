/** Publish a refined GLB and refresh its document fingerprint together, retaining a backup. */
import { readFile, writeFile, rename, mkdir } from "node:fs/promises";
import { resolve, dirname, basename, join } from "node:path";
import { createHash } from "node:crypto";
import { parseLevel3D, parseSceneDoc } from "@rle/shared";
import { mergeRefinedGroups, type AuthoredGltf } from "./refined-map-groups.ts";

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
const gltf = JSON.parse(newGlb.toString("utf8", 20, 20 + newGlb.readUInt32LE(12))) as AuthoredGltf;
const previousGltf = JSON.parse(oldGlb.toString("utf8", 20, 20 + oldGlb.readUInt32LE(12))) as AuthoredGltf;
const { nodes, ...groupUpdates } = mergeRefinedGroups(document, previousGltf, gltf);
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
console.log(JSON.stringify({ file: glbPath, document: documentPath, backup, parts: nodes.size, groups: document.groups.length, ...groupUpdates }));
