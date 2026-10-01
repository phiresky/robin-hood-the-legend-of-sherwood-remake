import { pathToFileURL, fileURLToPath } from "node:url";
// CPU-only geometry workloads; excludes browser/GPU time.
// Optional argument selects another level-editor checkout for comparison.
const root = process.argv[2] ?? fileURLToPath(new URL("../../", import.meta.url));
const load = (p) => import(pathToFileURL(root + "/" + p));
const THREE = await load("app/node_modules/three/build/three.module.js");
const { riverGeometry, wallMesh } = await load("app/src/spline-geometry.ts");
const { createTerrainGrid } = await load("shared/src/index.ts");
const camera = { kind: "oblique-orthographic", elevation_deg: 35 };
const river = {
  id: "river",
  kind: "river",
  points: [
    [0, 0, 0],
    [300, 0, 0],
    [600, 150, 0],
  ],
  width: 80,
  repeatLength: 100,
  closed: false,
};
const document = {
  camera,
  terrain: createTerrainGrid([-200, -200, 1000, 1000], 250),
  splines: [river],
};
let start = performance.now();
const road = riverGeometry({ ...river, kind: "road" }, camera, document);
console.log("road ms", performance.now() - start, "vertices", road.attributes.position.count);
let area = 0;
const p = road.attributes.position;
for (let i = 0; i < p.count; i += 3)
  area +=
    Math.abs(
      (p.getX(i + 1) - p.getX(i)) * (p.getY(i + 2) - p.getY(i)) -
        (p.getY(i + 1) - p.getY(i)) * (p.getX(i + 2) - p.getX(i)),
    ) / 2;
console.log("road projected area", area);
road.dispose();
const source = new THREE.Mesh(
  new THREE.BoxGeometry(100, 20, 60, 40, 8, 16),
  new THREE.MeshBasicMaterial(),
);
source.position.set(50, 0, 30);
source.updateMatrixWorld(true);
const path = {
  ...river,
  kind: "wall",
  asset: "wall",
  sourceStraight: true,
  width: 20,
  repeatLength: 100,
  points: [
    [0, 0, 0],
    [600, 200, 0],
    [1200, 0, 0],
  ],
};
start = performance.now();
const wall = wallMesh(path, camera, new Map([["asset:wall:mesh", source]]));
console.log("wall ms", performance.now() - start);
wall.traverse((n) => {
  if (n.isMesh) n.geometry.dispose();
});
source.geometry.dispose();
source.material.dispose();
