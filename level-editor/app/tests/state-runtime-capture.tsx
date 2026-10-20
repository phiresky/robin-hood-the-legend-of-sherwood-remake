import { render } from "@solidjs/web";
import { createSignal } from "solid-js";
import * as THREE from "three";
import { createTerrainGrid, type Level3D } from "@rle/shared";
import StatePreview from "../src/StatePreview";
import { EditorViewport } from "../src/editor-viewport";
import type { StateClockBatch } from "../../shared/src/state-event-clocks";
import type { StateDelivery } from "../src/state-delivery";

type Pin = { path: string; url: string; sha256: string };
type Config = {
  scope:
    | "unapproved-candidate-integration-diagnostic"
    | "approved-asset-recorded-integration-diagnostic";
  mission: string;
  map: string;
  size: [number, number];
  resources: Pin[];
  capture: Pin[];
  expected_frames: number[];
  expected_phases?: string[];
  visual_only?: boolean;
  live_bridge?: boolean;
  focus: {
    native_bounds: [number, number, number, number];
    origin: [number, number];
    elevation_deg: number;
  };
};
const result = document.querySelector("#result")!;
function assert(v: unknown, m: string): asserts v {
  if (!v) throw Error(m);
}
let diagnosticDetails = () => "";
const pause = () => new Promise((r) => setTimeout(r, 50));
async function until(check: () => boolean) {
  for (let i = 0; i < 600; i++) {
    if (check()) return;
    await pause();
  }
  throw Error(`Timed out waiting for preview: ${diagnosticDetails()}`);
}
const hash = async (b: ArrayBuffer) =>
  Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", b)), (v) =>
    v.toString(16).padStart(2, "0"),
  ).join("");
