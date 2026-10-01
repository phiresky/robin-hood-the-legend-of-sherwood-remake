import * as THREE from "three";
import { createTerrainGrid, type Level3D } from "@rle/shared";
import { EditorViewport } from "../src/editor-viewport.ts";
import { TerrainLayer } from "../src/terrain-layer.ts";
import { SunLighting } from "../src/sun-lighting.ts";
import { bakeScene, renderMapBake } from "../src/map-bake-render.ts";

export function checkTerrainSunShadows() {
  checkViewportSunUpdates();
  const renderer = new THREE.WebGLRenderer();
  renderer.shadowMap.enabled = true;
  renderer.shadowMap.type = THREE.PCFSoftShadowMap;
  const target = new THREE.WebGLRenderTarget(128, 128);
  const scene = new THREE.Scene();
  const terrain = new TerrainLayer();
  terrain.sync({
    camera: { kind: "oblique-orthographic", elevation_deg: 35 },
    terrain: createTerrainGrid([0, 0, 400, 400], 200, 0),
  } as Level3D);
  const caster = new THREE.Mesh(
    new THREE.BoxGeometry(60, 60, 100),
    new THREE.MeshBasicMaterial({ color: 0xffffff }),
  );
  caster.position.set(230, -350, 50);
  const sun = new SunLighting();
  scene.add(terrain.root, caster, sun.root);
  sun.setGround(terrain.root);
  const camera = new THREE.OrthographicCamera(-250, 250, 400, -400, 1, 2000);
  camera.position.set(200, -350, 1000);
  camera.lookAt(200, -350, 0);
  const render = () => {
    renderer.setRenderTarget(target);
    renderer.render(scene, camera);
    const data = new Uint8Array(128 * 128 * 4);
    renderer.readRenderTargetPixels(target, 0, 0, 128, 128, data);
    return data;
  };
  try {
    const without = render();
    sun.sync(
      { enabled: true, sunAzimuth: 90, sunElevation: 35, shadowOpacity: 0.7 },
      [caster],
      new THREE.Box3().setFromObject(terrain.root).expandByObject(caster),
    );
    sun.sun.shadow.intensity = 0;
    const litWithoutShadows = render();
    sun.sun.shadow.intensity = 0.7;
    const withShadows = render();
    let darkened = 0;
    for (let i = 0; i < without.length; i += 4)
      if (litWithoutShadows[i]! - withShadows[i]! > 10) darkened++;
    if (darkened < 50)
      throw new Error(`Editable terrain did not receive sun shadows (${darkened} pixels)`);
    sun.sync(undefined, [caster], new THREE.Box3().setFromObject(terrain.root));
    const disabled = render();
    if (disabled.some((value, i) => value !== without[i]))
      throw new Error("Disabling sun left shadows on terrain");
    // A raised ridge must shadow the flat land west of it, without object casters.
    caster.visible = false;
    sun.sync(
      { enabled: true, sunAzimuth: 305, sunElevation: 48, shadowOpacity: 0 },
      [terrain.root],
      new THREE.Box3().setFromObject(terrain.root),
    );
    const flatUnshadowed = render();
    sun.sun.shadow.intensity = 1;
    const flatShadowed = render();
    let flatDarkened = 0;
    for (let i = 0; i < flatUnshadowed.length; i += 4)
      if (flatUnshadowed[i]! - flatShadowed[i]! > 3) flatDarkened++;
    if (flatDarkened > 5)
      throw new Error(`Empty flat terrain shadows itself (${flatDarkened} pixels)`);
    const ridge = createTerrainGrid([0, 0, 400, 400], 50, 0);
    for (const vertex of ridge.vertices) if (vertex.position[0] === 250) vertex.position[2] = 120;
    terrain.sync({
      camera: { kind: "oblique-orthographic", elevation_deg: 35 },
      terrain: ridge,
    } as Level3D);
    sun.setGround(terrain.root);
    sun.sync(
      { enabled: true, sunAzimuth: 90, sunElevation: 35, shadowOpacity: 1 },
      [terrain.root],
      new THREE.Box3().setFromObject(terrain.root),
    );
    sun.sun.shadow.intensity = 0;
    const ridgeUnshadowed = render();
    sun.sun.shadow.intensity = 1;
    const ridgeShadowed = render();
    let terrainShadowPixels = 0;
    for (let i = 0; i < ridgeUnshadowed.length; i += 4)
      if (ridgeUnshadowed[i]! - ridgeShadowed[i]! > 10) terrainShadowPixels++;
    if (terrainShadowPixels < 100)
      throw new Error(`Terrain ridge did not cast shadows (${terrainShadowPixels} pixels)`);
    const bake = (shadowOpacity: number) =>
      renderMapBake(
        bakeScene([terrain.root]),
        { kind: "oblique-orthographic", elevation_deg: 35 },
        [0, 0, 400, 400],
        { enabled: true, sunAzimuth: 90, sunElevation: 35, shadowOpacity },
      );
    const unshadowedBake = bake(0);
    const shadowedBake = bake(1);
    let bakedShadowPixels = 0;
    for (let i = 0; i < shadowedBake.color.length; i += 4)
      if (unshadowedBake.color[i]! - shadowedBake.color[i]! > 10) bakedShadowPixels++;
    if (bakedShadowPixels < 100)
      throw new Error(`Export lost terrain shadows (${bakedShadowPixels} pixels)`);
    if (shadowedBake.depth.some((value, i) => value !== unshadowedBake.depth[i]))
      throw new Error("Lighting changed exported terrain depth");
  } finally {
    sun.dispose();
    terrain.clear();
    caster.geometry.dispose();
    caster.material.dispose();
    target.dispose();
    renderer.dispose();
    renderer.forceContextLoss();
  }
}

