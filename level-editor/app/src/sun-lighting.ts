import * as THREE from "three";
import type { Level3D } from "@rle/shared";

/** Shadow receivers borrow terrain geometry; baked asset colors stay unchanged. */
export class SunLighting {
  readonly root = new THREE.Group();
  readonly sun = new THREE.DirectionalLight(0xfff2da, 1);
  private receiver: THREE.Object3D | null = null;
  private material = new THREE.ShadowMaterial({
    color: 0x26303a,
    side: THREE.DoubleSide,
    opacity: 0.45,
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
    this.root.add(this.sun, this.sun.target);
    this.root.visible = false;
  }
  setGround(ground: THREE.Object3D | null) {
    this.receiver?.removeFromParent();
    this.receiver = ground?.clone(true) ?? null;
    this.receiver?.traverse((node) => {
      if (!(node instanceof THREE.Mesh)) return;
      node.material = this.material;
      node.castShadow = false;
      node.receiveShadow = true;
      node.renderOrder = 2;
      node.raycast = () => {};
    });
    if (this.receiver) this.root.add(this.receiver);
  }
  sync(settings: Level3D["lighting"], casters: THREE.Object3D[], bounds: THREE.Box3) {
    this.root.visible = !!settings?.enabled;
    if (!settings?.enabled || bounds.isEmpty()) return;
    this.material.opacity = settings.shadowOpacity;
    for (const root of casters)
      root.traverse((node) => {
        if (node instanceof THREE.Mesh) node.castShadow = !node.userData.noSunShadow;
      });
    this.root.updateWorldMatrix(true, false);
    const center = this.root.worldToLocal(bounds.getCenter(new THREE.Vector3()));
    const radius = Math.max(100, bounds.getSize(new THREE.Vector3()).length() / 2);
    const azimuth = THREE.MathUtils.degToRad(settings.sunAzimuth);
    const elevation = THREE.MathUtils.degToRad(settings.sunElevation);
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
