// Presence/ownership inventory only: a definition is not proof of playable geometry.
// Run from level-editor. Saved placements, not unused library references, count as placed.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { gameplayOwnerDependencies } from "./gameplay-owner-dependencies.mjs";

const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const indexBytes = await fs.readFile("library/3d-assets/index.json");
const entries = JSON.parse(indexBytes).assets;
const descriptors = new Map();
const owners = new Map();
for (const entry of entries) {
  const bytes = await fs.readFile(`library/3d-assets/${entry.descriptor}`);
  assert.equal(hash(bytes), entry.descriptor_sha256, `Stale index: ${entry.id}`);
  const descriptor = JSON.parse(bytes);
  assert.equal(descriptor.id, entry.id);
  assert.ok(!descriptors.has(entry.id), `Duplicate asset: ${entry.id}`);
  descriptors.set(entry.id, { entry, descriptor });
  for (const part of descriptor.parts) {
    assert.equal(typeof part.node, "string");
    assert.ok(part.node.length);
    if (!descriptor.gameplay || !descriptor.source_map) continue;
    const key = JSON.stringify([descriptor.source_map, part.node]);
    const candidates = owners.get(key) ?? new Set();
    candidates.add(entry.id);
    owners.set(key, candidates);
  }
}
const placements = new Map();
const scenes = [];
for (const file of (await fs.readdir("library/scenes")).sort((a, b) => a.localeCompare(b))) {
  if (!file.endsWith(".rhlos-map.json")) continue;
  const bytes = await fs.readFile(`library/scenes/${file}`);
  const scene = JSON.parse(bytes);
  assert.ok(Array.isArray(scene.placements), `Unsupported scene storage: ${file}`);
  const used = new Set([
    ...scene.placements.flatMap((placement) => placement.assets),
    ...(scene.sceneAssets ?? []).map((asset) => asset.id),
    ...(scene.splines ?? [])
      .filter((spline) => spline.kind === "wall")
      .flatMap((spline) => [spline.asset, spline.cornerAsset])
      .filter((id) => id !== undefined),
  ]);
  for (const id of used) {
    assert.ok(descriptors.has(id), `Unknown placed asset ${id} in ${file}`);
    const maps = placements.get(id) ?? [];
    maps.push(file);
    placements.set(id, maps);
  }
  scenes.push({ file, sha256: hash(bytes), placedAssets: used.size });
}
const missing = [...descriptors.values()]
  .filter(({ descriptor }) => !descriptor.gameplay)
  .map(({ entry, descriptor }) => {
    const parts = descriptor.parts.map((part) => {
      const candidateOwners = [
        ...(owners.get(JSON.stringify([descriptor.source_map, part.node])) ?? []),
      ].sort((a, b) => a.localeCompare(b));
      return {
        node: part.node,
        candidateOwners,
        candidateDependencies: candidateOwners.map((id) =>
          gameplayOwnerDependencies(descriptors.get(id).descriptor, part.node),
        ),
      };
    });
    return {
      asset: entry.id,
      descriptorSha256: entry.descriptor_sha256,
      placedIn: placements.get(entry.id) ?? [],
      allPartsHaveCandidateOwners:
        parts.length > 0 && parts.every((part) => part.candidateOwners.length > 0),
      hasStateOwnedParts: parts.some((part) =>
        part.candidateDependencies.some((owner) => owner.movementTransitions.length > 0),
      ),
      parts,
    };
  });
const summary = {
  indexedAssets: entries.length,
  missingDefinitions: missing.length,
  missingPlacedDefinitions: missing.filter((asset) => asset.placedIn.length).length,
  allPartsHaveCandidateOwners: missing.filter((asset) => asset.allPartsHaveCandidateOwners).length,
  missingAssetsWithStateOwnedParts: missing.filter((asset) => asset.hasStateOwnedParts).length,
};
assert.equal(hash(await fs.readFile("library/3d-assets/index.json")), hash(indexBytes));
const output = await fs.mkdtemp("work/map-compile/gameplay-coverage-");
await fs.writeFile(
  `${output}/report.json`,
  JSON.stringify(
    {
      scope:
        "Current library definition presence, not gameplay completeness or scene pin validation",
      note: "Matching part names identify authoring candidates, not equivalent geometry or local frames. Reported initial/applied sight membership exposes known state dependencies; an empty list does not certify complete dependency independence.",
      indexSha256: hash(indexBytes),
      ...summary,
      scenes,
      missing,
    },
    null,
    2,
  ),
);
console.log(JSON.stringify({ output, ...summary }));
