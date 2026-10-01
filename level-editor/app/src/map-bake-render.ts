import * as THREE from "three";
import { sceneToMap, type MapCamera } from "@rle/shared";
import { stableOpaqueSort } from "./render-order.ts";
import { TextureDisplay } from "./texture-display.ts";
import { SunLighting } from "./sun-lighting.ts";
import { PatchDisplay } from "./patch-display.ts";
import { validateBakeBounds, type BakeBounds, type BakePixels } from "./map-compile.ts";
import type { GameplayAssetDescriptor } from "../../shared/src/asset-gameplay.ts";

export interface BakeProgress {
  stage: string;
  completed: number;
  total: number;
}

/** Let progress paint and pending input run before borrowing the GPU again. */
export const yieldBakeFrame = () =>
  new Promise<void>((resolve) => requestAnimationFrame(() => setTimeout(resolve, 0)));

export function maskOcclusionObjects(
  document: import("@rle/shared").Level3D,
  assets?: ReadonlyMap<string, GameplayAssetDescriptor>,
): Set<string> {
  return new Set(
    document.objects
      .filter((part) => {
        const match = /^asset:([^:]+):(.+)$/.exec(part.node);
        return match && assets?.get(match[1]!)?.gameplay?.maskOcclusionNodes?.includes(match[2]!);
      })
      .map((part) => part.id),
  );
}

/** Omit only explicitly mask-owned parts; underlying meshes still write depth. */
export function withDepthOcclusion<T>(
  root: THREE.Object3D,
  excluded: ReadonlySet<string>,
  render: () => T,
): T {
  const restore = hideDepthOcclusion(root, excluded);
  try {
    return render();
  } finally {
    restore();
  }
}

function hideDepthOcclusion(root: THREE.Object3D, excluded: ReadonlySet<string>) {
  const hidden: THREE.Object3D[] = [];
  root.traverse((node) => {
    if (node.visible && excluded.has(node.userData.map_bake_object_id)) {
      hidden.push(node);
      node.visible = false;
    }
  });
  return () => {
    for (const node of hidden) node.visible = true;
  };
}

/** Sources are in the editor's Z-up map frame. Copy the full hierarchy so applied
 * states can reveal hidden meshes. Geometry/textures are borrowed during rendering. */
export function bakeScene(
  roots: THREE.Object3D[],
  appliedPatches: ReadonlySet<string> = new Set(),
): THREE.Group {
  const scene = new THREE.Group();
  const display = new PatchDisplay();
  for (const patch of appliedPatches) display.set(patch, true);
  for (const source of roots) {
    const clone = source.clone(true);
    // Export state is explicit and independent of viewport preview switches.
    display.apply(clone);
    scene.add(clone);
  }
  return scene;
}

export function contentBakeBounds(
  root: THREE.Object3D,
  camera: MapCamera,
  includeAppearanceStates = false,
): BakeBounds {
  root.updateMatrixWorld(true);
  const bounds = new THREE.Box2();
  function visit(node: THREE.Object3D) {
    const controlled =
      includeAppearanceStates &&
      ["reveal_material_patch", "reveal_hide_when_applied", "reveal_show_when_applied"].some(
        (key) => node.userData[key] !== undefined,
      );
    if (!node.visible && !controlled) return;
    if (node instanceof THREE.Mesh) {
      const positions = node.geometry.getAttribute("position");
      for (let i = 0; i < positions.count; i++) {
        const point = new THREE.Vector3()
          .fromBufferAttribute(positions, i)
          .applyMatrix4(node.matrixWorld);
        bounds.expandByPoint(new THREE.Vector2(...sceneToMap(camera, point.toArray())));
      }
    }
    for (const child of node.children) visit(child);
  }
  visit(root);
  if (bounds.isEmpty()) throw new Error("Add visible geometry before exporting a map.");
  return validateBakeBounds([
    bounds.min.x,
    bounds.min.y,
    bounds.max.x - bounds.min.x,
    bounds.max.y - bounds.min.y,
  ]);
}

