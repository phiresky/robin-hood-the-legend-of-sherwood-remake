import {
  prepareNativeExtractionContext,
  type NativeExtractionContext,
} from "./native-extraction-context.ts";
import * as THREE from "three";
import { hasQualifiedSourceDisplayShader } from "./texture-display.ts";
import type { NativeLoopDrawSnapshot, NativePixels } from "./native-state-presentation.ts";
import type { MissionStateContract } from "../../shared/src/mission-state.ts";
import {
  nativePresentationFrame,
  type NativeStatePresentationContract,
} from "../../shared/src/native-state-presentation.ts";

/** Source-emissive exports are sampled without lights or an environment. */
export function validateNativeEffectMaterial(material: THREE.Material) {
  if (material instanceof THREE.MeshBasicMaterial) return;
  if (
    !(material instanceof THREE.MeshStandardMaterial) ||
    material instanceof THREE.MeshPhysicalMaterial
  )
    throw new Error(
      "Physical native preview requires unlit or qualified source-emissive materials",
    );
  const map = material.map,
    emissive = material.emissiveMap;
  if (map?.matrixAutoUpdate) map.updateMatrix();
  if (emissive?.matrixAutoUpdate) emissive.updateMatrix();
  // Opaque timber exports carry their source RGB only in emission. With no
  // lights or environment, a black base adds no reflected contribution.
  const opaqueEmission =
    hasQualifiedSourceDisplayShader(material) &&
    material.color.equals(new THREE.Color(0, 0, 0)) &&
    material.emissive.equals(new THREE.Color(1, 1, 1)) &&
    material.emissiveIntensity === 1 &&
    emissive instanceof THREE.Texture &&
    emissive.channel === 0 &&
    map === null &&
    !material.vertexColors &&
    material.side === THREE.DoubleSide &&
    material.opacity === 1 &&
    !material.transparent &&
    !material.alphaHash &&
    !material.alphaToCoverage &&
    material.blending === THREE.NormalBlending &&
    !material.premultipliedAlpha &&
    material.alphaTest === 0 &&
    !material.alphaMap &&
    !material.envMap &&
    !material.lightMap &&
    !material.aoMap &&
    !material.normalMap &&
    !material.bumpMap &&
    !material.displacementMap &&
    !material.metalnessMap &&
    !material.roughnessMap &&
    !material.wireframe &&
    !material.flatShading &&
    !material.clippingPlanes?.length &&
    !material.userData.foliage_physical_opacity &&
    !material.userData.source_ownership_semantics &&
    (!material.defines || Object.keys(material.defines).every((key) => key === "STANDARD"));
  if (opaqueEmission) return;
  // Complementary exports carry color in UV0 emission and binary coverage in
  // UV1 base alpha. The exact black base contributes no reflected RGB.
  const complementaryEmission =
    material.userData.export_complementary_binary_ownership === true &&
    ["native", "inferred"].includes(material.userData.export_composite_role) &&
    material.onBeforeCompile === THREE.Material.prototype.onBeforeCompile &&
    material.customProgramCacheKey === THREE.Material.prototype.customProgramCacheKey &&
    material.color.equals(new THREE.Color(0, 0, 0)) &&
    material.emissive.equals(new THREE.Color(1, 1, 1)) &&
    material.emissiveIntensity === 1 &&
    map instanceof THREE.Texture && map.channel === 1 &&
    emissive instanceof THREE.Texture && emissive.channel === 0 &&
    material.metalness === 0 && material.roughness === 1 &&
    !material.vertexColors && material.side === THREE.DoubleSide &&
    material.opacity === 1 && !material.transparent && material.alphaTest === 0.5 &&
    !material.alphaHash && !material.alphaToCoverage &&
    material.blending === THREE.NormalBlending && !material.premultipliedAlpha &&
    material.depthTest && material.depthWrite && !material.stencilWrite && !material.polygonOffset &&
    !material.alphaMap && !material.envMap && !material.lightMap && !material.aoMap &&
    !material.normalMap && !material.bumpMap && !material.displacementMap &&
    !material.metalnessMap && !material.roughnessMap &&
    !material.wireframe && !material.flatShading && !material.clippingPlanes?.length &&
    !material.userData.foliage_physical_opacity &&
    !material.userData.source_ownership_semantics && !material.userData.source_ownership_fill &&
    (!material.defines || Object.keys(material.defines).every((key) => key === "STANDARD"));
  if (complementaryEmission) return;
  const sameTexture =
    map &&
    emissive &&
    map.source === emissive.source &&
    map.channel === emissive.channel &&
    map.matrix.equals(emissive.matrix) &&
    map.mapping === emissive.mapping &&
    map.wrapS === emissive.wrapS &&
    map.wrapT === emissive.wrapT &&
    map.magFilter === emissive.magFilter &&
    map.minFilter === emissive.minFilter &&
    map.anisotropy === emissive.anisotropy &&
    map.flipY === emissive.flipY &&
    map.colorSpace === emissive.colorSpace &&
    map.premultiplyAlpha === emissive.premultiplyAlpha;
  if (
    !hasQualifiedSourceDisplayShader(material) ||
    !sameTexture ||
    !material.emissive.equals(new THREE.Color(1, 1, 1)) ||
    material.emissiveIntensity !== 1 ||
    !material.color.equals(new THREE.Color(1, 1, 1)) ||
    material.metalness !== 0 ||
    !material.vertexColors ||
    material.side !==
      (material.userData.foliage_card_sides === "paired-one-sided"
        ? THREE.FrontSide
        : THREE.DoubleSide) ||
    material.opacity !== 1 ||
    material.transparent ||
    material.alphaHash ||
    material.alphaToCoverage ||
    material.blending !== THREE.NormalBlending ||
    material.premultipliedAlpha ||
    material.alphaTest !== 0.5 ||
    material.alphaMap ||
    material.envMap ||
    material.lightMap ||
    material.aoMap ||
    material.normalMap ||
    material.bumpMap ||
    material.displacementMap ||
    material.metalnessMap ||
    material.roughnessMap ||
    material.wireframe ||
    material.flatShading ||
    material.clippingPlanes?.length ||
    (material.defines && Object.keys(material.defines).some((k) => k !== "STANDARD")) ||
    material.userData.foliage_physical_opacity !== true ||
    material.userData.opacity_semantics !== "physical-coverage" ||
    material.userData.source_ownership_semantics !== "separate-mask" ||
    material.userData.source_ownership_channel !== "vertex-color-r"
  )
    throw new Error("Physical native preview rejected unsupported source-emissive material");
}

