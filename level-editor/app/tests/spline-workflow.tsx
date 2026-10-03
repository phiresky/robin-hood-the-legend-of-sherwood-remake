import { editRiverBanks, riverBankStyles } from "../../shared/src/river-banks.ts";
import { render } from "@solidjs/web";
import { createSignal } from "solid-js";
import * as THREE from "three";
import {
  createTerrainGrid,
  parseLevel3D,
  parseStoredMap,
  serializeStoredMap,
  terrainGameplay,
  terrainHeightAt,
  type Level3D,
  type LevelSpline,
} from "@rle/shared";
import { sampleSpline, splineMaterialWeightsAt } from "../../shared/src/spline-sampling.ts";
import { riverGeometry, blendedSplineTexture } from "../src/spline-geometry.ts";
import type { SplineLayer, SplineEditMode } from "../src/spline-layer.ts";
import { EditorViewport } from "../src/editor-viewport.ts";
import { packageCompiledMap } from "../src/map-compile.ts";
import SplinePanel from "../src/SplinePanel.tsx";
import "../src/styles.css";

const road: LevelSpline = {
  id: "road",
  name: "Dirt to cobblestone hillside path",
  kind: "road",
  closed: false,
  points: [
    [48, 90, 0],
    [240, 110, 0],
    [430, 70, 0],
  ],
  width: 40,
  pointWidths: [24, 72, 40],
  pointMaterials: ["path_dirt", "road_cobblestone_broken", "road_cobblestone_mossy"],
  repeatLength: 128,
};
const river: LevelSpline = {
  id: "river",
  name: "Rocky channel with a ford",
  kind: "river",
  closed: false,
  points: [
    [40, 265, 10],
    [180, 270, 10],
    [300, 250, 10],
    [440, 275, 10],
  ],
  width: 50,
  pointWidths: [40, 65, 80, 50],
  pointMaterials: ["water_stones_large", "water_ford", "water_ford", "water_white"],
  channel: { enabled: true, bedDepth: 18, bankSlope: 1 },
  repeatLength: 128,
};
river.pointBanks = editRiverBanks(river, [0, 1, 2, 3], "both", {
  width: 38,
  mix: { mixed_stones: 1 },
});
river.pointBanks = editRiverBanks(river, [2, 3], "left", { mix: { vegetation: 1 } });
const grid = createTerrainGrid([0, 0, 480, 384], 96);
let current: Level3D = {
  version: 1,
  map: "Spline workflow",
  size: [480, 384],
  exportBounds: [0, -60, 480, 444],
  camera: { kind: "oblique-orthographic", elevation_deg: 35 },
  objects: [],
  groups: [],
  sceneAssets: [],
  terrain: {
    ...grid,
    vertices: grid.vertices.map((v) => ({
      ...v,
      position: [v.position[0], v.position[1], 20 + v.position[0] / 24],
    })),
  },
  splines: [road, river],
};
const [doc, setDoc] = createSignal(current);
const errors: string[] = [];
let commits = 0;
const result = document.querySelector("#result")!;
const check = (condition: unknown, message: string) => {
  if (!condition) throw new Error(message);
};
const viewport = new EditorViewport({
  document: doc,
  selection: () => null,
  level: () => null,
  showObstacles: () => false,
  showElevation: () => false,
  onSelection: () => {},
  commitTransform: () => {},
  onError: (error) => errors.push(error),
});
function commit(next: Level3D) {
  parseLevel3D(next);
  current = next;
  commits++;
  setDoc(next);
  viewport.syncViews(next);
}
render(
  () => (
    <div style={{ display: "flex", height: "100vh", width: "100vw" }}>
      <div
        id="view"
        class="editor-canvas"
        style={{ flex: "1", "min-width": "0", position: "relative" }}
      />
      <aside class="editor-panel" style={{ width: "340px", overflow: "auto" }}>
        <SplinePanel
          document={doc}
          library={() => null}
          entries={() => []}
          viewport={viewport}
          commit={commit}
          onError={(error) => errors.push(error)}
        />
      </aside>
    </div>
  ),
  document.querySelector("#root")!,
);
viewport.setup(document.querySelector("#view")!);
viewport.replaceMap(new THREE.Group(), null, new Map());
const pause = () => new Promise((resolve) => setTimeout(resolve, 100));
const timings: Record<string, number> = {};
let stageStarted = performance.now();
async function stage(name: string) {
  timings[name] = Math.round(performance.now() - stageStarted);
  result.textContent = `Running spline workflow: finished ${name}`;
  await pause();
  stageStarted = performance.now();
}
async function run() {
  parseLevel3D(current);
  viewport.syncViews(current);
  viewport.frameContent(true);
  await stage("initial-render");
  const samples = sampleSpline(road, current.camera);
  check(Math.abs(samples[0]!.width - 24) < 1e-5, "Road starting width differs from control width");
  check(
    Math.abs(samples.at(-1)!.width - 40) < 1e-5,
    "Road ending width differs from control width",
  );
  check(Math.max(...samples.map((s) => s.width)) > 70, "Road never widens at middle control point");
  const weights = splineMaterialWeightsAt(road, 0.25);
  check(
    Math.abs(weights.path_dirt! - 0.5) < 1e-6 &&
      Math.abs(weights.road_cobblestone_broken! - 0.5) < 1e-6,
    "Road midpoint materials do not blend equally",
  );
  const geometry = riverGeometry(road, current.camera, current),
    positions = geometry.getAttribute("position");
  const sin = Math.sin((35 * Math.PI) / 180),
    cos = Math.cos((35 * Math.PI) / 180);
  for (let i = 0; i < positions.count; i++) {
    const ground = terrainHeightAt(current, positions.getX(i), -positions.getY(i) * sin);
    check(ground !== undefined, "Road ribbon escaped terrain");
    check(
      Math.abs((positions.getZ(i) - 0.8) * cos - ground!) < 0.001,
      "Road vertex does not conform to terrain across width",
    );
  }
  geometry.dispose();
  const tile = blendedSplineTexture(road, current.camera, current);
  check(tile.image.width === 128 && tile.image.height > 50, "Material blend texture not generated");
  tile.dispose();
  await stage("geometry-material-checks");
  const baseHeight = terrainHeightAt({ ...current, splines: [] }, 240, 260)!;
  const channelHeight = terrainHeightAt(current, 240, 260)!;
  check(channelHeight < baseHeight - 5, "River modifier did not excavate the channel");
  const gameplay = terrainGameplay(current)!.gameplay!;
  const waterSurfaces = gameplay.surfaces.filter((s) => s.id.startsWith("river/"));
  const blockedIds = new Set(gameplay.movementBlockers?.map((s) => s.id.replace(/^blocked\//, "")));
  check(
    waterSurfaces.some((s) => blockedIds.has(s.id)),
    "River has no blocked water",
  );
  check(
    waterSurfaces.some((s) => !blockedIds.has(s.id)),
    "Ford has no walkable surface",
  );
  await stage("channel-gameplay-checks");
  const saved = JSON.stringify(serializeStoredMap(current, new Map()));
  current = parseStoredMap(JSON.parse(saved), new Map());
  setDoc(current);
  viewport.syncViews(current);
  check(current.splines![0]!.pointWidths![1] === 72, "Variable widths lost on save/reload");
  check(current.splines![1]!.channel!.bedDepth === 18, "Channel settings lost on save/reload");
  check(
    JSON.stringify(current.splines![1]!.pointBanks) === JSON.stringify(river.pointBanks),
    "Bank designs lost on save/reload",
  );
  await stage("reload");
  const screenshot = viewport.captureThumbnail().toDataURL();
  const { compiled, pixels, appearance } = viewport.bakeMap(current, new Map());
  await stage("bake");
  check(pixels.color.length === 480 * 444 * 4, "Wrong color bake size");
  check(pixels.depth.length === 480 * 444, "Wrong depth bake size");
  check(new Set(pixels.depth).size > 4, "Depth bake does not contain terrain height variation");
  check(new Set(pixels.color).size > 32, "Color bake has no textured content");
  check(
    compiled.descriptor.asset_geometry!.motion_data.layers.flat().length > 0,
    "No gameplay walking regions exported",
  );
  const zip = await packageCompiledMap(compiled, pixels, appearance);
  await stage("package");
  check(zip.length > 1000, "Map ZIP is empty");
  const internals = viewport as unknown as {
    camera: THREE.Camera;
    splines: SplineLayer;
    splineMode: SplineEditMode | null;
  };
  const canvas = document.querySelector<HTMLCanvasElement>("#view canvas")!;
  async function clickSpline(id: string) {
    const line = internals.splines.controls.children.find(
      (child) => child.userData.splinePath === id,
    ) as THREE.Line;
    check(line, `Missing visible spline ${id}`);
    const position = new THREE.Vector3().fromBufferAttribute(
      line.geometry.getAttribute("position"),
      12,
    );
    line.localToWorld(position).project(internals.camera);
    const rect = canvas.getBoundingClientRect();
    const event = {
      bubbles: true,
      button: 0,
      pointerId: 7,
      clientX: rect.left + ((position.x + 1) * rect.width) / 2,
      clientY: rect.top + ((1 - position.y) * rect.height) / 2,
    };
    canvas.dispatchEvent(new PointerEvent("pointerdown", event));
    canvas.dispatchEvent(new PointerEvent("pointerup", event));
    await pause();
    check(internals.splineMode?.path.id === id, `Clicking spline ${id} did not select it`);
  }
  async function checkCurvedToggle(id: string) {
    const checkbox = document.querySelector<HTMLInputElement>('input[aria-label="Curved"]');
    check(checkbox?.checked, `Curved checkbox missing or off for ${id}`);
    checkbox!.click();
    await pause();
    check(
      current.splines!.find((path) => path.id === id)!.curved === false,
      "Curved toggle did not commit immediately",
    );
    const lines = internals.splines.controls.children.filter(
      (child) => child instanceof THREE.Line && child.userData.splinePath === id,
    ) as THREE.Line[];
    for (const line of lines) {
      const positions = line.geometry.getAttribute("position");
      const a = new THREE.Vector3().fromBufferAttribute(positions, 0);
      const b = new THREE.Vector3().fromBufferAttribute(positions, positions.count - 1);
      for (let i = 1; i + 1 < positions.count; i++) {
        const p = new THREE.Vector3().fromBufferAttribute(positions, i);
        check(
          p.distanceTo(a.clone().lerp(b, i / (positions.count - 1))) < 0.0001,
          `${id} editing spline still curves when disabled`,
        );
      }
    }
    const restored = parseStoredMap(
      JSON.parse(JSON.stringify(serializeStoredMap(current, new Map()))),
      new Map(),
    );
    check(
      restored.splines!.find((path) => path.id === id)!.curved === false,
      "Straight setting lost on reload",
    );
    document.querySelector<HTMLInputElement>('input[aria-label="Curved"]')!.click();
    await pause();
    check(
      current.splines!.find((path) => path.id === id)!.curved === true,
      "Curved toggle is one update late",
    );
  }
  await clickSpline("road");
  await checkCurvedToggle("road");
  const selectedRoad = () => current.splines!.find((path) => path.id === "road")!;
  const rect = canvas.getBoundingClientRect();
  const project = (object: THREE.Object3D) => {
    const p = object.getWorldPosition(new THREE.Vector3()).project(internals.camera);
    return {
      clientX: rect.left + ((p.x + 1) * rect.width) / 2,
      clientY: rect.top + ((1 - p.y) * rect.height) / 2,
    };
  };
  const line = internals.splines.controls.children.find(
    (child) => child.userData.splineSection === 0,
  ) as THREE.Line;
  const mid = new THREE.Vector3().fromBufferAttribute(line.geometry.getAttribute("position"), 12);
  line.localToWorld(mid).project(internals.camera);
  canvas.dispatchEvent(
    new MouseEvent("dblclick", {
      bubbles: true,
      button: 0,
      clientX: rect.left + ((mid.x + 1) * rect.width) / 2,
      clientY: rect.top + ((1 - mid.y) * rect.height) / 2,
    }),
  );
  await pause();
  check(selectedRoad().points.length === 4, "Double-click must insert one control point");
  check(internals.splineMode?.point === 1, "Inserted control point must be selected");
  canvas.setPointerCapture = () => {};
  canvas.releasePointerCapture = () => {};
  canvas.hasPointerCapture = () => false;
  const handles = () =>
    internals.splines.controls.children.filter(
      (child) => typeof child.userData.splinePoint === "number",
    );
  const handlePositions = handles().slice(0, 2).map(project);
  const start = {
    clientX: Math.min(...handlePositions.map((p) => p.clientX)) - 4,
    clientY: Math.min(...handlePositions.map((p) => p.clientY)) - 4,
  };
  const end = {
    clientX: Math.max(...handlePositions.map((p) => p.clientX)) + 4,
    clientY: Math.max(...handlePositions.map((p) => p.clientY)) + 4,
  };
  const pointer = (
    type: string,
    position: { clientX: number; clientY: number },
    shiftKey = false,
  ) =>
    canvas.dispatchEvent(
      new PointerEvent(type, { bubbles: true, button: 0, pointerId: 7, shiftKey, ...position }),
    );
  pointer("pointerdown", start, true);
  pointer("pointermove", end, true);
  pointer("pointerup", end, true);
  await pause();
  check(
    internals.splineMode?.selectedPoints?.join(",") === "1,0",
    "Shift marquee must add enclosed points to selection",
  );
  const before = selectedRoad().points.map((point) => [...point]);
  const first = project(handles().find((handle) => handle.userData.splinePoint === 0)!);
  pointer("pointerdown", first);
  pointer("pointermove", { clientX: first.clientX + 20, clientY: first.clientY + 10 });
  await pause(); // Let the coalesced, terrain-sampled drag preview render before release.
  const preview = (internals.splines as unknown as { preview: { object: THREE.Mesh } | null })
    .preview;
  check(preview?.object.geometry.index, "Road drag must use the sampled preview mesh");
  pointer("pointerup", { clientX: first.clientX + 20, clientY: first.clientY + 10 });
  await pause();
  const after = selectedRoad().points;
  check(Math.abs(after[0]![0] - before[0]![0]!) > 1, "Selected point must move");
  check(
    Math.abs(after[0]![0] - before[0]![0]! - (after[1]![0] - before[1]![0]!)) < 1e-6,
    "Marquee selected points must move together",
  );
  check(
    JSON.stringify(after.slice(2)) === JSON.stringify(before.slice(2)),
    "Unselected points must stay in place",
  );
  const single = project(handles().find((handle) => handle.userData.splinePoint === 0)!);
  pointer("pointerdown", single);
  pointer("pointerup", single);
  await pause();
  check(
    internals.splineMode?.selectedPoints?.join(",") === "0",
    "Single click must collapse selection",
  );
  pointer("pointerdown", start, true);
  pointer("pointermove", end, true);
  window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
  await pause();
  check(!document.querySelector('div[style*="100000"]'), "Canceled marquee must be removed");
  await clickSpline("river");
  await checkCurvedToggle("river");
  const selectBank = async (label: string, value: string) => {
    const input = document.querySelector<HTMLSelectElement>(`select[aria-label="${label}"]`)!;
    check(input, `Missing ${label}`);
    input.value = value;
    input.dispatchEvent(new Event("change", { bubbles: true }));
    await pause();
  };
  const bankRiver = () => current.splines!.find((path) => path.id === "river")!;
  await selectBank("Bank design scope", "river");
  await selectBank("Bank side", "both");
  for (const style of Object.keys(riverBankStyles)) {
    await selectBank("Bank edge design", style);
    check(
      bankRiver().pointBanks!.every(
        (p) =>
          p.left.mix[style as keyof typeof riverBankStyles] === 1 &&
          p.right.mix[style as keyof typeof riverBankStyles] === 1,
      ),
      `Whole-river style ${style} did not update both banks`,
    );
  }
  await selectBank("Bank design scope", "point");
  await selectBank("Bank side", "left");
  await selectBank("Bank edge design", "big_stones");
  check(
    bankRiver().pointBanks!.some((p) => p.left.mix.big_stones === 1),
    "Point bank design missing",
  );
  check(
    bankRiver().pointBanks!.every((p) => p.right.mix.vegetation === 1),
    "Point edit changed opposite bank",
  );
  await selectBank("Selected section", "1");
  await selectBank("Bank design scope", "section");
  await selectBank("Bank side", "right");
  await selectBank("Bank edge design", "stones_plants");
  check(
    bankRiver().pointBanks![1]!.right.mix.stones_plants === 1 &&
      bankRiver().pointBanks![2]!.right.mix.stones_plants === 1,
    "Section did not set both endpoints",
  );
  check(
    bankRiver().pointBanks![0]!.right.mix.vegetation === 1,
    "Section edit changed distant bank",
  );
  const riverBefore = current.splines!.find((path) => path.id === "river")!;
  const riverHandle = project(handles().find((handle) => handle.userData.splinePoint === 1)!);
  pointer("pointerdown", riverHandle);
  pointer("pointermove", { clientX: riverHandle.clientX + 25, clientY: riverHandle.clientY + 10 });
  pointer("pointerup", { clientX: riverHandle.clientX + 25, clientY: riverHandle.clientY + 10 });
  await pause();
  const riverAfter = current.splines!.find((path) => path.id === "river")!;
  check(
    riverAfter.points[1]![0] !== riverBefore.points[1]![0],
    "River drag must commit the new position",
  );
  const renderedRiver = (
    internals.splines as unknown as {
      views: Map<string, { path: LevelSpline }>;
    }
  ).views.get("river")!;
  check(
    JSON.stringify(renderedRiver.path.points) === JSON.stringify(riverAfter.points),
    "River release must not restore stale geometry after committing the new position",
  );
  const done = Array.from(document.querySelectorAll<HTMLButtonElement>("button")).find(
    (button) => button.textContent === "Done editing",
  )!;
  check(done, "Selected spline details are missing");
  done.click();
  await pause();
  check(!internals.splineMode, "Done editing did not clear spline selection");
  check(
    new Set(
      internals.splines.controls.children.flatMap((child) =>
        child.userData.splinePath ? [child.userData.splinePath] : [],
      ),
    ).size === 2,
    "All spline outlines must remain after finishing selection",
  );
  await stage("viewport-path-selection");
  const withBanks = viewport.bakeMap(current, new Map()).pixels;
  const afterBanks = viewport.captureThumbnail().toDataURL();
  const noBanks = {
    ...current,
    splines: current.splines!.map((p) => ({ ...p, pointBanks: undefined })),
  };
  setDoc(noBanks);
  await pause();
  viewport.syncViews(noBanks);
  const withoutBanks = viewport.bakeMap(noBanks, new Map()).pixels;
  const beforeBanks = viewport.captureThumbnail().toDataURL();
  check(
    withBanks.color.some((v, i) => v !== withoutBanks.color[i]),
    "Bank art missing from color export",
  );
  setDoc(current);
  await pause();
  viewport.syncViews(current);
  Object.assign(window, { __migrationImages: { before: beforeBanks, after: afterBanks } });
  check(!errors.length, errors.join("\n"));
  Object.assign(window, {
    __splineImages: { screenshot },
    __splineResults: {
      zipBytes: zip.length,
      channelHeight,
      baseHeight,
      waterSurfaces: waterSurfaces.length,
      blockedWater: waterSurfaces.filter((s) => blockedIds.has(s.id)).length,
      timings,
    },
  });
  result.textContent = `PASS variable widths, material blends, road conformance, channel excavation, ford, save/reload, color/depth bake and ${zip.length} byte ZIP; stage milliseconds ${JSON.stringify(timings)}`;
}
Object.assign(window, {
  splineTest: {
    state: () => ({ document: current, errors, commits }),
    frame: () => viewport.frameContent(true),
    bake: () => viewport.bakeMap(current, new Map()),
  },
});
run().catch((error) => {
  result.textContent = `FAIL ${error.stack ?? error}`;
  console.error(error);
});
