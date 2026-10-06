import * as THREE from "../../app/node_modules/three/build/three.module.js";
import {
  listProjectionAssets,
  prepareProjectionAsset,
  loadProjectionAssetPreview,
} from "../../app/src/projection-library.ts";
import { insertProjectionAsset } from "../../app/src/asset-commands.ts";
import { disposeObjectResources } from "../../app/src/resources.ts";
import { parseLevel3D, assetNodeKey, type Level3D, type ProjectionAssetEntry } from "../../shared/src/index.ts";

import {PatchDisplay,isEffectivelyVisible} from "../../app/src/patch-display.ts";

function directory(base: string): FileSystemDirectoryHandle {
  return {
    async getDirectoryHandle(name: string) {
      return directory(base + "/" + name);
    },
    async getFileHandle(name: string) {
      const response = await fetch(base + "/" + name);
      if (!response.ok) throw new DOMException(base + "/" + name, "NotFoundError");
      const bytes = await response.arrayBuffer();
      return { getFile: async () => new File([bytes], name) };
    },
  } as unknown as FileSystemDirectoryHandle;
}
let root: FileSystemDirectoryHandle,
  entries: ProjectionAssetEntry[] = [],
  owned: THREE.Object3D | null = null;
const renderer = new THREE.WebGLRenderer({
  canvas: document.querySelector("canvas")!,
  preserveDrawingBuffer: true,
});
renderer.setSize(900, 700);
const scene = new THREE.Scene();
scene.background = new THREE.Color("#555");
scene.add(new THREE.HemisphereLight(0xffffff, 0x999999, 2));
const light = new THREE.DirectionalLight(0xffffff, 2);
light.position.set(1, 2, 3);
scene.add(light);
const camera = new THREE.OrthographicCamera(-1,1,1,-1,0.01,1e7);
function check(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}
function retire() {
  if (owned) {
    scene.remove(owned);
    disposeObjectResources([owned]);
    owned = null;
  }
  renderer.renderLists.dispose();
  renderer.info.reset();
}
function display(object: THREE.Object3D) {
  retire();
  owned = object;
  scene.add(object);
  object.updateMatrixWorld(true);
  const box = new THREE.Box3().setFromObject(object),
    center = box.getCenter(new THREE.Vector3()),
    size = box.getSize(new THREE.Vector3()).length();
  camera.position
    .copy(center)
    .add(new THREE.Vector3(0, Math.sin(35*Math.PI/180), Math.cos(35*Math.PI/180)).multiplyScalar(size * 2));
  camera.near = Math.max(0.01, size / 10000);
  camera.far = size * 10;
  camera.updateProjectionMatrix();
  camera.lookAt(center);camera.updateMatrixWorld(true);
  let halfHeight=1;
  for(const x of [box.min.x,box.max.x])for(const y of [box.min.y,box.max.y])for(const z of [box.min.z,box.max.z]){
    const point=new THREE.Vector3(x,y,z).applyMatrix4(camera.matrixWorldInverse);
    halfHeight=Math.max(halfHeight,Math.abs(point.y),Math.abs(point.x)/(900/700));
  }
  halfHeight*=1.12;camera.left=-halfHeight*900/700;camera.right=halfHeight*900/700;camera.top=halfHeight;camera.bottom=-halfHeight;camera.updateProjectionMatrix();
  renderer.render(scene, camera);
}
function renderResourceProbe(material: THREE.Material) {
  check(owned === null, "Resource probe requires an empty asset scene");
  const geometry = new THREE.BoxGeometry(1, 1, 1);
  const mesh = new THREE.Mesh(geometry, material);
  const probeCamera = new THREE.PerspectiveCamera(35, 900 / 700, 0.01, 10);
  probeCamera.position.z = 3;
  scene.add(mesh);
  try {
    renderer.render(scene, probeCamera);
  } finally {
    scene.remove(mesh);
    geometry.dispose();
    material.dispose();
    renderer.renderLists.dispose();
    renderer.render(scene, camera);
  }
}
// PBR rendering lazily allocates renderer-owned lighting textures. Measure
// those before loading assets so retirement still rejects every asset leak.
const coldMemory = { ...renderer.info.memory };
renderResourceProbe(new THREE.MeshStandardMaterial());
const rendererBaseline = { ...renderer.info.memory };
check(rendererBaseline.geometries === 0, "Warmup geometry was not disposed");
const api = {
  resourceBaseline() {
    return { cold: coldMemory, warmed: rendererBaseline };
  },
  resourceLeakProbe() {
    const texture = new THREE.DataTexture(new Uint8Array([255, 255, 255, 255]), 1, 1);
    texture.needsUpdate = true;
    let leaked: typeof rendererBaseline;
    try {
      renderResourceProbe(new THREE.MeshStandardMaterial({ map: texture }));
      // Deliberately retain only the asset texture until the counter is read.
      leaked = { ...renderer.info.memory };
    } finally {
      texture.dispose();
      renderer.render(scene, camera);
    }
    return { leaked, cleaned: { ...renderer.info.memory } };
  },
  async configure(path: string) {
    retire();
    const flat = await fetch("/@fs" + path + "/index.json");
    root = flat.ok
      ? ({
          async getDirectoryHandle(name: string) {
            if (name !== "3d-assets") throw new Error("Unexpected staged root directory");
            return directory("/@fs" + path);
          },
        } as unknown as FileSystemDirectoryHandle)
      : directory("/@fs" + path);
    entries = await listProjectionAssets(root);
    const raw = await (
      await fetch("/@fs" + path + (flat.ok ? "/index.json" : "/3d-assets/index.json"))
    ).json();
    for (const item of raw.assets)
      if (!entries.some((e) => e.id === item.id))
        entries.push({
          ...item,
          descriptor: "3d-assets/" + item.descriptor,
          model: "3d-assets/" + item.model,
          ...(item.preview_model ? { preview_model: "3d-assets/" + item.preview_model } : {}),
        });
    return entries.map((e) => ({
      id: e.id,
      model_scene: e.model_scene,
      preview_model: e.preview_model,
    }));
  },
  async verify(id: string) {
    retire();
    const entry = entries.find((e) => e.id === id);
    check(entry, "Missing entry " + id);
    document.querySelector("#status")!.textContent = id;
    let prepared = await prepareProjectionAsset(root, entry, entry.source_map);
    const base: Level3D = {
      version: 1,
      map: "Verification",
      size: [100, 100],
      camera: { kind: "oblique-orthographic", elevation_deg: 35 },
      sceneAssets: [],
      groups: [],
      objects: [],
    };
    const inserted = insertProjectionAsset(
      base,
      prepared.descriptor,
      prepared.reference,
      [0, 0, 0],
      [],
      prepared.appearanceIds,
    ).document;
    const saved = parseLevel3D(JSON.parse(JSON.stringify(inserted)));
    check(saved.objects.length === prepared.descriptor.parts.length, "Inserted part count");
    for (const part of prepared.descriptor.parts) {
      const object = saved.objects.find(
        (p) => p.node === assetNodeKey(prepared.descriptor.id, part.node),
      );
      check(object && !!object.hidden === !!part.default_hidden, "Hidden flag " + part.node);
    }
    const result = {
      id,
      scene: entry.model_scene,
      parts: prepared.sources.size,
      reference: prepared.reference,
      preview: !!entry.preview_model,
    };
    disposeObjectResources([prepared.asset]);
    const reference = saved.assetSources![0]!;
    prepared = await prepareProjectionAsset(root, reference, saved.map, reference);
    check(prepared.sources.size === result.parts, "Reload part count");
    for (const part of prepared.descriptor.parts)
      prepared.sources.get(assetNodeKey(prepared.descriptor.id, part.node))!.visible =
        !part.default_hidden;
    new PatchDisplay().apply(prepared.asset);
    display(prepared.asset);
    return result;
  },
  async hallState(first: string, second: string) {
    check(owned,"No installed asset");
    check([first,second].every(value=>value==='initial'||value==='applied'),"Invalid hall endpoint");
    const display=new PatchDisplay();display.set('appearance-1',first==='applied');display.set('appearance-2',second==='applied');display.apply(owned);
    const meshes:string[]=[];owned.traverse(node=>{if((node as THREE.Mesh).isMesh&&isEffectivelyVisible(node))meshes.push(node.parent!.name);});
    check(meshes.length===(second==='initial'?15:14),"Hall endpoint mesh count");
    check(meshes.every(name=>name.endsWith('--'+first+'-'+second)),"Wrong hall endpoint visible");
    renderer.render(scene,camera);
    return {state:first+'-'+second,meshes:meshes.length,camera:'native orthographic elevation35; whole bounding box fitted'};
  },
  async preview(id: string) {
    const entry = entries.find((e) => e.id === id);
    check(entry, "Missing entry");
    retire();
    const preview = await loadProjectionAssetPreview(root, entry);
    display(preview);
    return { id, scene: preview.name, meshes: renderer.info.render.triangles };
  },
  retire() {
    retire();
    renderer.render(scene, camera);
    return { ...renderer.info.memory };
  },
};
Object.assign(window, { assetSceneVerification: api });
