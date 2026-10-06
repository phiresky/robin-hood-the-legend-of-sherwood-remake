import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { loadSceneModel } from "../pipeline/src/scene-assets.ts";
import { maskRecoveryMesh } from "../pipeline/src/mask-recovery-mesh.ts";
import { partMatrix } from "../shared/src/level3d.ts";
import { gameToScene } from "../shared/src/scene.ts";
import { gltfToScene, sceneToGame } from "../shared/src/geometry.ts";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

// Review independently owned contacts. Nothing in this tool runs during export.
const [stage, compiledFile, marginArgument] = process.argv.slice(2);
assert.ok(stage && compiledFile);
const margin = Number(marginArgument ?? 0);
assert.ok(Number.isFinite(margin) && margin >= 0);
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`, "utf8"));
assert.equal(edits.length, 1);
assert.equal(edits[0].asset, "nottingham-castle-upper-stair");
const document = await readStoredMap("library/scenes/nottingham.rhlos-map.json", "library");
const assets = await pinnedDescriptors("library", document.assetSources, document.sceneAssets);
const index = JSON.parse(await fs.readFile("library/3d-assets/index.json", "utf8")).assets;
assert.equal(
  index.find((e) => e.id === edits[0].asset).descriptor_sha256,
  edits[0].descriptorSha256,
);
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const flightReviews = JSON.parse(await fs.readFile(`${stage}/mesh-review.json`, "utf8"));
assert.equal(flightReviews.length, 2);
const stairEntry = index.find((e) => e.id === edits[0].asset);
const modelHash = hash(await fs.readFile(`library/3d-assets/${stairEntry.model}`));
for (const review of flightReviews) {
  assert.equal(review.asset, edits[0].asset);
  assert.equal(review.modelSha256, modelHash);
  assert.ok(review.maximumUncoveredMeshEdgeDistance < 0.034);
  edits[0].gameplay.draft ??= { issues: [] };
  edits[0].gameplay.draft.issues.push(
    `Upper castle stair ${review.node} has ${review.sampledMeshHits}/${review.sampledFloorPoints} mesh sample hits, with gaps up to ${review.maximumUncoveredMeshEdgeDistance.toFixed(3)} game units; rendered actor integration remains unverified.`,
  );
}
const compiledBytes = await fs.readFile(compiledFile);
const compiled = JSON.parse(compiledBytes).asset_geometry;
const floors = [
  [787, 1165.001, 175.001],
  [812, 1230.00101, 100.00101],
].map((point) => {
  const matches = compiled.lifts.filter((l) =>
    l.physical_navigation?.doors.some(
      (d) => Math.hypot(...d.outside.map((v, i) => v - point[i])) < 1e-5,
    ),
  );
  assert.equal(matches.length, 1);
  return matches[0].physical_navigation;
});
const contacts = [
  {
    asset: "nottingham-castle-upper-wall",
    surface: "building-362-walk-0",
    edges: [
      [7, 0],
      [8, 1],
    ],
  },
  {
    asset: "nottingham-castle-east-courtyard-wall",
    surface: "building-325-walk-0",
    edges: [[39, 0]],
  },
  { asset: "nottingham-castle-courtyard-ground", surface: "building-366-walk-0", edges: [[18, 1]] },
];
const output = await fs.mkdtemp("work/map-compile/upper-castle-stair-contacts-");
const changes = [];
function meshHeight(p, [a, b, c]) {
  const det = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
  if (Math.abs(det) < 1e-8) return undefined;
  const u = ((b[1] - c[1]) * (p[0] - c[0]) + (c[0] - b[0]) * (p[1] - c[1])) / det;
  const v = ((c[1] - a[1]) * (p[0] - c[0]) + (a[0] - c[0]) * (p[1] - c[1])) / det;
  return u >= -1e-8 && v >= -1e-8 && u + v <= 1 + 1e-8
    ? u * a[2] + v * b[2] + (1 - u - v) * c[2]
    : undefined;
}
function edgeDistance(p, a, b) {
  const d = b.map((v, i) => v - a[i]);
  const t = Math.max(
    0,
    Math.min(
      1,
      d.reduce((s, v, i) => s + v * (p[i] - a[i]), 0) / (d.reduce((s, v) => s + v * v, 0) || 1),
    ),
  );
  return Math.hypot(...p.map((v, i) => v - a[i] - t * d[i]));
}
for (const contact of contacts) {
  const descriptor = assets.get(contact.asset),
    gameplay = structuredClone(descriptor.gameplay);
  const surface = gameplay.surfaces.find((s) => s.id === contact.surface);
  const part = document.objects.find((o) => o.node === `asset:${contact.asset}:${surface.node}`);
  assert.ok(part);
  const matrix = partMatrix(document.camera, document, part);
  const transform = (point) => {
    const p = gameToScene(document.camera, ...point);
    return sceneToGame(
      document.camera,
      [0, 1, 2].map(
        (r) => matrix[r] * p[0] + matrix[r + 4] * p[1] + matrix[r + 8] * p[2] + matrix[r + 12],
      ),
    );
  };
  const origin = transform([0, 0, 0]);
  for (const axis of [0, 1, 2]) {
    const p = [0, 0, 0];
    p[axis] = 1;
    assert.ok(
      transform(p).every((v, i) => Math.abs(v - origin[i] - (i === axis ? 1 : 0)) < 1e-7),
      "Expected translation-only authoring frame",
    );
  }
  const points = surface.polygon.map((p, i) => transform([...p, surface.height[i]]));
  const plane = heightPlane(points);
  const constraints = new Map();
  for (const [edge, which] of contact.edges)
    for (const vertex of [edge, (edge + 1) % points.length]) {
      const lines = constraints.get(vertex) ?? [];
      lines.push(floors[which].plane.map((v, i) => v - plane[i]));
      constraints.set(vertex, lines);
    }
  const corrected = structuredClone(points);
  for (const [vertex, lines] of constraints) {
    const [x, y] = points[vertex];
    let q;
    if (lines.length === 1) {
      const [a, b, c] = lines[0];
      const t = (a * x + b * y + c) / (a * a + b * b);
      q = [x - a * t, y - b * t];
    } else {
      assert.equal(lines.length, 2);
      const [a, b, c] = lines[0],
        [d, e, f] = lines[1];
      const det = a * e - b * d;
      assert.ok(Math.abs(det) > 1e-8);
      q = [(b * f - c * e) / det, (c * d - a * f) / det];
    }
    corrected[vertex] = [...q, planeHeight(plane, q)];
    assert.ok(
      Math.hypot(...corrected[vertex].map((v, i) => v - points[vertex][i])) < 3.1,
      "Contact shift needs broader review",
    );
  }
  const entry = index.find((e) => e.id === contact.asset);
  const modelBytes = await fs.readFile(`library/3d-assets/${entry.model}`);
  const model = await loadSceneModel("library", {
    id: entry.id,
    role: "objects",
    descriptor: `3d-assets/${entry.descriptor}`,
    descriptor_sha256: entry.descriptor_sha256,
    model: `3d-assets/${entry.model}`,
    model_sha256: hash(modelBytes),
    resources: descriptor.resources ?? [],
    ...(entry.model_scene ? { model_scene: entry.model_scene } : {}),
  });
  const triangles = maskRecoveryMesh(model, surface.node, (p) =>
    transform(sceneToGame(document.camera, gltfToScene(p))),
  );
  const flat = triangles.filter((t) =>
    t.every((p) => Math.abs(p[2] - planeHeight(plane, p)) < 0.25),
  );
  for (const [edge, which] of contact.edges) {
    const vertices = [edge, (edge + 1) % points.length];
    const before = vertices.map((i) => points[i]),
      after = vertices.map((i) => corrected[i]);
    const samples = [];
    for (let along = 0; along <= 40; along++)
      for (let across = 0; across <= 4; across++) {
        const p = before[0].map((v, i) => v + ((before[1][i] - v) * along) / 40),
          q = after[0].map((v, i) => v + ((after[1][i] - v) * along) / 40);
        const point = p.map((v, i) => v + ((q[i] - v) * across) / 4);
        const hits = triangles.map((t) => meshHeight(point, t)).filter((h) => h !== undefined);
        const supported = hits.some((h) => Math.abs(h - point[2]) < 0.1);
        const gap = supported
          ? 0
          : Math.min(
              ...flat.flatMap((t) => t.map((a, i) => edgeDistance(point, a, t[(i + 1) % 3]))),
            );
        samples.push({ point, hits, supported, gap });
      }
    const gap = Math.max(...samples.map((s) => s.gap));
    const supported = samples.filter((s) => s.supported).length;
    changes.push({
      asset: contact.asset,
      surface: surface.id,
      edge,
      before,
      after,
      gap,
      supported,
      samples,
      modelSha256: hash(modelBytes),
    });
    await fs.writeFile(
      `${output}/review.json`,
      JSON.stringify(
        { stage, compiledFile, compiledSha256: hash(compiledBytes), margin, changes },
        null,
        2,
      ),
    );
    assert.ok(
      Number.isFinite(gap) && gap <= margin,
      `${contact.asset} edge ${edge}: ${supported}/205 supported, gap ${gap}; see ${output}`,
    );
    const floor = floors[which];
    const clearance = {
      id: `${surface.id}-upper-castle-stair-${which}-clearance`,
      node: surface.node,
      polygon: floor.boundary.map((p) => p.map((v, i) => v - origin[i])),
      height: floor.boundary.map((p) => planeHeight(floor.plane, p) - origin[2]),
      holes: [],
    };
    gameplay.movementClearances ??= [];
    assert.ok(!gameplay.movementClearances.some((c) => c.id === clearance.id));
    gameplay.movementClearances.push(clearance);
    if (gap > 0) {
      gameplay.draft ??= { issues: [] };
      gameplay.draft.issues.push(
        `Upper castle stair contact edge ${edge} has ${supported}/205 mesh sample hits, with gaps up to ${gap.toFixed(3)} game units; rendered actor integration remains unverified.`,
      );
    }
  }
  for (const vertex of constraints.keys()) {
    surface.polygon[vertex] = corrected[vertex].slice(0, 2).map((v, i) => v - origin[i]);
    surface.height[vertex] = corrected[vertex][2] - origin[2];
  }
  surface.preserveMovementPrecision = true;
  if (contact.asset === "nottingham-castle-courtyard-ground") {
    const volume = gameplay.volumes.find((v) => v.id === surface.projectionVolume);
    assert.ok(volume && volume.node === surface.node);
    const vertices = [24, 25];
    const before = vertices.map((i) => {
      const p = volume.shape.points[i];
      return transform([p.x, p.y, p.z_top]);
    });
    const difference = floors[1].plane.map((v, i) => v - plane[i]);
    const [a, b, c] = difference;
    const after = before.map(([x, y]) => {
      const t = (a * x + b * y + c) / (a * a + b * b);
      const q = [x - a * t, y - b * t];
      return [...q, planeHeight(plane, q)];
    });
    assert.ok(before.every((p, i) => Math.hypot(...p.map((v, j) => v - after[i][j])) < 2));
    const samples = [];
    for (let along = 0; along <= 40; along++)
      for (let across = 0; across <= 4; across++) {
        const p = before[0].map((v, i) => v + ((before[1][i] - v) * along) / 40),
          q = after[0].map((v, i) => v + ((after[1][i] - v) * along) / 40);
        const point = p.map((v, i) => v + ((q[i] - v) * across) / 4);
        const hits = triangles.map((t) => meshHeight(point, t)).filter((h) => h !== undefined);
        const supported = hits.some((h) => Math.abs(h - point[2]) < 0.1);
        const gap = supported
          ? 0
          : Math.min(
              ...flat.flatMap((t) => t.map((a, i) => edgeDistance(point, a, t[(i + 1) % 3]))),
            );
        samples.push({ point, hits, supported, gap });
      }
    const gap = Math.max(...samples.map((s) => s.gap)),
      supported = samples.filter((s) => s.supported).length;
    changes.push({
      asset: contact.asset,
      volume: volume.id,
      before,
      after,
      gap,
      supported,
      samples,
      modelSha256: hash(modelBytes),
    });
    await fs.writeFile(
      `${output}/review.json`,
      JSON.stringify(
        { stage, compiledFile, compiledSha256: hash(compiledBytes), margin, changes },
        null,
        2,
      ),
    );
    assert.ok(Number.isFinite(gap) && gap <= margin, `Physical receiver gap ${gap}; see ${output}`);
    vertices.forEach((index, i) => {
      volume.shape.points[index].x = after[i][0] - origin[0];
      volume.shape.points[index].y = after[i][1] - origin[1];
    });
  }
  validateAssetGameplay(gameplay, descriptor);
  edits.push({ asset: entry.id, descriptorSha256: entry.descriptor_sha256, gameplay });
}
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
console.log(JSON.stringify({ output, changes: changes.map(({ samples, ...c }) => c) }));
