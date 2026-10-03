import * as THREE from "three";
import { sceneToMap, remapPatchExtras, type Level3D, type MapCamera } from "@rle/shared";
import type { GameplayAssetDescriptor } from "../../shared/src/asset-gameplay.ts";
import { compileAppearanceBindings } from "../../shared/src/compile-appearance-bindings.ts";
import type { BakeBounds, BakePixels } from "./map-compile.ts";
import type { BakedAppearanceRegion } from "./map-appearance.ts";
import { PatchDisplay } from "./patch-display.ts";
import { sunShadowBoundsPadding } from "./sun-lighting.ts";

function patchIds(node: THREE.Object3D): string[] {
  const {
    reveal_material_patch: material,
    reveal_hide_when_applied: hide,
    reveal_show_when_applied: show,
  } = node.userData;
  const values: unknown[] = material === undefined ? [] : [material];
  for (const references of [hide, show]) {
    if (references === undefined) continue;
    if (!Array.isArray(references)) throw new Error("Invalid appearance binding in bake geometry");
    values.push(...references);
  }
  const result: string[] = [];
  for (const value of values) {
    if (typeof value !== "string" || !value)
      throw new Error("Invalid appearance binding in bake geometry");
    result.push(value);
  }
  return result;
}

/** Resolve model-local IDs through the compiled placement and contact bindings. */
export function bindBakeAppearances(
  root: THREE.Object3D,
  document: Level3D,
  assets: ReadonlyMap<string, GameplayAssetDescriptor>,
  transitions: readonly { id: string; aliases?: string[] }[],
  warn?: (message: string) => void,
) {
  const bindings = compileAppearanceBindings(document, assets, transitions, warn);
  const compiled = new Set(transitions.map((transition) => transition.id));
  root.traverse((wrapper) => {
    const part = wrapper.userData.map_bake_object_id;
    const mapping = bindings.get(part);
    if (!mapping) return;
    for (const id of Object.values(mapping))
      if (!compiled.has(id)) throw new Error(`Appearance has no compiled transition: ${id}`);
    wrapper.traverse((node) => {
      let hiddenInitially = false;
      for (const id of patchIds(node))
        if (!Object.hasOwn(mapping, id)) {
          const message = `Missing asset gameplay binding for appearance ${id} on ${part}`;
          if (!warn) throw new Error(message);
          warn(`${message}; exported in its initial visual state.`);
          if (node.userData.reveal_material_patch === id) {
            hiddenInitially ||= node.userData.reveal_material_state === "revealed";
            delete node.userData.reveal_material_patch;
            delete node.userData.reveal_material_state;
          }
          for (const key of ["reveal_hide_when_applied", "reveal_show_when_applied"])
            if (Array.isArray(node.userData[key])) {
              node.userData[key] = node.userData[key].filter((value: unknown) => value !== id);
              if (!node.userData[key].length) {
                hiddenInitially ||= key === "reveal_show_when_applied";
                delete node.userData[key];
              }
            }
        }
      if (hiddenInitially) {
        node.visible = false;
        for (const key of [
          "reveal_material_patch",
          "reveal_material_state",
          "reveal_hide_when_applied",
          "reveal_show_when_applied",
        ])
          delete node.userData[key];
      }
      node.userData = remapPatchExtras(node.userData, mapping);
    });
  });
}

interface AppearancePlan {
  bounds: BakeBounds;
  patches: string[];
}

