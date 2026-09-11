import { render } from "@solidjs/web";
import { createSignal } from "solid-js";
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

function gate() {
  let release!: () => void;
  let enter!: () => void;
  const blocked = new Promise<void>((resolve) => (release = resolve));
  const entered = new Promise<void>((resolve) => (enter = resolve));
  return { release, enter, blocked, entered };
}

async function fixtures(names = ["a", "b"]) {
  const files = new Map<string, File>();
  let readGate: { name: string; gate: ReturnType<typeof gate> } | null = null;
  let writeGate: ReturnType<typeof gate> | null = null;
  for (const name of names) {
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
  const originalFiles = new Map(files);
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
      return {
        getFile: async () => {
          if (readGate?.name === name) {
            const pending = readGate.gate;
            readGate = null;
            pending.enter();
            await pending.blocked;
          }
          return file;
        },
        createWritable: async () => {
          let text = "";
          return {
            write: async (value: string) => {
              text = value;
              if (writeGate) {
                const pending = writeGate;
                writeGate = null;
                pending.enter();
                await pending.blocked;
              }
            },
            close: async () => {
              files.set(name, new File([text], name));
            },
            abort: async () => {},
          };
        },
      };
    },
    async *entries() {
      for (const name of files.keys()) yield [name, { kind: "file" }];
    },
  } as unknown as FileSystemDirectoryHandle;
  return {
    handle: directory,
    delayRead: (name: string) => {
      const pending = gate();
      readGate = { name, gate: pending };
      return pending;
    },
    delayWrite: () => {
      writeGate = gate();
      return writeGate;
    },
    resetFiles: () => {
      for (const [name, file] of originalFiles) files.set(name, file);
    },
  };
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
  const replacement = await fixtures(["c", "d"]);
  const [activeLibrary, setActiveLibrary] = createSignal(library);
  const errors: string[] = [];
  const samples: unknown[] = [];
  const baselineListeners = listeners.size;
  for (let mount = 0; mount < 4; mount++) {
    let status: string | null = null;
    const dispose = render(
      () => (
        <Editor3D
          index={() => null}
          library={activeLibrary}
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
    if (mount === 0) {
      const selectedMap = () =>
        document.querySelector(".editor-bar button.selected")?.textContent;
      const rows = () => document.querySelectorAll(".object-list li").length;
      const pending = library.delayRead("a-volumes.scene.glb");
      button("a");
      await pending.entered;
      button("b");
      await until(() => selectedMap() === "b" && status === null);
      pending.release();
      await pause();
      await pause();
      assert(
        selectedMap() === "b",
        "stale map load replaced its successor scene",
      );

      (document.querySelector(".object-list li") as HTMLElement).click();
      await pause();
      button("Duplicate");
      await until(() => rows() === 2);
      button("Undo");
      await until(() => rows() === 1);
      button("Redo");
      await until(() => rows() === 2);
      // Undo removed the selected duplicate; redo restores its document node,
      // not an obsolete selection binding. Select the restored row explicitly.
      (document.querySelectorAll(".object-list li")[1] as HTMLElement).click();
      await pause();
      const saving = library.delayWrite();
      button("Save *");
      await saving.entered;
      button("Duplicate");
      await until(() => rows() === 3);
      saving.release();
      await until(() => status === "saved b.level3d.json");
      assert(
        [...document.querySelectorAll("button")].some(
          (b) => b.textContent?.trim() === "Save *" && !b.disabled,
        ),
        "save completion cleared newer edits",
      );
      button("Undo");
      await until(() => rows() === 2);
      assert(
        [...document.querySelectorAll("button")].some(
          (b) => b.textContent?.trim() === "Save" && b.disabled,
        ),
        "undo did not return to the saved revision",
      );
      button("Redo");
      await until(() => rows() === 3);

      const oldLibrary = library.delayRead("a-volumes.scene.glb");
      button("a");
      await oldLibrary.entered;
      setActiveLibrary(replacement);
      await until(() =>
        [...document.querySelectorAll("button")].some(
          (b) => b.textContent === "c",
        ),
      );
      button("c");
      await until(() => selectedMap() === "c" && status === null);
      oldLibrary.release();
      await pause();
      await pause();
      assert(
        selectedMap() === "c",
        "retired library published into the replacement viewport",
      );
      library.resetFiles();
      setActiveLibrary(library);
      await until(() =>
        [...document.querySelectorAll("button")].some(
          (b) => b.textContent === "a",
        ),
      );
      assert(errors.length === 0, errors.join("\n"));
    }
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
    const unmountedLoad =
      mount === 3 ? library.delayRead("a-volumes.scene.glb") : null;
    if (unmountedLoad) {
      button("a");
      await unmountedLoad.entered;
    }
    dispose();
    unmountedLoad?.release();
    await pause();
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
  result.textContent = `PASS ${JSON.stringify({ mounts: 4, mapLoads: 34, rejectedStaleLoads: 3, behavior: ["reverse-load", "library-replacement", "undo-redo", "save-during-edit", "unmount-during-load"], samples, final: { gpu: gpuCounts(), listeners: listeners.size, observers, frames: frames.size } })}`;
}
main().catch((error) => {
  result.textContent = `FAIL ${error.stack ?? error}`;
});