function depthMaterial(source: THREE.Material, camera: MapCamera, bounds: BakeBounds) {
  if (!(source instanceof THREE.MeshBasicMaterial || source instanceof THREE.MeshLambertMaterial))
    throw new Error(
      `Cannot compile depth for material ${source.name || source.type}. Expected a map or terrain material.`,
    );
  const material = source.clone();
  if (source.userData.terrainMaterialBlend) {
    material.onBeforeCompile = (shader, renderer) => source.onBeforeCompile(shader, renderer);
    material.customProgramCacheKey = () => source.customProgramCacheKey();
  }
  // Preserve physical alpha coverage, but never blend encoded depth bytes.
  material.transparent = false;
  material.blending = THREE.NoBlending;
  material.depthWrite = true;
  material.alphaTest = Math.max(source.alphaTest, 0.5);
  material.toneMapped = false;
  const display = new TextureDisplay();
  display.material(material);
  const prepare = material.onBeforeCompile.bind(material);
  material.onBeforeCompile = (shader, renderer) => {
    prepare(shader, renderer);
    shader.uniforms.bakeSin = { value: Math.sin((camera.elevation_deg * Math.PI) / 180) };
    shader.uniforms.bakeOriginY = { value: bounds[1] };
    shader.uniforms.bakeHeight = { value: bounds[3] };
    shader.vertexShader =
      "varying float bakeGroundY;\nuniform float bakeSin;\n" + shader.vertexShader;
    shader.vertexShader = shader.vertexShader.replace(
      "#include <project_vertex>",
      "#include <project_vertex>\nbakeGroundY = (modelMatrix * vec4(transformed, 1.0)).z * bakeSin;",
    );
    shader.fragmentShader =
      "varying float bakeGroundY;\nuniform float bakeOriginY;\nuniform float bakeHeight;\n" +
      shader.fragmentShader;
    const end = shader.fragmentShader.lastIndexOf("}");
    shader.fragmentShader =
      shader.fragmentShader.slice(0, end) +
      `
      float encoded = floor(clamp((bakeGroundY - bakeOriginY) / bakeHeight, 0.0, 1.0) * 65535.0 + 0.5);
      gl_FragColor = vec4(floor(encoded / 256.0) / 255.0, mod(encoded, 256.0) / 255.0, 0.0, 1.0);
    }`;
  };
  material.customProgramCacheKey = () =>
    "map-bake-ground-depth-v1:" + JSON.stringify(source.userData);
  return material;
}

