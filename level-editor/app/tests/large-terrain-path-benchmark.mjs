import { pathToFileURL, fileURLToPath } from "node:url";
// Same local edit on increasingly large maps; CPU-only, full-detail river geometry.
// Optional root argument benchmarks another level-editor checkout.
const root = process.argv[2] ?? fileURLToPath(new URL("../../", import.meta.url));
const load = (p) => import(pathToFileURL(root + "/" + p));
const { EditorViewport } = await load("app/src/editor-viewport.ts");
const { createTerrainGrid } = await load("shared/src/index.ts");
const { roadGeometry } = await load("app/src/road-geometry.ts");
const camera = { kind: "oblique-orthographic", elevation_deg: 35 };
for (const size of [2048, 8192, 16384, 32768]) {
  const grid = createTerrainGrid([0, 0, size, size], 128);
  const river = {
    id: "river",
    kind: "river",
    points: [
      [100, 500, 0],
      [700, 900, 0],
      [1400, 600, 0],
      [2000, 1200, 0],
    ],
    width: 100,
    repeatLength: 128,
    closed: false,
  };
  const road = {
    ...river,
    id: "road",
    kind: "road",
    points: [
      [100, 700, 0],
      [700, 1100, 0],
      [1400, 800, 0],
      [2000, 1400, 0],
    ],
  };
  const document = {
    version: 1,
    map: "perf",
    sceneAssets: [],
    objects: [],
    groups: [],
    size: [size, size],
    camera,
    terrain: grid,
    splines: [river, road],
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
  let t = [];
  for (let n = 0; n < 7; n++) {
    const moved = {
      ...river,
      points: river.points.map(([x, y, z], i) => [x, y + (i === 1 ? 10 + n : 0), z]),
    };
    let start = performance.now();
    viewport.previewSpline(moved);
    t.push(performance.now() - start);
  }
  t = t.slice(2).sort((a, b) => a - b);
  viewport.previewSpline(null);
  let start = performance.now();
  const geom = roadGeometry(road, camera, document);
  const exact = performance.now() - start;
  geom.dispose();
  viewport.dispose();
  console.log(
    JSON.stringify({ size, cells: grid.cells.length, riverDragMedianMs: t[2], exactRoadMs: exact }),
  );
}
