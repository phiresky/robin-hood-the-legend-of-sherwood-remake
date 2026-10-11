import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { createHash } from "node:crypto";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

// Rebind local gameplay after reviewed mesh splits. These mappings select the
// structural component that owns the room; decorative pieces do not own doors.
const rooms = {
  "york-riverside-stone-storehouse": "building-000--component-closed-masonry",
  "york-riverside-storehouse-timber-shed": "building-005--component-closed-wall-body",
};
const ids = ["york-castle-west-gatehouse", "york-market-roofed-stone-well", ...Object.keys(rooms)];
const index = JSON.parse(await fs.readFile("library/3d-assets/index.json", "utf8"));
const edits = [];
for (const id of ids) {
  const entry = index.assets.find((entry) => entry.id === id);
  assert.ok(entry);
  const bytes = await fs.readFile(`library/3d-assets/${entry.descriptor}`);
  const descriptorSha256 = createHash("sha256").update(bytes).digest("hex");
  assert.equal(descriptorSha256, entry.descriptor_sha256);
  const descriptor = JSON.parse(bytes);
  const gameplay = structuredClone(descriptor.gameplay);
  assert.equal(gameplay.collision, "parts");
  const nodes = new Set(descriptor.parts.map((part) => part.node));
  const replacements = (node) => {
    const parts = descriptor.parts.filter((part) => part.source_node === node);
    assert.ok(parts.length && parts.every((part) => part.obstacle_local_game), node);
    return parts.map((part) => part.node);
  };
  gameplay.sightOrder = Object.fromEntries(
    Object.entries(gameplay.sightOrder).flatMap(([node, order]) =>
      (nodes.has(node) ? [node] : replacements(node)).map((replacement) => [replacement, order]),
    ),
  );
  if (gameplay.movementClearances) {
    gameplay.movementClearances = gameplay.movementClearances.flatMap((surface) =>
      nodes.has(surface.node)
        ? [surface]
        : replacements(surface.node).map((node) => ({
            ...structuredClone(surface),
            id: `${surface.id}:${node}`,
            node,
          })),
    );
  }
  const room = rooms[id];
  if (room) {
    assert.ok(nodes.has(room));
    const source = descriptor.parts.find((part) => part.node === room).source_node;
    const rebind = (value) => {
      if (!value || typeof value !== "object") return;
      if (value.node === source) value.node = room;
      for (const child of Object.values(value)) rebind(child);
    };
    rebind(gameplay.interiors);
    rebind(gameplay.masks);
  }
  validateAssetGameplay(gameplay, descriptor);
  edits.push({ asset: id, descriptorSha256, gameplay });
}
const output = await fs.mkdtemp("work/map-compile/york-component-gameplay-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
console.log(JSON.stringify({ output, assets: edits.map((edit) => edit.asset), published: false }));