/** Both drivers share the same tile pipeline and guaranteed GPU resource cleanup. */
function* mapBakeTiles(
  root: THREE.Group,
  cameraModel: MapCamera,
  bounds: BakeBounds,
  lighting?: import("@rle/shared").Level3D["lighting"],
  ground?: THREE.Object3D | null,
  depthExcludedObjects: ReadonlySet<string> = new Set(),
  tileLimit = 1024,
): Generator<BakeProgress, BakePixels> {
  const [, , width, height] = validateBakeBounds(bounds);
  const renderer = new THREE.WebGLRenderer({ antialias: false, alpha: false });
  renderer.setPixelRatio(1);
  renderer.setOpaqueSort(stableOpaqueSort);
  renderer.outputColorSpace = THREE.SRGBColorSpace;
  renderer.debug.onShaderError = (gl, program) => {
    throw new Error(`Map bake shader failed: ${gl.getProgramInfoLog(program)}`);
  };
  const scene = new THREE.Scene();
  scene.background = new THREE.Color(0);
  const frame = new THREE.Group();
  frame.quaternion.set(-Math.SQRT1_2, 0, 0, Math.SQRT1_2);
  const parent = root.parent;
  const siblingIndex = parent?.children.indexOf(root);
  frame.add(root);
  scene.add(frame);
  const sunlight = new SunLighting();
  frame.add(sunlight.root);
  const materials = new Set<THREE.Material>();
  const original = new Map<THREE.Mesh, THREE.Material | THREE.Material[]>();
  const colorMaterials = new Map<THREE.Material, THREE.Material>();
  const display = new TextureDisplay();
  try {
    root.traverseVisible((node) => {
      if (!(node instanceof THREE.Mesh)) return;
      const copy = (source: THREE.Material) => {
        const cached = colorMaterials.get(source);
        if (cached) return cached;
        const material = source.clone();
        if (source.userData.terrainMaterialBlend) {
          material.onBeforeCompile = (shader, renderer) => source.onBeforeCompile(shader, renderer);
          material.customProgramCacheKey = () => source.customProgramCacheKey();
        }
        display.material(material);
        materials.add(material);
        colorMaterials.set(source, material);
        return material;
      };
      original.set(node, node.material);
      node.material = Array.isArray(node.material) ? node.material.map(copy) : copy(node.material);
    });
    scene.updateMatrixWorld(true);
    const box = new THREE.Box3().setFromObject(root);
    if (box.isEmpty()) throw new Error("The map has no renderable geometry.");
    sunlight.setGround(ground ?? null);
    sunlight.sync(lighting, [root], box);
    renderer.shadowMap.enabled = lighting?.enabled ?? true;
    renderer.shadowMap.type = THREE.PCFSoftShadowMap;
    // Every tile sees the same scene and full-map light camera; rebuild once per appearance.
    renderer.shadowMap.autoUpdate = false;
    renderer.shadowMap.needsUpdate = true;
    const angle = (cameraModel.elevation_deg * Math.PI) / 180;
    const center = new THREE.Vector3(
      bounds[0] + width / 2,
      0,
      (bounds[1] + height / 2) / Math.sin(angle),
    );
    const distance =
      box.getSize(new THREE.Vector3()).length() +
      center.distanceTo(box.getCenter(new THREE.Vector3())) +
      1000;
    const camera = new THREE.OrthographicCamera(
      -width / 2,
      width / 2,
      height / 2,
      -height / 2,
      0.1,
      distance * 3,
    );
    camera.position
      .copy(center)
      .add(new THREE.Vector3(0, Math.sin(angle), Math.cos(angle)).multiplyScalar(distance));
    camera.lookAt(center);
    camera.updateMatrixWorld(true);
    const tile = Math.min(tileLimit, renderer.capabilities.maxTextureSize);
    const color = new Uint8Array(width * height * 4),
      depth = new Uint16Array(width * height);
    const total = Math.ceil(width / tile) * Math.ceil(height / tile) * 2;
    let completed = 0;
    const renderPass = function* (isDepth: boolean): Generator<BakeProgress> {
      for (let y = 0; y < height; y += tile)
        for (let x = 0; x < width; x += tile) {
          const w = Math.min(tile, width - x),
            h = Math.min(tile, height - y);
          const target = new THREE.WebGLRenderTarget(w, h, {
            depthBuffer: true,
            colorSpace: isDepth ? THREE.NoColorSpace : THREE.SRGBColorSpace,
          });
          try {
            camera.setViewOffset(width, height, x, y, w, h);
            renderer.setRenderTarget(target);
            renderer.render(scene, camera);
            const bytes = new Uint8Array(w * h * 4);
            renderer.readRenderTargetPixels(target, 0, 0, w, h, bytes);
            if (renderer.getContext().isContextLost())
              throw new Error(
                "The GPU context was lost while compiling. Try a smaller export frame.",
              );
            for (let row = 0; row < h; row++) {
              const source = (h - row - 1) * w * 4,
                dest = (y + row) * width + x;
              if (!isDepth) color.set(bytes.subarray(source, source + w * 4), dest * 4);
              else
                for (let column = 0; column < w; column++)
                  depth[dest + column] =
                    bytes[source + column * 4]! * 256 + bytes[source + column * 4 + 1]!;
            }
          } finally {
            target.dispose();
          }
          yield {
            stage: isDepth ? "Rendering depth" : "Rendering color",
            completed: ++completed,
            total,
          };
        }
    };
    yield* renderPass(false);
    sunlight.root.visible = false;
    renderer.shadowMap.enabled = false;
    const depthMaterials = new Map<THREE.Material, THREE.Material>();
    for (const node of original.keys()) {
      const source = node.material;
      const convert = (material: THREE.Material) => {
        const cached = depthMaterials.get(material);
        if (cached) return cached;
        const depth = depthMaterial(material, cameraModel, bounds);
        materials.add(depth);
        depthMaterials.set(material, depth);
        return depth;
      };
      node.material = Array.isArray(source) ? source.map(convert) : convert(source);
    }
    const restoreVisibility = hideDepthOcclusion(root, depthExcludedObjects);
    try {
      yield* renderPass(true);
    } finally {
      restoreVisibility();
    }
    return { color, depth };
  } finally {
    sunlight.dispose();
    for (const [node, material] of original) node.material = material;
    for (const material of materials) material.dispose();
    renderer.dispose();
    renderer.forceContextLoss();
    root.removeFromParent();
    if (parent && siblingIndex !== undefined) {
      parent.add(root);
      parent.children.splice(parent.children.indexOf(root), 1);
      parent.children.splice(siblingIndex, 0, root);
    }
  }
}

/** Synchronous driver for callers that already own the uninterrupted render lifetime. */
export function renderMapBake(...args: Parameters<typeof mapBakeTiles>): BakePixels {
  const tiles = mapBakeTiles(...args);
  let result = tiles.next();
  while (!result.done) result = tiles.next();
  return result.value;
}

/** Render a bounded tile at a time so progress updates and cancellation can run. */
export async function renderMapBakeAsync(
  root: THREE.Group,
  cameraModel: MapCamera,
  bounds: BakeBounds,
  lighting?: import("@rle/shared").Level3D["lighting"],
  ground?: THREE.Object3D | null,
  depthExcludedObjects: ReadonlySet<string> = new Set(),
  progress: (progress: BakeProgress) => void = () => {},
  checkCurrent: () => void = () => {},
): Promise<BakePixels> {
  const tiles = mapBakeTiles(
    root,
    cameraModel,
    bounds,
    lighting,
    ground,
    depthExcludedObjects,
    512,
  );
  try {
    while (true) {
      checkCurrent();
      const result = tiles.next();
      if (result.done) return result.value;
      progress(result.value);
      await yieldBakeFrame();
    }
  } finally {
    // Closing a suspended generator restores materials, visibility, parents, and GPU resources.
    tiles.return(undefined as never);
  }
}
