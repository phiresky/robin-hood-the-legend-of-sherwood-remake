import { listFiles } from "../src/fs";
import { render } from "@solidjs/web";
import * as THREE from "three";
import { GLTFExporter } from "three/examples/jsm/exporters/GLTFExporter.js";
import Editor3D from "../src/Editor3D";
import { EditorViewport } from "../src/editor-viewport";
import { ASSET_DRAG_TYPE } from "../src/asset-library";
import { disposeObjectResources } from "../src/resources";
import { SplineLayer } from "../src/spline-layer";
import type { LevelSpline } from "@rle/shared";

const assert = (value: unknown, message: string) => {
  if (!value) throw new Error(message);
};

function checkPathControlVisibility() {
  const renderer = new THREE.WebGLRenderer();
  const target = new THREE.WebGLRenderTarget(64, 64);
  const layer = new SplineLayer();
  const scene = new THREE.Scene();
  scene.add(layer.root);
  const camera = new THREE.OrthographicCamera(-40, 40, 40, -40, 1, 1000);
  camera.position.z = 500;
  camera.lookAt(0, 0, 0);
  const path: LevelSpline = {
    id: "visibility",
    name: "Path",
    kind: "road",
    points: [
      [-100, 0, 0],
      [0, 0, 0],
      [100, 0, 0],
    ],
    width: 80,
    repeatLength: 100,
    closed: false,
  };
  try {
    layer.sync([path], { kind: "oblique-orthographic", elevation_deg: 35 }, new Map());
    layer.setMode({ path, drawing: false, point: 1, append() {}, move() {}, selectPoint() {} });
    renderer.setRenderTarget(target);
    const pixel = () => {
      renderer.render(scene, camera);
      const rgba = new Uint8Array(4);
      renderer.readRenderTargetPixels(target, 32, 32, 1, 1, rgba);
      return [...rgba];
    };
    const covered = pixel();
    for (const child of layer.root.children) if (child !== layer.controls) child.visible = false;
    const uncovered = pixel();
    assert(
      uncovered[0]! > uncovered[2]!,
      "Visibility fixture must sample the selected yellow handle",
    );
    assert(
      covered.every((value, i) => value === uncovered[i]),
      "Transparent path surface obscured its control point",
    );
  } finally {
    layer.clear();
    target.dispose();
    renderer.dispose();
    renderer.forceContextLoss();
  }
}
async function until(test: () => boolean) {
  for (let i = 0; i < 150; i++) {
    if (test()) return;
    await new Promise((resolve) => setTimeout(resolve, 40));
  }
  throw new Error(
    "Shared library acceptance timed out; inspector: " +
      [...document.querySelectorAll(".editor-modes button")].map((b) => b.outerHTML).join(" "),
  );
}

