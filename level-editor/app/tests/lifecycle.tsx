import { render } from "@solidjs/web";
import * as THREE from "three";
import { GLTFExporter } from "three/examples/jsm/exporters/GLTFExporter.js";
import Editor3D from "../src/Editor3D";
import "../src/styles.css";

// Browser acceptance fixture: no directory picker, disk writes or game data.
const result = document.querySelector("#result")!;
const assert = (value: unknown, message: string) => {
  if (!value) throw new Error(message);
};
const pause = () => new Promise((resolve) => setTimeout(resolve, 50));
async function until(test: () => boolean) {
  for (let i = 0; i < 100; i++) {
    if (test()) return;
    await pause();
  }
  throw new Error("Timed out waiting for editor");
}

const gpu = new Map<string, Set<unknown>>();
const contexts = new Map<unknown, WebGL2RenderingContext>();
const watched = new WeakSet<WebGL2RenderingContext>();
let contextLosses = 0;
for (const kind of [
  "Buffer",
  "Texture",
  "Program",
  "Shader",
  "Framebuffer",
  "Renderbuffer",
  "VertexArray",
]) {
  const owned = new Set<unknown>();
  gpu.set(kind, owned);
  const prototype = WebGL2RenderingContext.prototype as unknown as Record<
    string,
    (...args: any[]) => any
  >;
  const create = prototype[`create${kind}`]!;
  const destroy = prototype[`delete${kind}`]!;
  prototype[`create${kind}`] = function (...args) {
    const context = this as unknown as WebGL2RenderingContext;
    if (!watched.has(context)) {
      watched.add(context);
      add.call(
        context.canvas,
        "webglcontextlost",
        () => {
          contextLosses++;
          for (const resources of gpu.values())
            for (const resource of resources)
              if (contexts.get(resource) === context) {
                resources.delete(resource);
                contexts.delete(resource);
              }
        },
        { once: true },
      );
    }
    const resource = create.apply(this, args);
    if (resource) {
      owned.add(resource);
      contexts.set(resource, context);
    }
    return resource;
  };
  prototype[`delete${kind}`] = function (...args) {
    owned.delete(args[0]);
    contexts.delete(args[0]);
    return destroy.apply(this, args);
  };
}
const gpuCounts = () =>
  Object.fromEntries(
    [...gpu].map(([kind, resources]) => [kind, resources.size]),
  );
const frames = new Set<number>();
const requestFrame = window.requestAnimationFrame.bind(window);
const cancelFrame = window.cancelAnimationFrame.bind(window);
window.requestAnimationFrame = (callback) => {
  const id = requestFrame((time) => {
    frames.delete(id);
    callback(time);
  });
  frames.add(id);
  return id;
};
window.cancelAnimationFrame = (id) => {
  frames.delete(id);
  cancelFrame(id);
};
let observers = 0;
const NativeObserver = ResizeObserver;
window.ResizeObserver = class extends NativeObserver {
  active = false;
  override observe(target: Element, options?: ResizeObserverOptions) {
    if (!this.active) {
      observers++;
      this.active = true;
    }
    super.observe(target, options);
  }
  override disconnect() {
    if (this.active) {
      observers--;
      this.active = false;
    }
    super.disconnect();
  }
};
type Listener = {
  target: EventTarget;
  type: string;
  callback: EventListenerOrEventListenerObject | null;
  capture: boolean;
};
const listeners = new Set<Listener>();
const add = EventTarget.prototype.addEventListener;
const remove = EventTarget.prototype.removeEventListener;
EventTarget.prototype.addEventListener = function (type, callback, options) {
  if (this === window || this instanceof HTMLCanvasElement) {
    const capture = typeof options === "boolean" ? options : !!options?.capture;
    if (
      ![...listeners].some(
        (l) =>
          l.target === this &&
          l.type === type &&
          l.callback === callback &&
          l.capture === capture,
      )
    ) {
      const entry = { target: this, type, callback, capture };
      listeners.add(entry);
      if (typeof options === "object" && options.signal)
        add.call(options.signal, "abort", () => listeners.delete(entry), {
          once: true,
        });
    }
  }
  return add.call(this, type, callback, options);
};
EventTarget.prototype.removeEventListener = function (type, callback, options) {
  const capture = typeof options === "boolean" ? options : !!options?.capture;
  for (const l of listeners)
    if (
      l.target === this &&
      l.type === type &&
      l.callback === callback &&
      l.capture === capture
    )
      listeners.delete(l);
  return remove.call(this, type, callback, options);
};

