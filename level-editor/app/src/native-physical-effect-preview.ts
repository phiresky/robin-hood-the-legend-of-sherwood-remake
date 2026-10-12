import * as THREE from "three";
import type { NativeLoopDrawSnapshot, NativePixels } from "./native-state-presentation.ts";
import type { MissionStateContract } from "../../shared/src/mission-state.ts";
import {
  nativePresentationFrame,
  type NativeStatePresentationContract,
} from "../../shared/src/native-state-presentation.ts";

export interface PhysicalEffectBinding {
  object: THREE.Object3D;
  tick(): number;
  binding: MissionStateContract["targets"][number];
}

/** The viewport's Y-up coordinates projected into the untouched source-art frame. */
export function nativeEffectCamera(box: THREE.Box3, elevation: number) {
  if (!Number.isFinite(elevation) || elevation <= 0 || elevation >= 90 || box.isEmpty())
    throw new Error("Invalid native physical effect camera");
  const angle = (elevation * Math.PI) / 180,
    sin = Math.sin(angle),
    cos = Math.cos(angle);
  const projected = new THREE.Box2();
  for (const x of [box.min.x, box.max.x])
    for (const y of [box.min.y, box.max.y])
      for (const z of [box.min.z, box.max.z])
        projected.expandByPoint(new THREE.Vector2(x, z * sin - y * cos));
  const x = Math.floor(projected.min.x) - 1,
    y = Math.floor(projected.min.y) - 1;
  const width = Math.ceil(projected.max.x) + 1 - x,
    height = Math.ceil(projected.max.y) + 1 - y;
  const center = new THREE.Vector3(x + width / 2, 0, (y + height / 2) / sin);
  const distance =
    box.getSize(new THREE.Vector3()).length() +
    center.distanceTo(box.getCenter(new THREE.Vector3())) +
    100;
  const camera = new THREE.OrthographicCamera(
    -width / 2,
    width / 2,
    height / 2,
    -height / 2,
    0.1,
    distance * 3,
  );
  camera.position.copy(center).addScaledVector(new THREE.Vector3(0, sin, cos), distance);
  camera.lookAt(center);
  camera.updateMatrixWorld(true);
  return { camera, x, y, width, height };
}

/** Explicit preview only: actual sampled model geometry over the native receiver.
 * The existing compositor still owns source ordering, actor masks, and clocks. */
export class NativePhysicalEffectPreview {
  private readonly bound = new Map<string, PhysicalEffectBinding>();
  private readonly native: NativeStatePresentationContract;
  constructor(native: NativeStatePresentationContract, rows: readonly PhysicalEffectBinding[]) {
    this.native = structuredClone(native);
    for (const row of rows) {
      if (row.binding.source_kind !== "map-animation") continue;
      const element = native.elements.find(
        (e) => e.source.kind === "map-animation" && e.source.index === row.binding.target_index,
      );
      if (
        !element ||
        element.source.sha256 !== row.binding.target_sha256 ||
        this.bound.has(element.id)
      )
        throw new Error("Physical effect source identity differs");
      this.bound.set(element.id, row);
    }
    if (!this.bound.size) throw new Error("No source-bound physical map effects");
  }
  sample(renderer: THREE.WebGLRenderer, snapshot: NativeLoopDrawSnapshot): NativeLoopDrawSnapshot {
    if (snapshot.mission !== this.native.mission)
      throw new Error("Retired physical effect preview");
    return {
      ...snapshot,
      draws: snapshot.draws.map((draw) => {
        const bound = this.bound.get(draw.element.id);
        if (!bound) return draw;
        const root = bound.object;
        const expected = this.native.elements.find((e) => e.id === draw.element.id)!;
        if (draw.element.active && nativePresentationFrame(expected, bound.tick()) !== draw.frame)
          throw new Error("Physical effect pose and native frame differ");
        if (expected.frames.some((f) => f.shadow_key))
          throw new Error("Physical effect destination keying requires a separate proof");
        if (JSON.stringify(expected.source) !== JSON.stringify(draw.element.source))
          throw new Error("Physical effect snapshot identity changed");
        if (!draw.element.active || !root.visible)
          return { ...draw, pixels: { width: 1, height: 1, data: new Uint8Array(4) } };
        root.updateWorldMatrix(true, true);
        root.traverseVisible((node) => {
          if (!(node instanceof THREE.Mesh)) return;
          if (
            node instanceof THREE.SkinnedMesh ||
            (Array.isArray(node.material) ? node.material : [node.material]).some(
              (m) => !(m instanceof THREE.MeshBasicMaterial),
            )
          )
            throw new Error("Physical native preview requires unlit, unskinned source effects");
        });
        const clone = root.clone(true);
        clone.matrixAutoUpdate = false;
        clone.matrix.copy(root.matrixWorld);
        const scene = new THREE.Scene();
        scene.add(clone);
        scene.updateMatrixWorld(true);
        const view = nativeEffectCamera(
          new THREE.Box3().setFromObject(clone),
          this.native.camera_elevation_deg,
        );
        const pixels = renderPhysicalEffect(renderer, scene, view.camera, view.width, view.height);
        return { ...draw, pixels, x: view.x - snapshot.origin[0], y: view.y - snapshot.origin[1] };
      }),
    };
  }
}