export interface PhysicalEffectBinding {
  object: THREE.Object3D;
  extractionContext?: NativeExtractionContext;
  tick(): number;
  binding: MissionStateContract["targets"][number];
}

/** The viewport's Y-up coordinates projected into the untouched source-art frame. */
export function nativeEffectCamera(
  box: THREE.Box3,
  elevation: number,
  frame?: { x: number; y: number; width: number; height: number },
) {
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
  const x = frame?.x ?? Math.floor(projected.min.x) - 1,
    y = frame?.y ?? Math.floor(projected.min.y) - 1;
  const width = frame?.width ?? Math.ceil(projected.max.x) + 1 - x,
    height = frame?.height ?? Math.ceil(projected.max.y) + 1 - y;
  if (![x, y, width, height].every(Number.isSafeInteger) || width < 1 || height < 1)
    throw new Error("Invalid native receiver frame");
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
      const kind = row.binding.source_kind ?? "mission-target";
      const element = native.elements.find(
        (e) => e.source.kind === kind && e.source.index === row.binding.target_index,
      );
      if (
        !element ||
        element.source.sha256 !== row.binding.target_sha256 ||
        this.bound.has(element.id)
      )
        throw new Error("Physical effect source identity differs");
      if (row.binding.extraction_context && !row.extractionContext)
        throw new Error("Physical effect requires verified extraction context");
      this.bound.set(element.id, row);
    }
    if (!this.bound.size) throw new Error("No source-bound physical effects");
  }
  get cameraElevation() {
    return this.native.camera_elevation_deg;
  }
  sample(renderer: THREE.WebGLRenderer, snapshot: NativeLoopDrawSnapshot): NativeLoopDrawSnapshot {
    if (snapshot.mission !== this.native.mission)
      throw new Error("Retired physical effect preview");
    return {
      ...snapshot,
      draws: snapshot.draws.map((draw) => {
        const bound = this.bound.get(draw.element.id);
        if (!bound) return draw;
        if (bound.binding.extraction_context && !bound.extractionContext)
          throw new Error("Physical effect requires verified extraction context");
        const root = bound.object;
        const expected = this.native.elements.find((e) => e.id === draw.element.id)!;
        if (
          !Number.isSafeInteger(draw.sourceTick) ||
          draw.sourceTick! < 0 ||
          typeof draw.sourceActive !== "boolean"
        )
          throw new Error("Physical effect snapshot lacks authoritative source timing");
        const duration = expected.frames.reduce((sum, f) => sum + f.delay + 1, 0);
        let expectedTick = draw.sourceTick! % duration;
        if (!draw.sourceActive && expected.initial_frame) {
          const initial = expected.initial_frame;
          const index = expected.frames.findIndex(
            (f) =>
              f.path === initial.path &&
              f.sha256 === initial.sha256 &&
              JSON.stringify(f.offset) === JSON.stringify(initial.offset),
          );
          if (index < 0) throw new Error("Physical effect has no bound stationary frame");
          expectedTick = expected.frames.slice(0, index).reduce((sum, f) => sum + f.delay + 1, 0);
        }
        const sampledFrame = expected.frames[nativePresentationFrame(expected, expectedTick)];
        const shownFrame = draw.element.frames[draw.frame];
        if (
          draw.element.active &&
          (bound.tick() !== expectedTick ||
            sampledFrame?.sha256 !== shownFrame?.sha256 ||
            sampledFrame?.path !== shownFrame?.path)
        )
          throw new Error("Physical effect pose and native frame differ");
        if (expected.frames.some((f) => f.shadow_key))
          throw new Error("Physical effect destination keying requires a separate proof");
        if (JSON.stringify(expected.source) !== JSON.stringify(draw.element.source))
          throw new Error("Physical effect snapshot identity changed");
        if (!draw.element.active || !root.visible)
          return { ...draw, pixels: { width: 1, height: 1, data: new Uint8Array(4) } };
        root.updateWorldMatrix(true, true);
        root.traverseVisible((node) => {
          if (node instanceof THREE.Light)
            throw new Error("Physical native preview requires a light-free source effect");
          if (!(node instanceof THREE.Mesh)) return;
          if (node instanceof THREE.SkinnedMesh)
            throw new Error("Physical native preview requires unskinned source effects");
          for (const material of Array.isArray(node.material) ? node.material : [node.material])
            validateNativeEffectMaterial(material);
        });
        const context = bound.extractionContext
          ? prepareNativeExtractionContext(bound.extractionContext, root, bound.binding)
          : undefined;
        try {
          const clone = context?.owner ?? root.clone(true);
          clone.matrixAutoUpdate = false;
          clone.matrix.copy(root.matrixWorld);
          const scene = new THREE.Scene();
          scene.add(clone);
          scene.updateMatrixWorld(true);
          const depthScene = context ? new THREE.Scene() : undefined;
          if (depthScene && context) {
            depthScene.add(context.depth);
            depthScene.updateMatrixWorld(true);
          }
          const view = nativeEffectCamera(
            new THREE.Box3().setFromObject(clone),
            this.native.camera_elevation_deg,
          );
          const pixels = renderPhysicalEffect(
            renderer,
            scene,
            view.camera,
            view.width,
            view.height,
            false,
            depthScene,
          );
          return {
            ...draw,
            pixels,
            x: view.x - snapshot.origin[0],
            y: view.y - snapshot.origin[1],
          };
        } finally {
          context?.dispose();
        }
      }),
    };
  }
}

