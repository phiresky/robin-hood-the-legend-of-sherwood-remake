import * as THREE from "three";
import { gameToScene, type Level3D } from "@rle/shared";
import { sceneryThumbnailFixture } from "./scenery-thumbnail-fixture.ts";
import { EditorViewport } from "../src/editor-viewport.ts";
import type { SceneryLayer } from "../src/scenery-layer.ts";

function directory(files: Map<string, Uint8Array>, prefix = ""): FileSystemDirectoryHandle {
  return {
    getDirectoryHandle: async (name: string) => directory(files, `${prefix}${name}/`),
    getFileHandle: async (name: string) => ({
      getFile: async () => {
        const bytes = files.get(prefix + name);
        if (!bytes) throw new Error(`Unexpected read: ${prefix}${name}`);
        return new File([new Uint8Array(bytes)], name);
      },
    }),
  } as unknown as FileSystemDirectoryHandle;
}
function check(condition: unknown, message: string) {
  if (!condition) throw new Error(message);
}
const errors: string[] = [];
let viewport: EditorViewport | undefined;
try {
  const { descriptor, files } = await sceneryThumbnailFixture(true, true);
  const pin = "0".repeat(64);
  files.set(
    "3d-assets/index.json",
    new TextEncoder().encode(
      JSON.stringify({
        version: 1,
        assets: [
          {
            id: descriptor.id,
            name: descriptor.name,
            source_map: descriptor.source_map,
            descriptor: "fire/asset.json",
            model: "fire/model.glb",
            descriptor_sha256: pin,
            editor: descriptor,
          },
        ],
      }),
    ),
  );
  let current: Level3D = {
    version: 1,
    map: "Scenery preview",
    size: [300, 300],
    camera: { kind: "oblique-orthographic", elevation_deg: 35 },
    groups: [],
    sceneAssets: [],
    assetSources: [
      {
        id: "fire",
        descriptor: "3d-assets/fire/asset.json",
        model: "3d-assets/fire/model.glb",
        descriptor_sha256: pin,
        model_sha256: pin,
        resources: descriptor.resources,
      },
    ],
    objects: [
      {
        id: "fire-1",
        name: "Fire",
        kind: "scenery",
        node: "asset:fire:scenery-effect",
        source: { map: "Authored" },
        transform: { dx: 100, dy: 150, dz: 20, rot_deg: 0 },
      },
    ],
  };
  viewport = new EditorViewport({
    document: () => current,
    selection: () => ({ kind: "part", id: "fire-1" }),
    level: () => null,
    showObstacles: () => false,
    showElevation: () => false,
    onSelection: () => {},
    commitTransform: () => {},
    onError: (message) => errors.push(message),
  });
  viewport.setup(document.querySelector<HTMLDivElement>("#view")!);
  viewport.replaceMap(
    new THREE.Group(),
    null,
    new Map([["asset:fire:scenery-effect", new THREE.Group()]]),
  );
  viewport.setSceneryLibrary(directory(files));
  viewport.syncViews(current);
  const internals = viewport as unknown as {
    scenery: SceneryLayer;
    partViews: Map<string, { wrapper: THREE.Group }>;
    prepareMapBake(document: Level3D): { root: THREE.Object3D };
    selectionBox: THREE.Box3Helper;
  };
  const layer = internals.scenery;
  const ready = async () => {
    const deadline = performance.now() + 10000;
    while (
      !layer.root.children.length ||
      !(layer.root.children[0] as THREE.Mesh<THREE.BufferGeometry, THREE.MeshBasicMaterial>)
        .material.map
    ) {
      if (errors.length || performance.now() > deadline)
        throw new Error(errors.join("; ") || "Scenery did not load");
      await new Promise((resolve) => setTimeout(resolve, 10));
    }
  };
  await ready();
  const mesh = layer.root.children[0] as THREE.Mesh<THREE.BufferGeometry, THREE.MeshBasicMaterial>;
  const [x, y, z] = gameToScene(current.camera, 100, 150, 20);
  check(
    mesh.position.distanceTo(new THREE.Vector3(x, z, -y)) < 1e-5,
    "Scenery anchor differs from exported placement",
  );
  const texture = mesh.material.map!;
  const firstGeometry = mesh.geometry;
  const canvas = texture.image as OffscreenCanvas;
  check(
    [...canvas.getContext("2d")!.getImageData(0, 0, 1, 1).data].join() === "0,0,0,0",
    "Legacy key not transparent",
  );
  const gpu = new THREE.WebGLRenderer({ alpha: true, preserveDrawingBuffer: true });
  gpu.setSize(2, 1);
  gpu.setClearColor(0, 0);
  const probeScene = new THREE.Scene();
  const probeMaterial = mesh.material.clone();
  probeScene.add(new THREE.Mesh(firstGeometry, probeMaterial));
  const camera = new THREE.OrthographicCamera(-1, 1, 1, 0, 0.1, 100);
  const angle = (current.camera.elevation_deg * Math.PI) / 180;
  camera.position.set(0, 10 * Math.sin(angle), 10 * Math.cos(angle));
  camera.lookAt(0, 0, 0);
  gpu.render(probeScene, camera);
  const rendered = new OffscreenCanvas(2, 1).getContext("2d")!;
  rendered.drawImage(gpu.domElement, 0, 0);
  check(
    [...rendered.getImageData(0, 0, 2, 1).data].join() === "0,0,0,0,255,0,0,255",
    "Live scenery GPU projection or transparency differs from source pixels",
  );
  probeMaterial.dispose();
  gpu.dispose();
  const frameTime = performance.now();
  const textures = new Set<THREE.Texture>();
  const leftEdges = new Set<number>();
  for (let tick = 0; tick < 16; tick++) {
    layer.update(frameTime + tick * 40);
    textures.add(mesh.material.map!);
    mesh.geometry.computeBoundingBox();
    leftEdges.add(mesh.geometry.boundingBox!.min.x);
  }
  check(
    textures.size === 2 && leftEdges.has(-1) && leftEdges.has(2),
    "Animation frames or offsets did not advance",
  );
  layer.update(frameTime);
  const bake = internals.prepareMapBake(current);
  check(!bake.root.getObjectByName(mesh.name), "Animated preview leaked into baked geometry");
  check(!internals.selectionBox.box.isEmpty(), "Effect-only selection has no bounds");
  // The same live wrapper that asset dragging changes must move the preview immediately.
  internals.partViews.get("fire-1")!.wrapper.position.x += 12;
  layer.update(performance.now());
  check(
    Math.abs(mesh.position.x - x - 12) < 1e-5,
    `Scenery did not follow live asset drag: ${mesh.position.x} != ${x + 12}; ${errors.join("; ")}`,
  );
  current = {
    ...current,
    objects: [
      ...current.objects,
      {
        ...current.objects[0]!,
        id: "fire-2",
        transform: { dx: 200, dy: 150, dz: 40, rot_deg: 180 },
      },
    ],
  };
  viewport.syncViews(current);
  await new Promise((resolve) => setTimeout(resolve, 20));
  check(layer.root.children.length === 2, "Copied effect missing");
  check(
    textures.has((layer.root.children[1] as typeof mesh).material.map!),
    "Copied effect reloaded shared sprite textures",
  );
  current = { ...current, objects: current.objects.map((part) => ({ ...part, hidden: true })) };
  viewport.syncViews(current);
  check(layer.root.children.length === 0, "Hidden effects remain visible");
  current = { ...current, objects: current.objects.map((part) => ({ ...part, hidden: false })) };
  viewport.syncViews(current);
  await ready();
  let disposed = false;
  texture.addEventListener("dispose", () => {
    disposed = true;
  });
  layer.clear();
  await Promise.resolve();
  check(disposed && layer.root.children.length === 0, "Scenery resources leaked on map retirement");
  check(errors.length === 0, errors.join("; "));
  document.querySelector("#result")!.textContent =
    "PASS live scenery: verified pixels, placement, drag, copy, hide, bake isolation and cleanup";
} catch (error) {
  document.querySelector("#result")!.textContent = `FAIL ${String(error)}`;
} finally {
  viewport?.dispose();
}