export async function checkSharedLibrary() {
  checkPathControlVisibility();
  let viewport: EditorViewport | undefined;
  const setup = EditorViewport.prototype.setup;
  EditorViewport.prototype.setup = function (element) {
    viewport = this;
    setup.call(this, element);
  };
  const elevation = () => {
    const camera = (viewport as unknown as { camera: THREE.Camera }).camera;
    return THREE.MathUtils.radToDeg(Math.asin(-camera.getWorldDirection(new THREE.Vector3()).y));
  };
  const renderedGroups = () =>
    (viewport as unknown as { objectsRoot: THREE.Group }).objectsRoot.children.length;
  const panTarget = () =>
    (viewport as unknown as { orbit: { target: THREE.Vector3 } }).orbit.target.clone();
  const pathPoints = () =>
    (viewport as unknown as { splineMode: { path: LevelSpline } }).splineMode.path.points;
  const cornerSource = () =>
    (viewport as unknown as { splineMode?: { path: LevelSpline } }).splineMode?.path.cornerAsset;
  const oldPresets = localStorage.getItem("rle.wallPresets");
  localStorage.setItem(
    "rle.wallPresets",
    JSON.stringify([
      {
        name: "Battlement wall",
        asset: "house",
        axis: "x",
        width: 30,
        repeatLength: 100,
        cornerAsset: "nottingham-castle-east-round-tower",
      },
    ]),
  );
  const files = new Map<string, File>();
  const savedMaps = new Set<string>();
  const json = (name: string, value: unknown) =>
    files.set(name, new File([JSON.stringify(value)], name));
  const obstacle = {
    points: [
      { x: 0, y: 0, z_bottom: 0, z_top: 30 },
      { x: 30, y: 0, z_bottom: 0, z_top: 30 },
      { x: 0, y: 30, z_bottom: 0, z_top: 30 },
    ],
    solid: true,
    opaque: true,
    mouse: true,
    show_shadow_polygon: false,
    default_material: 0,
    material_indices: [],
    projection_area: null,
  };
  const camera = { kind: "oblique-orthographic", elevation_deg: 35 };
  async function model(id?: string) {
    const root = new THREE.Group();
    root.name = "map";
    root.rotation.x = -Math.PI / 2;
    const group = new THREE.Group();
    group.name = "buildings";
    if (id) group.userData.asset_group = id;
    root.add(group);
    const mesh = new THREE.Mesh(
      new THREE.BoxGeometry(30, 30, 30),
      new THREE.MeshBasicMaterial({ color: 0x46bbaa }),
    );
    mesh.position.z = 15;
    mesh.name = "building-000";
    mesh.userData.source_obstacle = 0;
    group.add(mesh);
    const bytes = (await new GLTFExporter().parseAsync(root, { binary: true })) as ArrayBuffer;
    disposeObjectResources([root]);
    return new File([bytes], "model.glb");
  }
  for (const name of ["York", "Lincoln"]) {
    const baseModel = await model();
    files.set(`3d-assets/base/${name}.glb`, baseModel);
    const digest = Array.from(
      new Uint8Array(await crypto.subtle.digest("SHA-256", await baseModel.arrayBuffer())),
      (byte) => byte.toString(16).padStart(2, "0"),
    ).join("");
    json(`scenes/${name}-volumes.scene.json`, {
      version: 1,
      map: name,
      size: [400, 400],
      camera,
      placements: [],
    });
    json(`scenes/${name}.rhlos-map.json`, {
      version: 1,
      map: name,
      size: [400, 400],
      camera,
      sceneAssets: [
        {
          id: name,
          role: "objects",
          model: `3d-assets/base/${name}.glb`,
          model_sha256: digest,
          resources: [],
        },
      ],
      groups: [],
      objects: [
        {
          id: "building-000",
          node: "building-000",
          kind: "building",
          source: { map: name, obstacle: 0 },
          obstacle,
          transform: { dx: 0, dy: 0, dz: 0, rot_deg: 0 },
        },
      ],
    });
  }
  const entries = [
    {
      id: "house",
      name: "Stone House",
      source_map: "Leicester",
      asset_type: "Building",
      tags: ["stone"],
    },
    { id: "tree", name: "Oak Tree", source_map: "Derby", asset_type: "Vegetation", tags: ["oak"] },
    ...Array.from({ length: 38 }, (_, index) => ({
      id: index === 0 ? "nottingham-castle-east-round-tower" : `prop-${index}`,
      name: index === 0 ? "Round corner tower" : `Courtyard prop ${index}`,
      source_map: "York",
      asset_type: "Prop",
      tags: ["courtyard"],
    })),
  ].map((entry) => ({
    ...entry,
    descriptor: `${entry.id}/asset.json`,
    model: `${entry.id}/model.glb`,
  }));
  for (const entry of entries) {
    files.set(`3d-assets/${entry.model}`, await model(entry.id));
    json(`3d-assets/${entry.descriptor}`, {
      version: 1,
      kind: "projection-mapped-asset",
      ...entry,
      model: "model.glb",
      source_origin_scene: [0, 0, 0],
      source_origin_game: [0, 0, 0],
      parts: [
        {
          node: "building-000",
          name: entry.name,
          source_obstacle: 0,
          obstacle_local_game: obstacle,
        },
      ],
    });
  }
  const digest = async (file: File) =>
    Array.from(
      new Uint8Array(await crypto.subtle.digest("SHA-256", await file.arrayBuffer())),
      (byte) => byte.toString(16).padStart(2, "0"),
    ).join("");
  json("3d-assets/index.json", {
    version: 1,
    assets: await Promise.all(
      entries.map(async (entry) => ({
        ...entry,
        editor: JSON.parse(await files.get(`3d-assets/${entry.descriptor}`)!.text()),
        descriptor_sha256: await digest(files.get(`3d-assets/${entry.descriptor}`)!),
        model_sha256: await digest(files.get(`3d-assets/${entry.model}`)!),
      })),
    ),
  });
  const publishedMaps = new Map(
    [...files].filter(([name]) => name.startsWith("scenes/") && name.endsWith(".rhlos-map.json")),
  );
  let modelReads = 0;
  const handle = (prefix: string): FileSystemDirectoryHandle =>
    ({
      name: "shared-library-fixture",
      kind: "directory",
      async getDirectoryHandle(name: string) {
        const next = prefix + name + "/";
        if (![...files.keys()].some((path) => path.startsWith(next)))
          throw new DOMException(name, "NotFoundError");
        return handle(next);
      },
      async getFileHandle(name: string, options?: { create?: boolean }) {
        const original = publishedMaps.get(prefix + name);
        name = name.replace(" (Modified).rhlos-map.json", ".rhlos-map.json");
        const path = prefix + name;
        if (!files.has(path) && !options?.create) throw new DOMException(path, "NotFoundError");
        return {
          getFile: async () => {
            if (path.endsWith(".glb")) modelReads++;
            return original ?? files.get(path)!;
          },
          createWritable: async () => {
            let value: BlobPart = "";
            return {
              write: async (text: BlobPart) => {
                value = text;
              },
              close: async () => {
                files.set(path, new File([value], name));
                savedMaps.add(name);
              },
              abort: async () => {},
            };
          },
        };
      },
      async removeEntry(name: string) {
        files.delete(prefix + name);
      },
      async *entries() {
        for (const path of files.keys())
          if (path.startsWith(prefix) && !path.slice(prefix.length).includes("/"))
            yield [path.slice(prefix.length), { kind: "file" }];
      },
    }) as unknown as FileSystemDirectoryHandle;
  const library = {
    handle: handle(""),
    documentMap: (name: string) => name.replace(" (Modified)", ""),
    savedMapName: (name: string) =>
      ["York", "Lincoln"].includes(name) ? name + " (Modified)" : name,
    mapLabels: async () =>
      new Map(
        ["York", "Lincoln"].flatMap(
          (name) =>
            [
              [name, name],
              ...(savedMaps.has(`${name}.rhlos-map.json`)
                ? [[name + " (Modified)", name + " (Modified)"]]
                : []),
            ] as [string, string][],
        ),
      ),
  };
  const host = document.querySelector("#root") as HTMLElement;
  const previousDisplay = host.style.display;
  const previousDirection = host.style.flexDirection;
  host.style.display = "flex";
  host.style.flexDirection = "column";
  const errors: string[] = [];
  const dispose = render(
    () => (
      <Editor3D
        index={() => null}
        library={() => library}
        onError={(error) => errors.push(error)}
        onStatus={() => {}}
      />
    ),
    document.querySelector("#root")!,
  );
  const click = (label: string) => {
    const button = [...document.querySelectorAll("button")].find(
      (button) =>
        button.textContent?.trim() === label ||
        button.querySelector("strong")?.textContent === label,
    );
    assert(button && !button.disabled, `Missing enabled button: ${label}`);
    button!.click();
  };
  const select = async (label: string, value: string) => {
    if (label === "Map") {
      document.querySelector<HTMLButtonElement>('[aria-label="Close map"]')?.click();
      await until(() => !!document.querySelector(`[data-map="${value}"]`));
      document.querySelector<HTMLButtonElement>(`[data-map="${value}"]`)!.click();
      return;
    }
    const element = document.querySelector(`select[aria-label="${label}"]`) as HTMLSelectElement;
    element.value = value;
    element.dispatchEvent(new Event("change", { bubbles: true }));
  };
  try {
    assert(
      !document.querySelector(".editor-modes"),
      "Editing modes should be absent before loading a map",
    );
    assert(
      ![...document.querySelectorAll(".editor-bar button")].some(
        (b) => b.textContent?.trim() === "View settings",
      ),
      "View settings should be absent before loading a map",
    );
    assert(
      Math.abs(elevation() - 35) < 1e-8,
      "Initial camera differs from the default map elevation",
    );
    await until(
      () =>
        (document.querySelector('select[aria-label="Source level"]') as HTMLSelectElement)
          ?.value === "refined-levels",
    );
    await until(() => document.querySelectorAll(".asset-library-host .asset-card").length === 2);
    await select("Source level", "");
    await until(() => document.querySelectorAll(".asset-library-host .asset-card").length === 40);
    await select("Map", "York");
    await until(() => !!document.querySelector("[data-map-name]"));
    await until(
      () => !document.querySelector(".asset-library-host .asset-card:first-child .preview-status"),
    );
    assert(
      !(document.querySelector(".spline-panel") as HTMLElement).checkVisibility(),
      "Drawing controls clutter the initial inspector",
    );
    assert(
      !(document.querySelector(".view-settings") as HTMLElement).checkVisibility(),
      "View controls clutter the initial inspector",
    );
    await select("Map", "York");
    await until(() => !!document.querySelector("[data-map-name]"));
    assert(document.querySelector(".editor-bar .editor-modes"), "Modes must be in the title bar");
    assert(
      [...document.querySelectorAll(".editor-modes button")]
        .map((b) => b.textContent?.trim())
        .join(",") === "Assets,Paths,Terrain,Mission",
      "Unexpected editing modes",
    );
    click("Help");
    await until(() => document.querySelector("#editor-help h2")?.textContent === "Assets controls");
    click("Terrain");
    await until(
      () => document.querySelector("#editor-help h2")?.textContent === "Terrain controls",
    );
    assert(
      document.querySelector("#editor-help")!.textContent.includes("Subdivide"),
      "Terrain help lacks subdivision",
    );
    await until(
      () => !!document.querySelector("#asset-browser .material-picker")?.checkVisibility(),
    );
    assert(
      !document.querySelector(".asset-library-host")?.checkVisibility(),
      "Terrain should replace the asset library",
    );
    assert(
      !document.querySelector(".editor-panel .material-picker"),
      "Material library remains in inspector",
    );
    click("Mission");
    await until(
      () => document.querySelector("#editor-help h2")?.textContent === "Mission controls",
    );
    assert(
      !document.querySelector('#asset-browser select[aria-label="Mission"]'),
      "Maps without original missions should not show an empty loader",
    );
    assert(
      !document.querySelector("#editor-help")!.textContent.includes("Load a mission"),
      "Mission help should not advertise an unavailable loader",
    );
    assert(
      !document.querySelector('.editor-bar select[aria-label="Mission"]'),
      "Mission loader remains in title bar",
    );
    assert(
      document
        .querySelector('#asset-browser select[aria-label="Character category"]')
        ?.checkVisibility(),
      "Character category must be in left library",
    );
    click("Paths");
    await until(() => document.querySelector("#editor-help h2")?.textContent === "Paths controls");
    await until(
      () => !!document.querySelector("#asset-browser .spline-preset-grid")?.checkVisibility(),
    );
    assert(
      !document.querySelector(".editor-panel .spline-preset-grid"),
      "Path library remains in inspector",
    );
    click("Help");
    click("Assets");
    await until(() => !!document.querySelector(".asset-library-host")?.checkVisibility());
    await until(
      () => !document.querySelector(".asset-library-host .asset-card:first-child .preview-status"),
    );
    await new Promise((resolve) => setTimeout(resolve, 200));
    const readsBeforeModeSwitch = modelReads;
    click("Terrain");
    await new Promise((resolve) => setTimeout(resolve, 100));
    click("Assets");
    await new Promise((resolve) => setTimeout(resolve, 150));
    assert(
      modelReads === readsBeforeModeSwitch,
      "Returning to Assets reloaded cached preview GLBs",
    );
    const originalWidth = document.querySelector(".editor-canvas")!.getBoundingClientRect().width;
    (document.querySelector('button[aria-label="Hide asset library"]') as HTMLElement).click();
    await until(
      () => !(document.querySelector(".library-content") as HTMLElement).checkVisibility(),
    );
    assert(
      (document.querySelector(".library-heading h2") as HTMLElement).checkVisibility(),
      "Collapsed library lost its title",
    );
    assert(
      document.querySelector(".editor-canvas")!.getBoundingClientRect().width > originalWidth,
      "Hiding assets did not expand the viewport",
    );
    (document.querySelector('button[aria-label="Show asset library"]') as HTMLElement).click();
    await until(() =>
      (document.querySelector(".library-content") as HTMLElement).checkVisibility(),
    );
    const resizer = document.querySelector(".library-resizer") as HTMLElement;
    const width = document.querySelector("#asset-browser")!.getBoundingClientRect().width;
    resizer.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }));
    await until(
      () => document.querySelector("#asset-browser")!.getBoundingClientRect().width > width,
    );
    resizer.dispatchEvent(new KeyboardEvent("keydown", { key: "End", bubbles: true }));
    await until(
      () =>
        getComputedStyle(document.querySelector(".asset-grid")!).gridTemplateColumns.split(" ")
          .length > 2,
    );
    const resized = document.querySelector("#asset-browser")!.getBoundingClientRect().width;
    (document.querySelector('button[aria-label="Hide asset library"]') as HTMLElement).click();
    await until(
      () => !(document.querySelector(".library-content") as HTMLElement).checkVisibility(),
    );
    (document.querySelector('button[aria-label="Show asset library"]') as HTMLElement).click();
    await until(
      () => document.querySelector("#asset-browser")!.getBoundingClientRect().width === resized,
    );
    click("Help");
    await until(() => !!document.querySelector("#editor-help"));
    click("Help");
    await until(() => !document.querySelector("#editor-help"));
    const grid = document.querySelector(".asset-grid") as HTMLElement;
    const firstCard = grid.querySelector(".asset-card") as HTMLElement;
    const preview = firstCard.querySelector(".asset-preview") as HTMLElement;
    const info = firstCard.querySelector(".asset-card-info") as HTMLElement;
    const bounds = firstCard.getBoundingClientRect();
    assert(
      bounds.height >= preview.getBoundingClientRect().height + info.getBoundingClientRect().height,
      "Catalog rows clipped the preview or asset details",
    );
    assert(preview.getBoundingClientRect().height > 70, "Preview collapsed in a full catalog");
    assert(
      grid.scrollHeight > grid.clientHeight,
      "Full catalog must scroll rather than compress its rows",
    );
    const lastCard = grid.lastElementChild as HTMLElement;
    grid.scrollTop = grid.scrollHeight;
    await new Promise((resolve) => requestAnimationFrame(resolve));
    assert(
      lastCard.getBoundingClientRect().bottom <= grid.getBoundingClientRect().bottom + 1,
      "Last asset cannot be reached by scrolling",
    );
    grid.scrollTop = 0;
    const canvas = document.querySelector(".asset-preview canvas") as HTMLCanvasElement;
    const pixels = canvas.getContext("2d")!.getImageData(0, 0, canvas.width, canvas.height).data;
    assert(
      pixels.some((value, index) => index % 4 === 3 && value > 0),
      "3D preview did not render any geometry",
    );
    await select("Asset type", "Building");
    await until(() => document.querySelectorAll(".asset-library-host .asset-card").length === 1);
    await select("Source level", "Derby");
    await until(() => document.querySelectorAll(".asset-library-host .asset-card").length === 0);
    await select("Source level", "Leicester");
    await until(() => document.querySelectorAll(".asset-library-host .asset-card").length === 1);
    await select("Map", "York");
    await until(
      () => document.querySelector("[data-map-name]")?.getAttribute("data-map-name") === "York",
    );
    await until(
      () =>
        document.querySelector(".asset-library-host .asset-card")?.getAttribute("draggable") ===
        "true",
    );
    const card = document.querySelector(".asset-library-host .asset-card")!;
    const transfer = new DataTransfer();
    card.dispatchEvent(new PointerEvent("pointerenter"));
    await until(() => {
      card.dispatchEvent(new DragEvent("dragstart", { bubbles: true, dataTransfer: transfer }));
      return transfer.getData(ASSET_DRAG_TYPE) === "house";
    });
    assert(transfer.getData(ASSET_DRAG_TYPE) === "house", "Drag did not identify the asset");
    const viewport = document.querySelector(".editor-canvas")!;
    const rect = viewport.getBoundingClientRect();
    const beforeDrag = renderedGroups();
    const moveDrag = () =>
      viewport.dispatchEvent(
        new DragEvent("dragover", {
          bubbles: true,
          cancelable: true,
          dataTransfer: transfer,
          clientX: rect.left + rect.width * 0.6,
          clientY: rect.top + rect.height * 0.6,
        }),
      );
    moveDrag();
    await until(() => renderedGroups() === beforeDrag + 1);
    assert(
      document.querySelectorAll(".object-list li").length === 1,
      "Dragging committed an edit before release",
    );
    document
      .querySelector(".shared-library")!
      .dispatchEvent(new DragEvent("dragenter", { bubbles: true, dataTransfer: transfer }));
    await until(() => renderedGroups() === beforeDrag);
    moveDrag();
    await until(() => renderedGroups() === beforeDrag + 1);
    viewport.dispatchEvent(
      new DragEvent("drop", {
        bubbles: true,
        cancelable: true,
        dataTransfer: transfer,
        clientX: rect.left + rect.width * 0.6,
        clientY: rect.top + rect.height * 0.6,
      }),
    );
    await until(
      () => document.querySelector(".editor-status")?.textContent === "Added Stone House",
    );
    const x = document.querySelector('input[aria-label="X"]') as HTMLInputElement;
    const originalX = Number(x.value);
    const currentX = () =>
      Number((document.querySelector('input[aria-label="X"]') as HTMLInputElement).value);
    // Synthetic pointers cannot capture a native pointer; exercise the real component handlers and history.
    const row = x.closest(".meta-row") as HTMLElement;
    const label = row.querySelector(".meta-key")!;
    const capture = row.setPointerCapture,
      release = row.releasePointerCapture;
    row.setPointerCapture = () => {};
    row.releasePointerCapture = () => {};
    try {
      label.dispatchEvent(
        new PointerEvent("pointerdown", { bubbles: true, button: 0, pointerId: 1, clientX: 100 }),
      );
      x.dispatchEvent(
        new PointerEvent("pointermove", { bubbles: true, pointerId: 1, clientX: 120 }),
      );
      x.dispatchEvent(
        new PointerEvent("pointermove", { bubbles: true, pointerId: 1, clientX: 140 }),
      );
      x.dispatchEvent(new PointerEvent("pointerup", { bubbles: true, pointerId: 1, clientX: 140 }));
      await until(() => Math.abs(currentX() - originalX - 40) < 0.01);
      click("Undo");
      await until(() => currentX() === originalX);
      assert(
        document.querySelectorAll(".object-list li").length > 1,
        "Scrub created extra history entries or undid insertion",
      );
    } finally {
      row.setPointerCapture = capture;
      row.releasePointerCapture = release;
    }
    click("Save *");
    await until(
      () =>
        ![...document.querySelectorAll("button")].some(
          (button) => button.textContent?.trim() === "Save *",
        ),
    );
    await until(
      () => document.querySelector(".active-map-name")?.textContent === "York (Modified)",
    );
    assert(
      document.querySelector(".document-state")?.textContent?.includes("York (Modified)"),
      "Document status must label the modified copy",
    );
    const saved = JSON.parse(await files.get("scenes/York.rhlos-map.json")!.text());
    assert(
      saved.assetSources.some((source: { id: string }) => source.id === "house") &&
        saved.objects.some((part: { node: string }) => part.node === "asset:house:building-000"),
      "Cross-level source was lost",
    );
    assert(saved.groups[0].transform.dx !== 200, "Drop used map center instead of cursor");
    click("Undo");
    await until(() => document.querySelectorAll(".object-list li").length === 1);
    click("Redo");
    await until(() => document.querySelectorAll(".object-list li").length > 1);
    await select("Map", "Lincoln");
    await until(
      () => document.querySelector("[data-map-name]")?.getAttribute("data-map-name") === "Lincoln",
    );
    await select("Map", "York");
    await until(
      () => document.querySelector("[data-map-name]")?.getAttribute("data-map-name") === "York",
    );
    assert(
      document.querySelectorAll(".object-list li").length === 1,
      "Original map contains the saved edits",
    );
    await select("Map", "York (Modified)");
    await until(
      () =>
        document.querySelector("[data-map-name]")?.getAttribute("data-map-name") ===
        "York (Modified)",
    );
    assert(
      document.querySelectorAll(".object-list li").length > 1,
      "Saved cross-level asset failed to reload",
    );
    // Exercise actual viewport path handling. Synthetic pointer events cannot
    // acquire native pointer capture, so the fixture supplies that browser API.
    const drawingCanvas = document.querySelector(".editor-canvas canvas") as HTMLCanvasElement;
    drawingCanvas.setPointerCapture = () => {};
    drawingCanvas.releasePointerCapture = () => {};
    drawingCanvas.hasPointerCapture = () => false;
    const drawPoint = async (x: number, y: number) => {
      await new Promise((resolve) => requestAnimationFrame(resolve));
      const rect = drawingCanvas.getBoundingClientRect();
      const init = {
        bubbles: true,
        cancelable: true,
        pointerId: 1,
        button: 0,
        clientX: rect.left + rect.width * x,
        clientY: rect.top + rect.height * y,
      };
      drawingCanvas.dispatchEvent(new PointerEvent("pointerdown", init));
      drawingCanvas.dispatchEvent(new PointerEvent("pointerup", init));
      await new Promise((resolve) => requestAnimationFrame(resolve));
    };
    // The resize checks leave only ~100 px of canvas. Give control-point hit
    // targets room so the next click appends instead of moving the first point.
    document.querySelector<HTMLButtonElement>('[aria-label="Hide asset library"]')!.click();
    await until(
      () => !(document.querySelector(".library-content") as HTMLElement).checkVisibility(),
    );
    click("Terrain");
    await until(() =>
      (document.querySelector(".terrain-settings") as HTMLElement).checkVisibility(),
    );
    assert(
      !(document.querySelector(".spline-panel") as HTMLElement).checkVisibility(),
      "Paths visible in Terrain mode",
    );
    click("Paths");
    await until(() => (document.querySelector(".spline-panel") as HTMLElement).checkVisibility());
    assert(
      !(document.querySelector(".terrain-settings") as HTMLElement).checkVisibility(),
      "Terrain visible in Paths mode",
    );
    click("Terrain");
    await until(() =>
      (document.querySelector(".terrain-settings") as HTMLElement).checkVisibility(),
    );
    click("Paths");
    await until(() => (document.querySelector(".spline-panel") as HTMLElement).checkVisibility());
    click("River");
    await until(() => !!document.querySelector('input[aria-label="Path name"]'));
    await until(
      () =>
        (
          [...document.querySelectorAll(".editor-modes button")].find(
            (b) => b.textContent === "Assets",
          ) as HTMLButtonElement
        ).disabled,
    );
    const panRect = drawingCanvas.getBoundingClientRect();
    const panStart = panTarget();
    const panEvent = {
      bubbles: true,
      cancelable: true,
      pointerId: 1,
      pointerType: "mouse",
      button: 0,
      buttons: 1,
      clientX: panRect.left + panRect.width / 2,
      clientY: panRect.top + panRect.height / 2,
    };
    drawingCanvas.dispatchEvent(new PointerEvent("pointerdown", panEvent));
    drawingCanvas.dispatchEvent(
      new PointerEvent("pointermove", { ...panEvent, clientX: panEvent.clientX + 40 }),
    );
    assert(panTarget().distanceTo(panStart) > 0.01, "Dragging while drawing must pan the camera");
    // Returning to the starting position still counts as a drag, not a click.
    drawingCanvas.dispatchEvent(new PointerEvent("pointermove", panEvent));
    drawingCanvas.dispatchEvent(new PointerEvent("pointerup", { ...panEvent, buttons: 0 }));
    assert(pathPoints().length === 0, "Camera panning must not append a path point");
    await drawPoint(0.25, 0.45);
    await drawPoint(0.5, 0.5);
    await drawPoint(0.75, 0.65);
    click("Finish path");
    await until(() =>
      [...document.querySelectorAll("button")].some((b) => b.textContent === "Done editing"),
    );
    click("Done editing");
    await until(() => document.querySelectorAll(".spline-list button").length === 1);
    click("Save *");
    await until(
      () =>
        ![...document.querySelectorAll("button")].some(
          (button) => button.textContent?.trim() === "Save *",
        ),
    );
    const riverSaved = JSON.parse(await files.get("scenes/York.rhlos-map.json")!.text());
    assert(riverSaved.splines[0].points.length === 3, "River control points were not saved");
    click("Undo");
    await until(() => document.querySelectorAll(".spline-list button").length === 0);
    click("Redo");
    await until(() => document.querySelectorAll(".spline-list button").length === 1);
    await until(() => !!document.querySelector(".spline-preset-grid"));
    click("Battlement wall");
    await until(
      () =>
        document.querySelector('input[aria-label="Path name"]')?.getAttribute("value") ===
          "Battlement wall" ||
        (document.querySelector('input[aria-label="Path name"]') as HTMLInputElement)?.value ===
          "Battlement wall",
    );
    await drawPoint(0.3, 0.7);
    await drawPoint(0.55, 0.75);
    await drawPoint(0.6, 0.45);
    await until(() => !!document.querySelector('input[aria-label="Corner tower scale"]'));
    click("Change corner type");
    await until(() => !!document.querySelector(".asset-picker-dialog[open]"));
    click("Round corner tower");
    await until(() => !document.querySelector(".asset-picker-dialog[open]"));
    const flip = document.querySelector(
      'input[aria-label="Flip battlement side"]',
    ) as HTMLInputElement;
    flip.checked = true;
    flip.dispatchEvent(new Event("change", { bubbles: true }));
    await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
    await until(() =>
      [...document.querySelectorAll("button")].some(
        (button) => button.textContent?.trim() === "Finish path" && !button.disabled,
      ),
    );
    click("Finish path");
    await until(() =>
      [...document.querySelectorAll("button")].some((b) => b.textContent === "Done editing"),
    );
    await until(() =>
      [...document.querySelectorAll("button")].some((b) => b.textContent === "Save as wall preset"),
    );
    click("Save as wall preset");
    click("Done editing");
    await until(() => !!document.querySelector(".spline-preset-grid"));
    click("Footpath");
    await until(
      () =>
        (document.querySelector('input[aria-label="Path name"]') as HTMLInputElement)?.value ===
        "Footpath",
    );
    await drawPoint(0.2, 0.25);
    await drawPoint(0.8, 0.8);
    click("Finish path");
    await until(() =>
      [...document.querySelectorAll("button")].some((b) => b.textContent === "Done editing"),
    );
    click("Done editing");
    await until(() => document.querySelectorAll(".spline-list button").length === 3);
    click("View settings");
    await until(() =>
      (
        document.querySelector('input[aria-label="Cast sun shadows"]') as HTMLElement
      ).checkVisibility(),
    );
    const sun = document.querySelector('input[aria-label="Cast sun shadows"]') as HTMLInputElement;
    sun.checked = true;
    sun.dispatchEvent(new Event("change", { bubbles: true }));
    await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
    click("Save *");
    await until(
      () =>
        ![...document.querySelectorAll("button")].some(
          (button) => button.textContent?.trim() === "Save *",
        ),
    );
    assert(
      JSON.parse(await files.get("scenes/York.rhlos-map.json")!.text()).splines?.find(
        (path: { kind: string }) => path.kind === "wall",
      )?.flipCrossSection === true,
      "Battlement-side choice was not saved",
    );
    assert(
      JSON.parse(await files.get("scenes/York.rhlos-map.json")!.text()).lighting?.enabled === true,
      "Sun settings were not saved",
    );
    const pathsSaved = JSON.parse(await files.get("scenes/York.rhlos-map.json")!.text());
    assert(
      pathsSaved.splines.some(
        (p: { kind: string; cornerAsset?: string }) =>
          p.kind === "wall" && p.cornerAsset === "nottingham-castle-east-round-tower",
      ),
      "Corner tower source was not saved",
    );
    assert(
      pathsSaved.splines.some((p: { kind: string }) => p.kind === "road"),
      "Footpath was not saved",
    );
    await select("Map", "Lincoln");
    await until(
      () => document.querySelector("[data-map-name]")?.getAttribute("data-map-name") === "Lincoln",
    );
    click("Paths");
    await until(() => !!document.querySelector(".spline-preset-grid"));
    click("Battlement wall");
    await until(() => !!document.querySelector('input[aria-label="Corner tower scale"]'));
    await until(() => cornerSource() === "nottingham-castle-east-round-tower");
    assert(
      cornerSource() === "nottingham-castle-east-round-tower",
      "Preset did not restore its tower across levels",
    );
    click("Cancel");
    await select("Map", "York (Modified)");
    await until(
      () =>
        document.querySelector("[data-map-name]")?.getAttribute("data-map-name") ===
        "York (Modified)",
    );
    assert(
      document.querySelectorAll(".spline-list button").length === 3,
      "River, wall and footpath failed to reload",
    );
    document.querySelector<HTMLButtonElement>('[aria-label="Close map"]')?.click();
    await until(() => !!document.querySelector(".map-selection"));
    click("New map");
    await until(() => (document.querySelector("dialog") as HTMLDialogElement).open);
    assert(
      document.querySelector('input[aria-label="Map width"]'),
      "Creating a map must expose workspace dimensions",
    );
    const preset = document.querySelector<HTMLSelectElement>('[aria-label="Map size preset"]')!;
    preset.value = "York";
    preset.dispatchEvent(new Event("change", { bubbles: true }));
    await new Promise((resolve) => requestAnimationFrame(resolve));
    const name = document.querySelector('input[aria-label="Map name"]') as HTMLInputElement;
    name.value = "New forest";
    name.dispatchEvent(new Event("input", { bubbles: true }));
    click("Create map");
    await until(
      () =>
        document.querySelector("[data-map-name]")?.getAttribute("data-map-name") === "New forest",
    );
    assert(document.querySelectorAll(".object-list li").length === 0, "New map inherited objects");
    assert(
      JSON.stringify(
        JSON.parse(await files.get("scenes/New forest.rhlos-map.json")!.text()).size,
      ) === "[3136,2318]",
      "New map did not use its named reference size",
    );
    const initialElevation = elevation();
    click("Game camera");
    await new Promise((resolve) => setTimeout(resolve, 900));
    assert(
      Math.abs(elevation() - initialElevation) < 1e-8,
      "Game camera changed the initial map elevation",
    );
    click("Add to scene");
    await until(() => document.querySelectorAll(".object-list li").length > 0);
    click("View settings");
    await until(() =>
      (document.querySelector(".export-settings") as HTMLElement).checkVisibility(),
    );
    click("Set export frame");
    await until(() => !!document.querySelector('input[aria-label="Export width"]'));
    const cropWidth = document.querySelector(
      'input[aria-label="Export width"]',
    ) as HTMLInputElement;
    cropWidth.value = "10";
    cropWidth.dispatchEvent(new Event("change", { bubbles: true }));
    await new Promise((resolve) => requestAnimationFrame(resolve));
    click("Save *");
    await until(
      () =>
        ![...document.querySelectorAll("button")].some((b) => b.textContent?.trim() === "Save *"),
    );
    const newSaved = JSON.parse(await files.get("scenes/New forest.rhlos-map.json")!.text());
    assert(
      newSaved.size[0] === 3136 &&
        newSaved.size[1] === 2318 &&
        newSaved.terrain.vertices.length > 0 &&
        newSaved.exportBounds[2] === 10,
      "Advisory crop changed canvas size or expanded to fit assets",
    );
    await select("Map", "York");
    await until(
      () => document.querySelector("[data-map-name]")?.getAttribute("data-map-name") === "York",
    );
    await select("Map", "New forest");
    await until(
      () =>
        document.querySelector("[data-map-name]")?.getAttribute("data-map-name") === "New forest",
    );
    assert(
      document.querySelectorAll(".object-list li").length > 0,
      "New map assets were not restored",
    );
    assert(
      (document.querySelector('input[aria-label="Export width"]') as HTMLInputElement)
        .valueAsNumber === 10,
      "Export frame was not restored",
    );
    const dropJson = (text: string, filename = "download_2026-09-26T16-30-12.rhlos-map.json") => {
      const transfer = new DataTransfer();
      transfer.items.add(new File([text], filename, { type: "application/json" }));
      document
        .querySelector(".editor-canvas")!
        .dispatchEvent(
          new DragEvent("drop", { bubbles: true, cancelable: true, dataTransfer: transfer }),
        );
    };
    const imported = { ...newSaved, exportBounds: [...newSaved.exportBounds] };
    imported.exportBounds[2] = 77;
    dropJson(JSON.stringify(imported));
    await until(
      () =>
        (document.querySelector('input[aria-label="Export width"]') as HTMLInputElement)
          ?.valueAsNumber === 77,
    );
    assert(
      document.querySelector(".document-state")?.textContent?.includes("Unsaved changes"),
      "Imported map must be unsaved",
    );
    assert(
      JSON.parse(await files.get("scenes/New forest.rhlos-map.json")!.text()).exportBounds[2] ===
        10,
      "Dropping JSON wrote a map before Save",
    );
    dropJson("{ invalid JSON");
    await until(() => errors.length > 0);
    errors.pop();
    assert(
      (document.querySelector('input[aria-label="Export width"]') as HTMLInputElement)
        .valueAsNumber === 77,
      "Invalid import replaced the open map",
    );
    dropJson(JSON.stringify({ ...imported, map: "York" }));
    await until(
      () =>
        document.querySelector("[data-map-name]")?.getAttribute("data-map-name") ===
        "York (Modified)",
    );
    assert(
      (await listFiles(await library.handle.getDirectoryHandle("scenes"))).includes(
        "York.rhlos-map.json",
      ),
      "Import removed the original map",
    );
    click("Save *");
    await until(
      () =>
        ![...document.querySelectorAll("button")].some((b) => b.textContent?.trim() === "Save *"),
    );
    await select("Map", "York");
    await until(
      () => document.querySelector("[data-map-name]")?.getAttribute("data-map-name") === "York",
    );
    assert(
      document.querySelectorAll(".object-list li").length === 1,
      "Imported map overwrote the original",
    );
    await select("Map", "York (Modified)");
    await until(
      () =>
        document.querySelector("[data-map-name]")?.getAttribute("data-map-name") ===
        "York (Modified)",
    );
    assert(
      (document.querySelector('input[aria-label="Export width"]') as HTMLInputElement)
        .valueAsNumber === 77,
      "Imported map failed to save and reload",
    );
    dropJson(JSON.stringify({ ...imported, map: "Dropped forest" }));
    await until(
      () =>
        document.querySelector("[data-map-name]")?.getAttribute("data-map-name") ===
        "Dropped forest",
    );
    await select("Map", "York");
    await until(
      () => document.querySelector("[data-map-name]")?.getAttribute("data-map-name") === "York",
    );
    assert(
      !document.querySelector('[data-map="Dropped forest"]'),
      "Discarded unsaved import left an unloadable map entry",
    );
    assert(errors.length === 0, errors.join("\n"));
  } catch (error) {
    throw new Error(
      `${error}; errors: ${errors.join("; ")}; UI: ${document.querySelector("#root")?.textContent}`,
      { cause: error },
    );
  } finally {
    EditorViewport.prototype.setup = setup;
    if (oldPresets === null) localStorage.removeItem("rle.wallPresets");
    else localStorage.setItem("rle.wallPresets", oldPresets);
    dispose();
    host.style.display = previousDisplay;
    host.style.flexDirection = previousDirection;
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
}