/** Render one physical root in the complete source frame for a native remainder.
 * The root may be hidden in the 3D view; only its own subtree is drawn, with its world placement.
 */
export function renderNativeRemainderLayer(
  renderer: THREE.WebGLRenderer,
  root: THREE.Object3D,
  frame: { origin: readonly [number, number]; width: number; height: number },
  elevation: number,
): NativePixels {
  root.updateWorldMatrix(true, true);
  const clone = root.clone(true);
  clone.visible = true;
  clone.matrixAutoUpdate = false;
  clone.matrix.copy(root.matrixWorld);
  let meshes = 0;
  clone.traverseVisible((node) => {
    if (node instanceof THREE.Light)
      throw new Error("Native remainder physical layer requires a light-free model");
    if (!(node instanceof THREE.Mesh)) return;
    if (node instanceof THREE.SkinnedMesh)
      throw new Error("Native remainder physical layer requires unskinned models");
    for (const material of Array.isArray(node.material) ? node.material : [node.material])
      validateNativeEffectMaterial(material);
    meshes++;
  });
  if (!meshes) throw new Error("Native remainder physical layer has no visible model");
  const scene = new THREE.Scene();
  scene.add(clone);
  scene.updateMatrixWorld(true);
  const { camera, width, height } = nativeEffectCamera(
    new THREE.Box3().setFromObject(clone),
    elevation,
    { x: frame.origin[0], y: frame.origin[1], width: frame.width, height: frame.height },
  );
  return renderPhysicalEffect(renderer, scene, camera, width, height);
}