/** Include hidden applied variants and their shadows down to the lowest scene geometry. */
export function planAppearanceRegions(
  root: THREE.Object3D,
  camera: MapCamera,
  bounds: BakeBounds,
  transitions: readonly { id: string }[],
  shadows: boolean | NonNullable<Level3D["lighting"]>,
): AppearancePlan[] {
  root.updateMatrixWorld(true);
  const lighting = typeof shadows === "boolean" ? undefined : shadows;
  const fullShadowFrame = shadows === true;
  const sceneBounds = new THREE.Box3().setFromObject(root);
  const shadowDirection = lighting?.enabled
    ? new THREE.Vector3(
        Math.sin(THREE.MathUtils.degToRad(lighting.sunAzimuth)),
        Math.cos(THREE.MathUtils.degToRad(lighting.sunAzimuth)),
        Math.tan(THREE.MathUtils.degToRad(lighting.sunElevation)),
      )
    : undefined;
  if (shadowDirection && (!Number.isFinite(shadowDirection.z) || shadowDirection.z <= 0))
    throw new Error("Appearance shadow bounds require sunlight above the horizon");
  const shadowPadding = lighting?.enabled
    ? sunShadowBoundsPadding(sceneBounds, lighting.sunElevation)
    : 0;
  const compiled = new Set(transitions.map((transition) => transition.id));
  const boxes = new Map<string, THREE.Box2>();
  function visit(node: THREE.Object3D, inherited: readonly string[]) {
    const local = patchIds(node);
    if (!node.visible && !local.length) return;
    const patches = [...new Set([...inherited, ...local])];
    for (const id of patches)
      if (!compiled.has(id)) throw new Error(`Unbound bake appearance ${id}`);
    if (node instanceof THREE.Mesh && patches.length) {
      if (
        node instanceof THREE.InstancedMesh ||
        node instanceof THREE.SkinnedMesh ||
        node.geometry.morphAttributes.position?.length
      )
        throw new Error("Appearance bounds require static, non-instanced asset meshes");
      const positions = node.geometry.getAttribute("position");
      const box = new THREE.Box2();
      for (let i = 0; i < positions.count; i++) {
        const point = new THREE.Vector3()
          .fromBufferAttribute(positions, i)
          .applyMatrix4(node.matrixWorld);
        box.expandByPoint(new THREE.Vector2(...sceneToMap(camera, point.toArray())));
        if (shadowDirection && !node.userData.noSunShadow) {
          const shadow = point
            .clone()
            .addScaledVector(shadowDirection, -(point.z - sceneBounds.min.z) / shadowDirection.z);
          box.expandByPoint(new THREE.Vector2(...sceneToMap(camera, shadow.toArray())));
        }
      }
      if (shadowDirection && !node.userData.noSunShadow) box.expandByScalar(shadowPadding);
      for (const id of patches) {
        const previous = boxes.get(id) ?? new THREE.Box2();
        previous.union(box);
        boxes.set(id, previous);
      }
    }
    for (const child of node.children) visit(child, patches);
  }
  visit(root, []);
  const [originX, originY, width, height] = bounds;
  const regions: { box: THREE.Box2; patches: string[] }[] = [];
  for (const [patch, source] of boxes) {
    if (source.isEmpty()) continue;
    const box = fullShadowFrame
      ? new THREE.Box2(new THREE.Vector2(0, 0), new THREE.Vector2(width, height))
      : new THREE.Box2(
          new THREE.Vector2(
            Math.max(0, Math.floor(source.min.x - originX) - 2),
            Math.max(0, Math.floor(source.min.y - originY) - 2),
          ),
          new THREE.Vector2(
            Math.min(width, Math.ceil(source.max.x - originX) + 2),
            Math.min(height, Math.ceil(source.max.y - originY) + 2),
          ),
        );
    if (box.isEmpty() || box.min.x === box.max.x || box.min.y === box.max.y) continue;
    const patches = [patch];
    // Merging bounding rectangles can create a new intersection, so revisit earlier regions.
    for (let i = 0; i < regions.length;) {
      const other = regions[i]!;
      if (!box.intersectsBox(other.box)) {
        i++;
        continue;
      }
      box.union(other.box);
      patches.push(...other.patches);
      regions.splice(i, 1);
      i = 0;
    }
    regions.push({ box, patches });
  }
  let statePixels = 0;
  return regions.map(({ box, patches }) => {
    if (patches.length > 16) throw new Error("Too many overlapping appearance switches to bake");
    const w = box.max.x - box.min.x,
      h = box.max.y - box.min.y;
    statePixels += w * h * 2 ** patches.length;
    if (statePixels > 64 * 1024 * 1024)
      throw new Error(
        "Appearance combinations exceed 64 megapixels; reduce the export frame or overlapping state geometry",
      );
    return { bounds: [box.min.x, box.min.y, w, h], patches: patches.sort() };
  });
}

function crop(pixels: BakePixels, width: number, bounds: BakeBounds): BakePixels {
  const [x, y, w, h] = bounds;
  const color = new Uint8Array(w * h * 4),
    depth = new Uint16Array(w * h);
  for (let row = 0; row < h; row++) {
    const start = (y + row) * width + x;
    color.set(pixels.color.subarray(start * 4, (start + w) * 4), row * w * 4);
    depth.set(pixels.depth.subarray(start, start + w), row * w);
  }
  return { color, depth };
}

export function bakeAppearanceRegions(
  root: THREE.Object3D,
  plans: readonly AppearancePlan[],
  width: number,
  initial: BakePixels,
  render: () => BakePixels,
): BakedAppearanceRegion[] {
  const display = new PatchDisplay();
  try {
    return plans.map((plan) => {
      const states = [crop(initial, width, plan.bounds)];
      for (let state = 1; state < 2 ** plan.patches.length; state++) {
        display.clear();
        plan.patches.forEach((patch, bit) => display.set(patch, (state & (1 << bit)) !== 0));
        display.apply(root);
        // Use the full export frame to retain the depth field's normalization and lighting.
        states.push(crop(render(), width, plan.bounds));
      }
      return { ...plan, states };
    });
  } finally {
    display.clear();
    display.apply(root);
  }
}

/** Async state rendering preserves the same reset guarantee if rendering is interrupted. */
export async function bakeAppearanceRegionsAsync(
  root: THREE.Object3D,
  plans: readonly AppearancePlan[],
  width: number,
  initial: BakePixels,
  render: () => Promise<BakePixels>,
): Promise<BakedAppearanceRegion[]> {
  const display = new PatchDisplay();
  try {
    const regions: BakedAppearanceRegion[] = [];
    for (const plan of plans) {
      const states = [crop(initial, width, plan.bounds)];
      for (let state = 1; state < 2 ** plan.patches.length; state++) {
        display.clear();
        plan.patches.forEach((patch, bit) => display.set(patch, (state & (1 << bit)) !== 0));
        display.apply(root);
        states.push(crop(await render(), width, plan.bounds));
      }
      regions.push({ ...plan, states });
    }
    return regions;
  } finally {
    display.clear();
    display.apply(root);
  }
}