async function fixtures() {
  const files = new Map<string, File>();
  for (const name of ["a", "b"]) {
    const root = new THREE.Group();
    root.name = "map";
    const buildings = new THREE.Group();
    buildings.name = "buildings";
    root.add(buildings);
    const canvas = document.createElement("canvas");
    canvas.width = 4;
    canvas.height = 4;
    const pixels = canvas.getContext("2d")!;
    pixels.fillStyle = "#33aa66";
    pixels.fillRect(0, 0, 4, 4);
    const texture = new THREE.CanvasTexture(canvas);
    const material = new THREE.MeshBasicMaterial({ map: texture });
    const mesh = new THREE.Mesh(new THREE.BoxGeometry(80, 100, 60), material);
    mesh.name = "building-000";
    buildings.add(mesh);
    const glb = await new GLTFExporter().parseAsync(root, { binary: true });
    assert(glb instanceof ArrayBuffer, "fixture GLB export");
    files.set(
      `${name}-volumes.scene.glb`,
      new File([glb as ArrayBuffer], "scene.glb"),
    );
    mesh.geometry.dispose();
    material.dispose();
    texture.dispose();
    const camera = { kind: "oblique-orthographic", elevation_deg: 35 };
    const scene = {
      version: 1,
      map: name,
      size: [400, 400],
      camera,
      placements: [],
    };
    files.set(
      `${name}-volumes.scene.json`,
      new File([JSON.stringify(scene)], "scene.json"),
    );
    const doc = {
      ...scene,
      glb: `${name}-volumes.scene.glb`,
      groups: [],
      objects: [
        {
          id: "building-000",
          node: "building-000",
          kind: "building",
          source: { map: name, obstacle: 0 },
          transform: { dx: 0, dy: 0, dz: 0, rot_deg: 0 },
          obstacle: {
            points: [
              { x: 0, y: 0, z_bottom: 0, z_top: 60 },
              { x: 80, y: 0, z_bottom: 0, z_top: 60 },
              { x: 80, y: 100, z_bottom: 0, z_top: 60 },
            ],
            opaque: true,
            solid: true,
            mouse: false,
            show_shadow_polygon: false,
            default_material: 0,
            material_indices: [],
            projection_area: {},
          },
        },
      ],
    };
    files.set(
      `${name}.level3d.json`,
      new File([JSON.stringify(doc)], "level3d.json"),
    );
  }
  const directory = {
    name: "memory-fixture",
    kind: "directory",
    async getDirectoryHandle(name: string) {
      if (name !== "scenes") throw new DOMException("missing", "NotFoundError");
      return this;
    },
    async getFileHandle(name: string) {
      const file = files.get(name);
      if (!file) throw new DOMException(name, "NotFoundError");
      return { getFile: async () => file };
    },
    async *entries() {
      for (const name of files.keys()) yield [name, { kind: "file" }];
    },
  } as unknown as FileSystemDirectoryHandle;
  return { handle: directory };
}
function button(label: string) {
  const button = [...document.querySelectorAll("button")].find(
    (b) => b.textContent?.trim() === label,
  );
  assert(button, `missing button ${label}`);
  button!.click();
}

async function main() {
  const library = await fixtures();
  const errors: string[] = [];
  const samples: unknown[] = [];
  const baselineListeners = listeners.size;
  for (let mount = 0; mount < 4; mount++) {
    let status: string | null = null;
    const dispose = render(
      () => (
        <Editor3D
          index={() => null}
          library={() => library}
          onError={(e) => errors.push(e)}
          onStatus={(s) => (status = s)}
        />
      ),
      document.querySelector("#root")!,
    );
    await until(() =>
      [...document.querySelectorAll("button")].some(
        (b) => b.textContent === "a",
      ),
    );
    let stable: string | null = null;
    for (let cycle = 0; cycle < 8; cycle++) {
      button(cycle % 2 ? "b" : "a");
      try {
        await until(() => {
          assert(errors.length === 0, errors.join("\n"));
          return (
            status === null &&
            document.querySelectorAll(".object-list li").length === 1
          );
        });
      } catch (error) {
        throw new Error(
          `mount=${mount} cycle=${cycle} status=${status} rows=${document.querySelectorAll(".object-list li").length}`,
          { cause: error },
        );
      }
      assert(errors.length === 0, errors.join("\n"));
      (document.querySelector(".object-list li") as HTMLElement).click();
      await pause();
      button("Duplicate");
      await pause();
      button("Delete");
      await pause();
      for (const checkbox of document.querySelectorAll<HTMLInputElement>(
        ".editor-bar input[type=checkbox]",
      )) {
        checkbox.checked = true;
        checkbox.dispatchEvent(new Event("change", { bubbles: true }));
      }
      await pause();
      for (const checkbox of document.querySelectorAll<HTMLInputElement>(
        ".editor-bar input[type=checkbox]",
      )) {
        checkbox.checked = false;
        checkbox.dispatchEvent(new Event("change", { bubbles: true }));
      }
      await pause();
      await new Promise(requestFrame);
      const sample = JSON.stringify({
        gpu: gpuCounts(),
        listeners: listeners.size,
        observers,
        frames: frames.size,
      });
      for (const context of new Set(contexts.values()))
        assert(
          context.getError() === context.NO_ERROR,
          "WebGL error after shared-source duplicate deletion",
        );
      if (cycle >= 2) {
        if (stable)
          assert(
            sample === stable,
            `Viewport resources changed after warmup: ${stable} -> ${sample}`,
          );
        stable = sample;
      }
      samples.push({
        mount,
        cycle,
        gpu: gpuCounts(),
        listeners: listeners.size,
        observers,
        frames: frames.size,
      });
    }
    dispose();
    await pause();
    assert(observers === 0, `ResizeObserver retained: ${observers}`);
    assert(frames.size === 0, `RAF retained: ${frames.size}`);
    assert(
      listeners.size === baselineListeners,
      `DOM listeners retained: ${listeners.size - baselineListeners}`,
    );
    assert(
      [...gpu.values()].every((s) => s.size === 0),
      `GPU resources retained: ${JSON.stringify(gpuCounts())}`,
    );
    assert(
      contextLosses === mount + 1,
      `viewport context not released: ${contextLosses}`,
    );
  }
  result.textContent = `PASS ${JSON.stringify({ mounts: 4, mapLoads: 32, samples, final: { gpu: gpuCounts(), listeners: listeners.size, observers, frames: frames.size } })}`;
}
main().catch((error) => {
  result.textContent = `FAIL ${error.stack ?? error}`;
});
