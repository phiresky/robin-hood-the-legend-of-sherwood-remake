import { pathToFileURL, fileURLToPath } from "node:url";
// CPU-only drag benchmark with a curved, blended river; excludes browser/GPU time.
// Optional argument points to another level-editor checkout for before/after comparisons.
const root = process.argv[2] ?? fileURLToPath(new URL("../../", import.meta.url));
const load = (path) => import(pathToFileURL(root + "/" + path));
const { EditorViewport } = await load("app/src/editor-viewport.ts");
const { createTerrainGrid } = await load("shared/src/index.ts");
const grid = createTerrainGrid([0, 0, 2048, 2048], 128);
grid.vertices[0].position[2] = 200;
const document = {
  version: 1,
  map: "perf",
  sceneAssets: [],
  objects: [],
  groups: [],
  size: [2048, 2048],
  camera: { kind: "oblique-orthographic", elevation_deg: 35 },
  terrain: grid,
  splines: [
    {
      id: "river",
      name: "River",
      kind: "river",
      points: [
        [0, 500, 0],
        [700, 900, 0],
        [1400, 600, 0],
        [2048, 1200, 0],
      ],
      width: 100,
      closed: false,
      repeatLength: 128,
      pointMaterials: ["water_still", "water_white", "water_still", "water_white"],
    },
  ],
};
const viewport = new EditorViewport({
  document: () => document,
  selection: () => null,
  level: () => null,
  showObstacles: () => false,
  showElevation: () => false,
  onSelection: () => {},
  commitTransform: () => {},
});
viewport.syncViews(document);
const times = [];
for (let n = 0; n < 14; n++) {
  const next = {
    ...grid,
    vertices: grid.vertices.map((v) =>
      v.position[0] === 1024 && v.position[1] === 896
        ? { ...v, position: [v.position[0], v.position[1], 20 + n] }
        : v,
    ),
  };
  const start = performance.now();
  viewport.previewTerrain(next);
  const elapsed = performance.now() - start;
  if (n >= 4) times.push(elapsed);
}
viewport.previewTerrain(null);
viewport.dispose();
times.sort((a, b) => a - b);
console.log(
  JSON.stringify({
    medianMs: times[Math.floor(times.length / 2)],
    minMs: times[0],
    maxMs: times.at(-1),
    samples: times.length,
  }),
);
