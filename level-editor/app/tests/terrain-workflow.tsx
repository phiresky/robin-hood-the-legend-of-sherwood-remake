import { render } from "@solidjs/web";
import { createSignal } from "solid-js";
import {
  parseStoredMap,
  serializeStoredMap,
  createTerrainGrid,
  gameToScene,
  terrainHeightAt,
  type Level3D,
} from "@rle/shared";
import * as THREE from "three";
import TerrainPanel from "../src/TerrainPanel";
import { EditorViewport } from "../src/editor-viewport";
import { packageCompiledMap } from "../src/map-compile";
import "../src/styles.css";

let current: Level3D = {
  version: 1,
  map: "Terrain test",
  size: [800, 640],
  terrain: createTerrainGrid([0, 0, 800, 640], 128),
  camera: { kind: "oblique-orthographic", elevation_deg: 35 },
  objects: [],
  groups: [],
  sceneAssets: [],
};
const [doc, setDoc] = createSignal(current);
const result = document.querySelector("#result")!;
const assert = (v: unknown, message: string) => {
  if (!v) throw new Error(message);
};
const errors: string[] = [];
const viewport = new EditorViewport({
  document: () => current,
  selection: () => null,
  level: () => null,
  showObstacles: () => false,
  showElevation: () => false,
  onSelection: () => {},
  commitTransform: () => {},
  onError: (e) => errors.push(e),
});
let commits = 0;
function commit(next: Level3D) {
  commits++;
  current = next;
  setDoc(next);
  viewport.syncViews(next);
}
const pause = () => new Promise((resolve) => setTimeout(resolve, 80));
render(
  () => (
    <div style={{ display: "flex", height: "100vh", width: "100vw" }}>
      <div
        id="view"
        class="editor-canvas"
        style={{ flex: "1", "min-width": "0", position: "relative" }}
      />
      <aside class="editor-panel" style={{ width: "340px", overflow: "auto" }}>
        <TerrainPanel
          viewport={viewport}
          document={doc}
          commit={commit}
          onError={(e) => errors.push(e)}
        />
      </aside>
    </div>
  ),
  document.querySelector("#root")!,
);
viewport.setup(document.querySelector("#view")!);
viewport.replaceMap(new THREE.Group(), null, new Map());
async function click(text: string) {
  const button = [...document.querySelectorAll("button")].find((b) => b.textContent === text);
  assert(button, `Missing ${text}`);
  button!.click();
  await pause();
}
async function set(label: string, value: string) {
  const input = document.querySelector<HTMLInputElement | HTMLSelectElement>(
    `[aria-label="${label}"]`,
  );
  assert(input, `Missing ${label}`);
  input!.value = value;
  input!.dispatchEvent(new Event("change", { bubbles: true }));
  await pause();
}
type Internals = {
  terrainControls: {
    root: THREE.Group;
    mode: {
      selectedVertices?: string[];
      selectedCells?: string[];
      selectVertices?(ids: string[]): void;
      selectCells?(ids: string[]): void;
    };
  };
  activeCamera(): THREE.Camera;
  renderer: THREE.WebGLRenderer;
};
const internals = viewport as unknown as Internals;
function selectVertex(index: number) {
  internals.terrainControls.mode.selectCells?.([]);
  internals.terrainControls.mode.selectVertices?.([current.terrain!.vertices[index]!.id]);
}
function selectCell(index: number) {
  internals.terrainControls.mode.selectVertices?.(
    current.terrain!.cells[index]!.vertices.map((i) => current.terrain!.vertices[i]!.id),
  );
  internals.terrainControls.mode.selectCells?.([current.terrain!.cells[index]!.id]);
}
async function activate() {
  await pause();
  current = { ...current };
  setDoc(current);
  viewport.syncViews(current);
  await pause();
}
async function run() {
  await activate();
  selectVertex(9);
  await pause();
  await set("Vertex Z", "24");
  await set("Vertex X", "140");
  assert(current.terrain!.vertices[9]!.position[2] === 24, "Vertex height did not commit");
  assert(
    current.terrain!.vertices[9]!.position[0] === 140,
    "Vertex horizontal edit did not commit",
  );
  selectCell(0);
  await pause();
  const count = current.terrain!.cells.length;
  await click("Subdivide selected cell");
  assert(current.terrain!.cells.length > count, "Subdivision did not add detail");
  const saved = JSON.stringify(serializeStoredMap(current, new Map()));
  current = parseStoredMap(JSON.parse(saved), new Map());
  setDoc(current);
  viewport.syncViews(current);
  assert(current.terrain!.vertices[9]!.position[2] === 24, "Saved vertex height did not reopen");
  viewport.frameContent(true);
  const before = viewport.captureThumbnail().toDataURL();
  const { compiled, pixels, appearance } = viewport.bakeMap(current, new Map());
  assert(
    compiled.descriptor.asset_geometry!.motion_data.layers.flat().length > 0,
    "No walking areas compiled",
  );
  const zip = await packageCompiledMap(compiled, pixels, appearance);
  assert(zip.length > 1000, "Missing baked map ZIP");
  const target = (viewport as unknown as { orbit: { target: THREE.Vector3 } }).orbit.target.clone();
  viewport.topView();
  viewport.setCardinalView("E");
  viewport.rotateViewQuarterTurn(-1);
  assert(
    target.distanceTo((viewport as unknown as { orbit: { target: THREE.Vector3 } }).orbit.target) <
      1e-5,
    "Direction controls changed target",
  );
  assert(errors.length === 0, errors.join("\n"));
  Object.assign(window, {
    __migrationImages: { before, after: viewport.captureThumbnail().toDataURL() },
  });
  result.textContent = `PASS grid XYZ editing, subdivision, save/reload, walking slopes, camera directions and ${zip.length} byte ZIP`;
}
Object.assign(window, {
  terrainTest: {
    state: () => ({
      document: current,
      commits,
      errors,
      selected: internals.terrainControls.mode.selectedVertices,
      cells: internals.terrainControls.mode.selectedCells,
      hover: internals.terrainControls.root.userData.terrainHoverVertices,
    }),
    point: (index: number) => {
      const position = current.terrain!.vertices[index]!.position;
      const p = new THREE.Vector3(...gameToScene(current.camera, ...position));
      internals.terrainControls.root.localToWorld(p);
      p.project(internals.activeCamera());
      const r = internals.renderer.domElement.getBoundingClientRect();
      return { x: r.left + ((p.x + 1) * r.width) / 2, y: r.top + ((1 - p.y) * r.height) / 2 };
    },
    selectVertex,
    selectCell,
    height: (x: number, y: number) => terrainHeightAt(current, x, y),
    roundtrip: () => {
      current = parseStoredMap(
        JSON.parse(JSON.stringify(serializeStoredMap(current, new Map()))),
        new Map(),
      );
      setDoc(current);
      viewport.syncViews(current);
    },
    clearSelection: () => {
      internals.terrainControls.mode.selectVertices?.([]);
      internals.terrainControls.mode.selectCells?.([]);
    },
    frame: () => viewport.frameContent(true),
    top: async () => {
      viewport.topView();
      await new Promise((resolve) => setTimeout(resolve, 800));
      viewport.setCardinalView("N");
    },
  },
});
async function prepareControls() {
  await activate();
  viewport.syncViews(current);
  viewport.frameContent(true);
  await pause();
  selectVertex(9);
  await pause();
  result.textContent = "READY";
}
(new URLSearchParams(location.search).has("controls") ? prepareControls() : run()).catch(
  (error) => {
    result.textContent = "FAIL " + (error.stack ?? error);
  },
);