export function renderPhysicalEffect(
  renderer: THREE.WebGLRenderer,
  scene: THREE.Scene,
  camera: THREE.Camera,
  width: number,
  height: number,
  preserveToneMapping = false,
  depthContext?: THREE.Scene,
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
  const shadowEnabled = renderer.shadowMap.enabled,
    shadowNeedsUpdate = renderer.shadowMap.needsUpdate;
  const output = new THREE.WebGLRenderTarget(width, height, {
    depthBuffer: true,
    colorSpace: THREE.SRGBColorSpace,
  });
  try {
    renderer.setRenderTarget(output);
    renderer.setViewport(0, 0, width, height);
    renderer.setScissorTest(false);
    renderer.autoClear = false;
    if (!preserveToneMapping) {
      renderer.toneMapping = THREE.NoToneMapping;
      renderer.shadowMap.enabled = false;
    }
    renderer.setClearColor(0, 0);
    renderer.clear(true, true, false);
    if (depthContext) renderer.render(depthContext, camera);
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
    renderer.shadowMap.enabled = shadowEnabled;
    renderer.shadowMap.needsUpdate = shadowNeedsUpdate;
    output.dispose();
  }
}

/** Isolate real static receivers without moving them or flattening their depth. */
export function withNativeReceiverVisibility<T>(
  scene: THREE.Scene,
  roots: readonly THREE.Object3D[],
  excluded: readonly THREE.Object3D[],
  action: () => T,
): T {
  const allowed = new Set(roots),
    excludedRoots = new Set(excluded),
    hidden: THREE.Object3D[] = [];
  const within = (node: THREE.Object3D, set: Set<THREE.Object3D>) => {
    for (let p: THREE.Object3D | null = node; p; p = p.parent) if (set.has(p)) return true;
    return false;
  };
  try {
    scene.traverse((node) => {
      if (!node.visible) return;
      const renderable =
        node instanceof THREE.Mesh ||
        node instanceof THREE.Sprite ||
        node instanceof THREE.Line ||
        node instanceof THREE.Points;
      if (
        renderable &&
        (!(node instanceof THREE.Mesh) || !within(node, allowed) || within(node, excludedRoots))
      ) {
        hidden.push(node);
        node.visible = false;
      }
    });
    return action();
  } finally {
    for (const node of hidden) node.visible = true;
  }
}

/** Refined geometry is the receiver; native order remains a separate dynamic pass. */
export function renderNativeStaticReceiver(
  renderer: THREE.WebGLRenderer,
  scene: THREE.Scene,
  roots: readonly THREE.Object3D[],
  excluded: readonly THREE.Object3D[],
  snapshot: NativeLoopDrawSnapshot,
  elevation: number,
): NativePixels {
  scene.updateMatrixWorld(true);
  const box = new THREE.Box3();
  for (const root of roots) box.expandByObject(root);
  const { camera, width, height } = nativeEffectCamera(box, elevation, {
    x: snapshot.origin[0],
    y: snapshot.origin[1],
    width: snapshot.background.width,
    height: snapshot.background.height,
  });
  // Source effects and actors must not cast a second shadow into this receiver.
  const needsUpdate = renderer.shadowMap.needsUpdate;
  try {
    renderer.shadowMap.needsUpdate = true;
    return withNativeReceiverVisibility(scene, roots, excluded, () =>
      renderPhysicalEffect(renderer, scene, camera, width, height, true),
    );
  } finally {
    // An ordinary scene render must rebuild any shadow cache changed by isolation.
    renderer.shadowMap.needsUpdate = needsUpdate || renderer.shadowMap.enabled;
  }
}
