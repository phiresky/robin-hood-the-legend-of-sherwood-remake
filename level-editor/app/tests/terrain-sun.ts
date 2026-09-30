import * as THREE from "three";
import { createTerrainGrid, type Level3D } from "@rle/shared";
import { TerrainLayer } from "../src/terrain-layer.ts";
import { SunLighting } from "../src/sun-lighting.ts";

export function checkTerrainSunShadows() {
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
    const withShadows = render();
    let darkened = 0;
    for (let i = 0; i < without.length; i += 4) if (without[i]! - withShadows[i]! > 10) darkened++;
    if (darkened < 50)
      throw new Error(`Editable terrain did not receive sun shadows (${darkened} pixels)`);
    sun.sync(undefined, [caster], new THREE.Box3().setFromObject(terrain.root));
    const disabled = render();
    if (disabled.some((value, i) => value !== without[i]))
      throw new Error("Disabling sun left shadows on terrain");
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
