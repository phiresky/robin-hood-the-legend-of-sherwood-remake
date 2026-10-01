import * as THREE from "three";
import type { Level3D } from "@rle/shared";

/** Direct terrain lighting, with shadow overlays for baked ground artwork. */
export class SunLighting {
  readonly root = new THREE.Group();
  // Neutral light preserves the texture color on unshadowed horizontal terrain.
  readonly sun = new THREE.DirectionalLight(0xffffff, 1);
  readonly ambient = new THREE.AmbientLight(0xffffff, Math.PI);
  private receiver: THREE.Object3D | null = null;
  private material = new THREE.ShadowMaterial({
    color: 0x26303a,
    side: THREE.DoubleSide,
    depthWrite: false,
    polygonOffset: true,
    polygonOffsetFactor: -1,
    polygonOffsetUnits: -2,
  });
  constructor() {
    this.sun.castShadow = true;
    this.sun.shadow.mapSize.set(2048, 2048);
    this.sun.shadow.normalBias = 1;
    this.sun.shadow.bias = -0.0001;
    this.sun.shadow.radius = 2;
    this.root.add(this.sun, this.sun.target, this.ambient);
    this.sun.visible = false;
  }
  setGround(ground: THREE.Object3D | null) {
    this.receiver?.removeFromParent();
    this.receiver = ground?.clone(true) ?? null;
    const litTerrain: THREE.Mesh[] = [];
    this.receiver?.traverse((node) => {
      if (!(node instanceof THREE.Mesh)) return;
      // Lit terrain receives shadows directly; only baked artwork needs an overlay.
      if (node.userData.terrainSurface) {
        litTerrain.push(node);
        return;
      }
      node.material = this.material;
      node.castShadow = false;
      node.receiveShadow = true;
      node.renderOrder = 2;
      node.raycast = () => {};
    });
    for (const node of litTerrain) node.removeFromParent();
    if (this.receiver instanceof THREE.Mesh && this.receiver.userData.terrainSurface)
      this.receiver = null;
    if (this.receiver) this.root.add(this.receiver);
  }
  sync(settings: Level3D["lighting"], casters: THREE.Object3D[], bounds: THREE.Box3) {
    this.root.visible = true;
    this.sun.visible = !!settings?.enabled;
    if (this.receiver) this.receiver.visible = !!settings?.enabled;
    this.ambient.intensity = Math.PI * (settings?.enabled ? 0.35 : 1);
    if (!settings?.enabled || bounds.isEmpty()) return;
    this.sun.shadow.intensity = settings.shadowOpacity;
    for (const root of casters)
      root.traverse((node) => {
        if (node instanceof THREE.Mesh) node.castShadow = !node.userData.noSunShadow;
      });
    this.root.updateWorldMatrix(true, false);
    const center = this.root.worldToLocal(bounds.getCenter(new THREE.Vector3()));
    const radius = Math.max(100, bounds.getSize(new THREE.Vector3()).length() / 2);
    const azimuth = THREE.MathUtils.degToRad(settings.sunAzimuth);
    const elevation = THREE.MathUtils.degToRad(settings.sunElevation);
    // Keep horizontal terrain brightness stable while slopes respond to sun direction.
    this.sun.intensity = (Math.PI * 0.65) / Math.sin(elevation);
    this.sun.target.position.copy(center);
    this.sun.position
      .copy(center)
      .addScaledVector(
        new THREE.Vector3(
          Math.sin(azimuth) * Math.cos(elevation),
          Math.cos(azimuth) * Math.cos(elevation),
          Math.sin(elevation),
        ),
        radius * 2,
      );
    const camera = this.sun.shadow.camera;
    camera.left = camera.bottom = -radius;
    camera.right = camera.top = radius;
    camera.near = 1;
    camera.far = radius * 4;
    camera.updateProjectionMatrix();
    this.sun.shadow.needsUpdate = true;
  }
  dispose() {
    this.setGround(null);
    this.sun.shadow.dispose();
    this.material.dispose();
    this.root.removeFromParent();
  }
}