export function renderPhysicalEffect(
  renderer: THREE.WebGLRenderer,
  scene: THREE.Scene,
  camera: THREE.Camera,
  width: number,
  height: number,
): NativePixels {
  if (renderer.xr.enabled || Math.max(width, height) > renderer.capabilities.maxTextureSize)
    throw new Error("Physical effect preview exceeds renderer limits");
  const target = renderer.getRenderTarget(),
    cube = renderer.getActiveCubeFace(),
    mip = renderer.getActiveMipmapLevel();
  const viewport = renderer.getViewport(new THREE.Vector4()),
    scissor = renderer.getScissor(new THREE.Vector4());
  const scissorTest = renderer.getScissorTest(),
    autoClear = renderer.autoClear;
  const clearColor = renderer.getClearColor(new THREE.Color()),
    clearAlpha = renderer.getClearAlpha();
  const toneMapping = renderer.toneMapping;
  const output = new THREE.WebGLRenderTarget(width, height, {
    depthBuffer: true,
    colorSpace: THREE.SRGBColorSpace,
  });
  try {
    renderer.setRenderTarget(output);
    renderer.setViewport(0, 0, width, height);
    renderer.setScissorTest(false);
    renderer.autoClear = false;
    renderer.toneMapping = THREE.NoToneMapping;
    renderer.setClearColor(0, 0);
    renderer.clear(true, true, false);
    renderer.render(scene, camera);
    const bytes = new Uint8Array(width * height * 4),
      data = new Uint8Array(bytes.length);
    renderer.readRenderTargetPixels(output, 0, 0, width, height, bytes);
    for (let y = 0; y < height; y++)
      for (let x = 0; x < width; x++) {
        const from = ((height - 1 - y) * width + x) * 4,
          to = (y * width + x) * 4,
          alpha = bytes[from + 3]!;
        if (alpha !== 0 && alpha !== 255)
          throw new Error("Physical effect preview requires binary cutout alpha");
        for (let c = 0; c < 3; c++) data[to + c] = alpha ? bytes[from + c]! : 0;
        data[to + 3] = alpha;
      }
    return { width, height, data };
  } finally {
    renderer.setRenderTarget(target, cube, mip);
    renderer.setViewport(viewport);
    renderer.setScissor(scissor);
    renderer.setScissorTest(scissorTest);
    renderer.setClearColor(clearColor, clearAlpha);
    renderer.autoClear = autoClear;
    renderer.toneMapping = toneMapping;
    output.dispose();
  }
}