async function main() {
  const configUrl = new URLSearchParams(location.search).get("config");
  assert(configUrl, "Missing config URL");
  const response = await fetch(configUrl);
  assert(response.ok, "Missing fixture config");
  const config = (await response.json()) as Config;
  assert(
    [
      "unapproved-candidate-integration-diagnostic",
      "approved-asset-recorded-integration-diagnostic",
    ].includes(config.scope),
    "Wrong diagnostic scope",
  );
  document.querySelector("h1")!.textContent =
    config.scope === "approved-asset-recorded-integration-diagnostic"
      ? "Approved asset recorded-caller diagnostic (whole map unproven)"
      : "Unapproved candidate integration diagnostic";
  let allowedLiveSequence = 0;
  const originalFetch = window.fetch.bind(window);
  if (config.live_bridge)
    window.fetch = async (input, init) => {
      const match =
        typeof input === "string" ? /^\/runtime-observations\/packet\/(\d+)$/.exec(input) : null;
      if (match)
        await until(() => Number(match[1]) <= allowedLiveSequence || !!init?.signal?.aborted);
      return originalFetch(input, init);
    };
  const resources = new Map<string, File>();
  async function pinned(pin: Pin) {
    const r = await fetch(pin.url);
    assert(r.ok, `Missing ${pin.path}`);
    const b = await r.arrayBuffer();
    assert((await hash(b)) === pin.sha256, `Changed ${pin.path}`);
    return new File([b], pin.path.split("/").at(-1)!);
  }
  for (const pin of config.resources) resources.set(pin.path, await pinned(pin));
  let captures: File[] = [];
  function directory(prefix = ""): FileSystemDirectoryHandle {
    return {
      kind: "directory",
      name: prefix.split("/").at(-2) ?? "diagnostic",
      async getDirectoryHandle(name: string) {
        const p = prefix + name + "/";
        if (![...resources.keys()].some((k) => k.startsWith(p)))
          throw new DOMException("Missing directory", "NotFoundError");
        return directory(p);
      },
      async getFileHandle(name: string) {
        const f = resources.get(prefix + name);
        if (!f) throw new DOMException("Missing file", "NotFoundError");
        return {
          kind: "file",
          name,
          async getFile() {
            return f;
          },
        };
      },
      async *entries() {
        for (const [p, f] of resources) {
          if (p.startsWith(prefix) && !p.slice(prefix.length).includes("/"))
            yield [f.name, { kind: "file", name: f.name, getFile: async () => f }];
        }
      },
    } as unknown as FileSystemDirectoryHandle;
  }
  const current: Level3D = {
    version: 1,
    map: config.map,
    size: config.size,
    sceneAssets: [],
    camera: { kind: "oblique-orthographic", elevation_deg: 35 },
    terrain: createTerrainGrid([0, 0, ...config.size], 128, 0),
    groups: [],
    objects: [],
    mission: { importedFrom: config.mission } as Level3D["mission"],
  };
  const [doc, setDoc] = createSignal<Level3D>(current),
    errors: string[] = [],
    batches: StateClockBatch[] = [];
  diagnosticDetails = () =>
    JSON.stringify({
      phase: result.textContent,
      errors,
      controls: document.querySelector("#controls")?.textContent?.slice(0, 4000),
      loadedResources: resources.size,
      loadedCaptures: captures.length,
    });
  const viewport = new EditorViewport({
    document: doc,
    selection: () => null,
    level: () => null,
    showObstacles: () => false,
    showElevation: () => false,
    onSelection: () => {},
    commitTransform: () => {},
    onError: (e) => errors.push(String(e)),
  });
  viewport.setup(document.querySelector<HTMLDivElement>("#view")!);
  viewport.replaceMap(new THREE.Group(), null, new Map());
  viewport.syncViews(current);
  const realSample = viewport.sampleDeliveredSourceState.bind(viewport);
  let rejectSink = false;
  viewport.sampleDeliveredSourceState = (batch) => {
    if (rejectSink) throw Error("diagnostic sink failure");
    realSample(batch);
    batches.push(structuredClone(batch));
  };
  const root = directory();
  render(
    () => (
      <StatePreview
        document={doc}
        library={() => root}
        viewport={viewport}
        active={true}
        onError={(e) => errors.push(e)}
      />
    ),
    document.querySelector("#controls")!,
  );
  const captureInput = () =>
    document.querySelector<HTMLInputElement>('[aria-label="Recorded runtime capture"]');
  const manualReady = () => {
    const input = captureInput();
    return !!input && input.isConnected && !input.disabled;
  };
  result.textContent = "RUNNING: initial approved model load";
  await until(() => {
    assert(!errors.length, errors.join("\n"));
    return manualReady();
  });
  assert(!errors.length, errors.join("\n"));
  result.textContent = "RUNNING: loading pinned recorded files";
  for (const pin of config.capture) captures.push(await pinned(pin));
  const chooseFile = (files: File[]) => {
    const input = captureInput();
    assert(input?.isConnected && !input.disabled, "Capture input is not ready for import");
    const transfer = new DataTransfer();
    for (const f of files) transfer.items.add(f);
    input.files = transfer.files;
    input.dispatchEvent(new Event("change", { bubbles: true }));
  };
  const button = (text: string) =>
    Array.from(document.querySelectorAll("button")).find((b) => b.textContent?.trim() === text)!;
  async function load(files = captures) {
    const previousErrors = errors.length;
    // The asset selector stays mounted while choose() removes the contract/input.
    // Wait for the replacement input, not merely for the old controls to disappear.
    await until(manualReady);
    chooseFile(files);
    await until(() => {
      assert(errors.length === previousErrors, errors.slice(previousErrors).join("\n"));
      return !!button("Next recorded snapshot") && !button("Next recorded snapshot").disabled;
    });
  }
  const internals = viewport as unknown as { stateDelivery: StateDelivery };
  async function pixels() {
    return hash(internals.stateDelivery.native.pixels().data.slice().buffer);
  }
  result.textContent = "RUNNING: real initial capture import";
  if (config.live_bridge) {
    button("Connect observation bridge").click();
    await until(() => {
      assert(!errors.length, errors.join("\n"));
      return batches.length === 1;
    });
  } else await load();
  const initial = await pixels();
  assert(button("Play").disabled && button("Reset").disabled, "Manual playback remains enabled");
  assert(
    document.querySelector<HTMLInputElement>('[aria-label="State preview frame"]')!.disabled,
    "Manual seek remains enabled",
  );
  const images: Record<string, string> = {};
  const imageScopes: Record<string, string> = {};
  const focusedNativeCanvases = new WeakSet<HTMLCanvasElement>();
  async function captureViews(label: string) {
    const selector = document.querySelector<HTMLSelectElement>(
      '[aria-label="State preview view"]',
    )!;
    selector.value = "art";
    selector.dispatchEvent(new Event("change", { bubbles: true }));
    await pause();
    // Native artwork is its own visible canvas overlay, not the WebGL thumbnail.
    const nativeCanvas = document.querySelector<HTMLCanvasElement>(
      '[aria-label="Original artwork preview; pan and zoom"]',
    );
    assert(nativeCanvas, "Native artwork surface is not displayed");
    if (!focusedNativeCanvases.has(nativeCanvas)) {
      const rect = nativeCanvas.getBoundingClientRect(),
        image = internals.stateDelivery.native.pixels();
      const [bx, by, bw, bh] = config.focus.native_bounds,
        [ox, oy] = config.focus.origin;
      const oldScale = Math.min(rect.width / image.width, rect.height / image.height);
      const oldX = (rect.width - image.width * oldScale) / 2,
        oldY = (rect.height - image.height * oldScale) / 2;
      const oldPointX = oldX + (bx + bw / 2 - ox) * oldScale,
        oldPointY = oldY + (by + bh / 2 - oy) * oldScale;
      const desiredScale = Math.min(rect.width / (bw * 1.65), rect.height / (bh * 1.65));
      const ratio = desiredScale / oldScale;
      assert(Math.abs(ratio - 1) > 0.001, "Focus must zoom the whole-map native view");
      nativeCanvas.dispatchEvent(
        new WheelEvent("wheel", {
          bubbles: true,
          cancelable: true,
          clientX: rect.left + (rect.width / 2 - ratio * oldPointX) / (1 - ratio),
          clientY: rect.top + (rect.height / 2 - ratio * oldPointY) / (1 - ratio),
          deltaY: -Math.log(ratio) / 0.001,
        }),
      );
      await pause();
      focusedNativeCanvases.add(nativeCanvas);
    }
    const before = nativeCanvas.toDataURL("image/png");
    const view = document.querySelector<HTMLSelectElement>('[aria-label="State preview view"]')!;
    if ([...view.options].some((o) => o.value === "motion")) {
      view.value = "motion";
      view.dispatchEvent(new Event("change", { bubbles: true }));
      await pause();
    }
    let meshes = 0;
    internals.stateDelivery.physical.traverse((node) => {
      if ((node as THREE.Mesh).isMesh) meshes++;
    });
    assert(meshes > 0, "No real candidate model meshes loaded");
    const cameraAccess = viewport as unknown as {
      lookState(position: THREE.Vector3, target: THREE.Vector3, frustum: number): unknown;
      applyState(state: unknown): void;
      activeCamera(): THREE.Camera;
      renderer: THREE.WebGLRenderer;
      scene: THREE.Scene;
    };
    internals.stateDelivery.physical.updateWorldMatrix(true, true);
    const modelBounds = new THREE.Box3().setFromObject(internals.stateDelivery.physical);
    assert(!modelBounds.isEmpty(), "Physical focus has no model bounds");
    const center = modelBounds.getCenter(new THREE.Vector3());
    const elevation = (config.focus.elevation_deg * Math.PI) / 180;
    const offset = new THREE.Vector3(0, Math.sin(elevation), Math.cos(elevation));
    const size = modelBounds.getSize(new THREE.Vector3()).length();
    cameraAccess.applyState(
      cameraAccess.lookState(center.clone().addScaledVector(offset, size * 3), center, size),
    );
    const camera = cameraAccess.activeCamera();
    camera.updateMatrixWorld();
    const projected = new THREE.Box2();
    for (const x of [modelBounds.min.x, modelBounds.max.x])
      for (const y of [modelBounds.min.y, modelBounds.max.y])
        for (const z of [modelBounds.min.z, modelBounds.max.z]) {
          const point = new THREE.Vector3(x, y, z).applyMatrix4(camera.matrixWorldInverse);
          projected.expandByPoint(new THREE.Vector2(point.x, point.y));
        }
    const extent = projected.getSize(new THREE.Vector2());
    const canvas = cameraAccess.renderer.domElement;
    const aspect = canvas.clientWidth / canvas.clientHeight;
    cameraAccess.applyState(
      cameraAccess.lookState(
        center.clone().addScaledVector(offset, size * 3),
        center,
        Math.max(extent.y, extent.x / aspect) * 0.7,
      ),
    );
    await pause();
    cameraAccess.renderer.render(cameraAccess.scene, cameraAccess.activeCamera());
    const after = canvas.toDataURL("image/png");
    return { native: before, physical: after };
  }
  const pixelHashes: string[] = [];
  const seenPhases = new Set<string>();
  for (let i = 0; i < config.expected_frames.length; i++) {
    if (config.live_bridge) allowedLiveSequence = i;
    else button("Next recorded snapshot").click();
    await until(() => {
      assert(!errors.length, errors.join("\n"));
      return batches.length === i + 1;
    });
    const batch = batches[i]!;
    if (config.live_bridge && i === 0)
      result.textContent = `LIVE_BRIDGE_CONNECTED engine${batch.simulation_tick}`;
    assert(batch.simulation_tick === config.expected_frames[i], "Wrong source clock");
    const sample = batch.samples[0]!;
    assert(sample.kind === "patch", "Expected bound patch sample");
    const phase = sample.phase;
    if (config.expected_phases) assert(phase === config.expected_phases[i], "Wrong source phase");
    const nativeHash = await pixels();
    pixelHashes.push(nativeHash);
    if (!config.expected_phases) assert(nativeHash === initial, "Initial native art changed");
    assert(!internals.stateDelivery.native.isPlaying, "Manual clock running");
    if (
      config.expected_phases &&
      (!seenPhases.has(phase) ||
        (phase === "forward" && config.expected_phases[i + 1] === "applied"))
    ) {
      seenPhases.add(phase);
      const label = `${phase}-engine${batch.simulation_tick}`;
      const pair = await captureViews(label);
      images[`${label}-native`] = pair.native;
      images[`${label}-physical`] = pair.physical;
      imageScopes[`${label}-native.png`] = `Actual native artwork canvas, recorded ${label}`;
      imageScopes[`${label}-physical.png`] = `Actual physical viewport, recorded ${label}`;
    }
  }
  if (config.expected_phases) {
    assert(
      ["initial", "forward", "applied"].every((p) => seenPhases.has(p)),
      "Missing transition phase",
    );
    assert(new Set(pixelHashes).size > 1, "Native output never changed across transition records");
    const physicalImages = Object.entries(images)
      .filter(([key]) => key.endsWith("-physical"))
      .map(([, value]) => value);
    assert(
      new Set(physicalImages).size > 1,
      "Physical viewport never changed across transition records",
    );
  } else {
    const pair = await captureViews(`initial-engine${config.expected_frames.at(-1)}`);
    images.before = pair.native;
    images.after = pair.physical;
    imageScopes["before.png"] = "Actual native artwork initial state, focused";
    imageScopes["after.png"] =
      "Actual physical viewport initial state, original-art camera elevation, focused";
  }
  if (config.visual_only) {
    Object.assign(window, {
      __migrationImages: images,
      __recordedCallerReport: {
        scope: config.scope,
        visual_only: true,
        live_bridge: !!config.live_bridge,
        frames: batches.map((b) => b.simulation_tick),
        screenshot_scopes: imageScopes,
        native_source_bounds: config.focus.native_bounds,
        recorded_transition_checked: !!config.expected_phases,
        behavioral_failure_checks_rerun: false,
        whole_map_proof: false,
      },
    });
    allowedLiveSequence = config.expected_frames.length;
    result.textContent = config.live_bridge
      ? "PASS: live observation bridge visual packet; no full-map or free-running-hook claim"
      : "PASS: recorded transition visual packet; prior behavioral acceptance is separate";
    return;
  }
  button("Reload state").click();
  await until(
    () =>
      !!document.querySelector('[aria-label="Recorded runtime capture"]') &&
      !button("Next recorded snapshot"),
  );
  assert(!button("Play").disabled, "Reload did not restore manual controls");
  result.textContent = "RUNNING: reload during pending capture read";
  // Retirement while the selected File is still being read must not reach the actual viewport.
  let release!: () => void;
  const wait = new Promise<void>((r) => (release = r));
  let reading = false;
  const originalRead = File.prototype.arrayBuffer;
  await load();
  const captureManifest = JSON.parse(
    await captures.find((f) => f.name === "runtime-capture.json")!.text(),
  ) as { snapshots: { file: string }[] };
  const firstSnapshot = captureManifest.snapshots[0]!.file;
  File.prototype.arrayBuffer = function () {
    if (this.name === firstSnapshot) {
      reading = true;
      return wait.then(() => originalRead.call(this));
    }
    return originalRead.call(this);
  };
  const count = batches.length;
  button("Next recorded snapshot").click();
  await until(() => reading);
  button("Reload state").click();
  release();
  File.prototype.arrayBuffer = originalRead;
  await pause();
  await until(() => !button("Next recorded snapshot") && manualReady());
  assert(batches.length === count, "Retired read reached viewport");
  await load();
  result.textContent = "RUNNING: sink failure invalidation";
  rejectSink = true;
  button("Next recorded snapshot").click();
  await until(() => errors.some((e) => e.includes("diagnostic sink failure")));
  assert(button("Next recorded snapshot").disabled, "Failed sink still advances");
  rejectSink = false;
  button("Reload state").click();
  await until(
    () =>
      !button("Next recorded snapshot") &&
      !!document.querySelector('[aria-label="Recorded runtime capture"]'),
  );
  result.textContent = "RUNNING: corrupt snapshot rejection";
  const bad = captures.map((f) => (f.name === firstSnapshot ? new File(["{}"], f.name) : f));
  await load(bad);
  button("Next recorded snapshot").click();
  await until(() => errors.some((e) => e.toLowerCase().includes("hash")));
  assert(button("Next recorded snapshot").disabled, "Corrupt snapshot still advances");
  setDoc({
    ...current,
    mission: { ...current.mission, importedFrom: "RetiredDiagnosticMission" } as Level3D["mission"],
  });
  await pause();
  assert(!button("Next recorded snapshot"), "Mission retirement retained capture");
  Object.assign(window, {
    __migrationImages: images,
    __recordedCallerReport: {
      scope: config.scope,
      frames: batches.map((b) => b.simulation_tick),
      native_initial_pixels_unchanged: !config.expected_phases,
      recorded_transition_checked: !!config.expected_phases,
      screenshot_scopes: imageScopes,
      screenshot_comparison: config.expected_phases
        ? "Recorded native/physical phase pairs"
        : "Native initial versus physical initial",
      live_trigger_parity: false,
      approved_asset_scope: config.scope === "approved-asset-recorded-integration-diagnostic",
      whole_map_proof: false,
      expected_errors: errors,
    },
  });
  result.textContent =
    "PASS: recorded StatePreview caller diagnostic; no live bridge or whole-map claim";
}
main().catch((e) => {
  result.textContent = `FAIL: ${e.stack ?? e}`;
});