function checkViewportSunUpdates() {
  const grid = createTerrainGrid([0, 0, 400, 400], 50, 0);
  for (const vertex of grid.vertices) if (vertex.position[0] === 250) vertex.position[2] = 120;
  let document: Level3D = {
    version: 1,
    map: "shadow-regression",
    sceneAssets: [],
    objects: [],
    groups: [],
    size: [400, 400],
    camera: { kind: "oblique-orthographic", elevation_deg: 35 },
    terrain: grid,
  };
  // Model publication inside a reactive batch: the accessor still exposes the prior revision.
  let publishedDocument = document;
  const viewport = new EditorViewport({
    document: () => publishedDocument,
    selection: () => null,
    level: () => null,
    showObstacles: () => false,
    showElevation: () => false,
    onSelection: () => {},
    commitTransform: () => {},
  });
  const renderer = new THREE.WebGLRenderer();
  renderer.shadowMap.autoUpdate = false;
  renderer.shadowMap.type = THREE.PCFShadowMap;
  const internal = viewport as unknown as {
    renderer: THREE.WebGLRenderer | null;
    scene: THREE.Scene;
  };
  internal.renderer = renderer;
  const target = new THREE.WebGLRenderTarget(128, 128);
  const camera = new THREE.OrthographicCamera(-250, 250, 400, -400, 1, 2000);
  camera.position.set(200, 1000, 350);
  camera.up.set(0, 0, -1);
  camera.lookAt(200, 0, 350);
  const render = () => {
    renderer.setRenderTarget(target);
    renderer.render(internal.scene, camera);
    const pixels = new Uint8Array(128 * 128 * 4);
    renderer.readRenderTargetPixels(target, 0, 0, 128, 128, pixels);
    return pixels;
  };
  try {
    viewport.syncViews(document);
    const disabled = render();
    document = {
      ...document,
      lighting: { enabled: true, sunAzimuth: 90, sunElevation: 35, shadowOpacity: 0 },
    };
    viewport.syncViews(document, false);
    publishedDocument = document;
    const unshadowed = render();
    document = { ...document, lighting: { ...document.lighting!, shadowOpacity: 1 } };
    viewport.syncViews(document, false);
    publishedDocument = document;
    const shadowed = render();
    let changed = 0;
    for (let i = 0; i < shadowed.length; i += 4) if (unshadowed[i]! - shadowed[i]! > 10) changed++;
    if (changed < 100)
      throw new Error(`Incremental viewport lighting produced no shadows (${changed} pixels)`);
    document = { ...document, lighting: { ...document.lighting!, enabled: false } };
    viewport.syncViews(document, false);
    publishedDocument = document;
    if (render().some((value, i) => value !== disabled[i]))
      throw new Error("Incremental viewport lighting did not disable cleanly");
  } finally {
    internal.renderer = null;
    viewport.dispose();
    target.dispose();
    renderer.dispose();
    renderer.forceContextLoss();
  }
}
