import * as THREE from "three";
import { StateAppearancePlayer, type StateAppearanceTemplate } from "./state-appearance-player.ts";
import type {
  PhysicalElementComponentClips,
  ComponentClip,
  ComponentClipSelection,
} from "../../shared/src/component-clips.ts";

const fail = (why: string): never => {
  throw new Error(`Component clip: ${why}`);
};
function meshes(root: THREE.Object3D) {
  const out = new Map<string, THREE.Mesh>();
  root.updateMatrixWorld(true);
  root.traverse((o) => {
    if (!(o instanceof THREE.Mesh)) return;
    if ((o as THREE.SkinnedMesh).isSkinnedMesh || !o.name || out.has(o.name))
      fail("unsupported or ambiguous mesh");
    out.set(o.name, o);
  });
  return out;
}
/** A flattened plane is still drawable; only three zero axes retire a scaled node. */
export function componentMeshDrawable(mesh: THREE.Mesh, boundary?: THREE.Object3D) {
  const a = mesh.matrixWorld.elements;
  if (!a.every(Number.isFinite)) fail("nonfinite matrix");
  if (mesh.morphTargetInfluences?.some((v) => !Number.isFinite(v)))
    fail("nonfinite morph influence");
  for (let p: THREE.Object3D | null = mesh; p && p !== boundary?.parent; p = p.parent)
    if (!p.visible) return false;
  return [0, 4, 8].some((i) => Math.hypot(a[i]!, a[i + 1]!, a[i + 2]!) > 1e-10);
}
function imagePixels(image: unknown): { width: number; height: number; data: ArrayLike<number> } {
  if (!image || typeof image !== "object") return fail("missing texture image");
  const im = image as { width: number; height: number; data?: ArrayLike<number> };
  if (
    !Number.isSafeInteger(im.width) ||
    !Number.isSafeInteger(im.height) ||
    im.width < 1 ||
    im.height < 1
  )
    fail("invalid texture dimensions");
  if (im.data) return { width: im.width, height: im.height, data: im.data };
  if (typeof OffscreenCanvas === "undefined")
    return fail("texture pixels unavailable for endpoint verification");
  const canvas = new OffscreenCanvas(im.width, im.height),
    ctx = canvas.getContext("2d");
  if (!ctx) return fail("texture verification canvas unavailable");
  ctx.drawImage(image as CanvasImageSource, 0, 0);
  return ctx.getImageData(0, 0, im.width, im.height);
}
function sameTexture(a: THREE.Texture, b: THREE.Texture) {
  if (a === b) return;
  const state = (t: THREE.Texture) => [
    t.wrapS,
    t.wrapT,
    t.magFilter,
    t.minFilter,
    t.anisotropy,
    t.flipY,
    t.channel,
    t.colorSpace,
    t.format,
    t.type,
    t.offset.toArray(),
    t.repeat.toArray(),
    t.center.toArray(),
    t.rotation,
    t.matrix.toArray(),
    t.matrixAutoUpdate,
  ];
  if (JSON.stringify(state(a)) !== JSON.stringify(state(b)))
    fail("terminal texture sampling differs");
  const x = imagePixels(a.image),
    y = imagePixels(b.image);
  if (x.width !== y.width || x.height !== y.height || x.data.length !== y.data.length)
    fail("terminal texture dimensions differ");
  for (let i = 0; i < x.data.length; i++)
    if (x.data[i] !== y.data[i]) fail("terminal texture pixels differ");
}
function sameMaterial(a: THREE.Material, b: THREE.Material) {
  if (a === b) return;
  const data = (m: THREE.Material) => {
    const d = m.toJSON() as unknown as Record<string, unknown>;
    for (const k of ["uuid", "name", "metadata", "textures", "images", "userData"]) delete d[k];
    return d;
  };
  const x = data(a),
    y = data(b);
  for (const key of new Set([...Object.keys(a), ...Object.keys(b)])) {
    const u = (a as unknown as Record<string, unknown>)[key],
      v = (b as unknown as Record<string, unknown>)[key];
    if (u instanceof THREE.Texture || v instanceof THREE.Texture) {
      if (!(u instanceof THREE.Texture) || !(v instanceof THREE.Texture))
        fail("terminal texture slot differs");
      sameTexture(u as THREE.Texture, v as THREE.Texture);
      x[key] = y[key] = "verified-texture";
    }
  }
  if (JSON.stringify(x) !== JSON.stringify(y)) fail("terminal material differs");
}
/** Compare evaluated visible geometry; retired nodes cannot hide missing or extra endpoint meshes. */
export function assertComponentClipTerminal(
  applied: THREE.Object3D,
  root: THREE.Object3D,
  c: ComponentClip,
) {
  const actual = meshes(root),
    expected = meshes(applied);
  const owned = new Set(c.nodes),
    active = new Set(c.terminal.active_nodes),
    retired = new Set(c.terminal.retired_nodes);
  for (const n of owned) {
    const o = actual.get(n);
    if (!o) fail(`missing declared mesh ${n}`);
    const visible = componentMeshDrawable(o!, root);
    if (visible !== active.has(n) || (!active.has(n) && !retired.has(n)))
      fail(`terminal visibility differs: ${n}`);
  }
  for (const [n, o] of actual)
    if (componentMeshDrawable(o, root) && !active.has(n)) fail(`extraneous visible mesh ${n}`);
  const v = new THREE.Vector3(),
    w = new THREE.Vector3();
  for (const n of active) {
    const a = expected.get(n),
      b = actual.get(n);
    if (!a || !b || !componentMeshDrawable(a, applied)) fail(`missing active endpoint ${n}`);
    const p = a!.geometry.getAttribute("position"),
      q = b!.geometry.getAttribute("position");
    if (!p || !q || p.count !== q.count || p.itemSize !== q.itemSize)
      fail(`terminal topology differs: ${n}`);
    const ai = a!.geometry.index,
      bi = b!.geometry.index;
    if (!!ai !== !!bi || ai?.count !== bi?.count) fail(`terminal indices differ: ${n}`);
    if (ai && bi)
      for (let i = 0; i < ai.count; i++)
        if (ai.getX(i) !== bi.getX(i)) fail(`terminal indices differ: ${n}`);
    if (JSON.stringify(a!.geometry.groups) !== JSON.stringify(b!.geometry.groups))
      fail(`terminal material groups differ: ${n}`);
    if (JSON.stringify(a!.geometry.drawRange) !== JSON.stringify(b!.geometry.drawRange))
      fail(`terminal draw range differs: ${n}`);
    for (let i = 0; i < p.count; i++) {
      a!.getVertexPosition(i, v).applyMatrix4(a!.matrixWorld);
      b!.getVertexPosition(i, w).applyMatrix4(b!.matrixWorld);
      if (![...v, ...w].every(Number.isFinite) || v.distanceTo(w) > 1e-5 * Math.max(1, v.length()))
        fail(`evaluated terminal geometry differs: ${n}`);
    }
    const normal = (mesh: THREE.Mesh, index: number, out: THREE.Vector3) => {
      const attr = mesh.geometry.getAttribute("normal");
      out.fromBufferAttribute(attr, index);
      const base = out.clone();
      for (const [j, target] of (mesh.geometry.morphAttributes.normal ?? []).entries()) {
        const weight = mesh.morphTargetInfluences?.[j] ?? 0;
        const delta = new THREE.Vector3().fromBufferAttribute(target, index);
        if (!mesh.geometry.morphTargetsRelative) delta.sub(base);
        out.addScaledVector(delta, weight);
      }
      return out.applyNormalMatrix(new THREE.Matrix3().getNormalMatrix(mesh.matrixWorld));
    };
    const an = a!.geometry.getAttribute("normal"),
      bn = b!.geometry.getAttribute("normal");
    if (!!an !== !!bn || an?.count !== bn?.count) fail(`terminal normals differ: ${n}`);
    if (an && bn)
      for (let i = 0; i < an.count; i++) {
        normal(a!, i, v);
        normal(b!, i, w);
        if (![...v, ...w].every(Number.isFinite) || v.distanceTo(w) > 1e-5)
          fail(`evaluated terminal normals differ: ${n}`);
      }
    for (const attr of ["uv", "uv1", "color", "tangent"]) {
      const x = a!.geometry.getAttribute(attr),
        y = b!.geometry.getAttribute(attr);
      if (!!x !== !!y || x?.count !== y?.count || x?.itemSize !== y?.itemSize)
        fail(`terminal ${attr} differs: ${n}`);
      if (x && y)
        for (let i = 0; i < x.array.length; i++)
          if (x.array[i] !== y.array[i]) fail(`terminal ${attr} differs: ${n}`);
    }
    const am = Array.isArray(a!.material) ? a!.material : [a!.material],
      bm = Array.isArray(b!.material) ? b!.material : [b!.material];
    if (am.length !== bm.length) fail("terminal material slots differ");
    am.forEach((m, i) => sameMaterial(m, bm[i]!));
  }
}

type Rig = {
  component: ComponentClip;
  player: StateAppearancePlayer;
  objects: Map<string, THREE.Mesh>;
};
/** Independent source owners share immutable resources, never animation cursors. */
export class ComponentClips {
  readonly roots = new Map<string, THREE.Group>();
  private rigs = new Map<string, Map<string, Rig>>();
  constructor(
    applied: THREE.Group,
    motion: PhysicalElementComponentClips,
    templates: readonly StateAppearanceTemplate[],
  ) {
    const endpoint = meshes(applied),
      active = motion.components.flatMap((c) => c.terminal.active_nodes);
    if (
      new Set(active).size !== active.length ||
      active.length !== endpoint.size ||
      active.some((n) => !endpoint.has(n))
    )
      fail("applied endpoint coverage differs");
    // Applied groups are hidden during loading; their visibility is not endpoint geometry.
    const shown = applied.visible;
    applied.visible = true;
    try {
      motion.components.forEach((c, index) => {
        const template = templates[index];
        if (!template) fail("missing pinned template");
        const t = template!;
        if (t.clips.length !== 1 || t.clips[0]!.name !== c.clip)
          fail("pinned clip identity differs");
        const expected = new Set(
          motion.components
            .filter((other) => JSON.stringify(other.binding) === JSON.stringify(c.binding))
            .flatMap((other) => other.nodes),
        );
        const source = meshes(t.sourceRoot);
        for (const mesh of source.values()) {
          for (const attr of [
            mesh.geometry.getAttribute("position"),
            ...Object.values(mesh.geometry.attributes),
            ...Object.values(mesh.geometry.morphAttributes).flat(),
          ]) {
            if (!attr || attr.array.some((v) => !Number.isFinite(v)))
              fail("nonfinite source geometry");
          }
        }
        if (source.size !== expected.size || [...source.keys()].some((n) => !expected.has(n)))
          fail("unowned clip mesh");
        if (
          Math.max(...t.clips[0]!.tracks.map((x) => x.track.times.at(-1)!)) < c.terminal.clip_tick
        )
          fail("clip ends before terminal tick");
        for (const stage of c.stages) {
          let group = this.roots.get(stage.element_id);
          if (!group) {
            group = new THREE.Group();
            this.roots.set(stage.element_id, group);
            this.rigs.set(stage.element_id, new Map());
          }
          const player = new StateAppearancePlayer(t, true);
          player.select(c.clip, { mode: "clamp", terminalTick: c.terminal.clip_tick });
          if (c.binding.position)
            player.object.position.add(new THREE.Vector3(...c.binding.position));
          const objects = meshes(player.object),
            rig = { component: c, player, objects };
          group.add(player.object);
          this.rigs.get(stage.element_id)!.set(c.id, rig);
          this.sample(rig, c.terminal.clip_tick);
          assertComponentClipTerminal(applied, player.object, c);
          // Validate each declared visibility sample before exposing the contract.
          for (const tick of new Set(
            c.stages.flatMap((s) => s.frame_ticks.filter((v) => v !== null)),
          ))
            this.sample(rig, tick);
        }
      });
    } catch (e) {
      this.dispose();
      throw e;
    } finally {
      applied.visible = shown;
    }
    this.reset();
  }
  private filter(rig: Rig) {
    for (const [n, o] of rig.objects) if (!rig.component.nodes.includes(n)) o.visible = false;
  }
  private sample(rig: Rig, tick: number) {
    rig.player.object.visible = true;
    rig.player.sampleExternalTick(tick);
    this.filter(rig);
    rig.player.object.updateWorldMatrix(true, true);
    const phase = rig.component.visibility_phases.find(
      (p) => p.first_tick <= tick && tick <= p.last_tick,
    );
    if (!phase) fail("missing visibility sample");
    for (const n of rig.component.nodes) {
      const o = rig.objects.get(n)!;
      o.visible = phase!.active_nodes.includes(n);
      if (componentMeshDrawable(o, rig.player.object) !== phase!.active_nodes.includes(n))
        fail(`visibility schedule differs: ${n} at ${tick}`);
      if (o.morphTargetInfluences?.some((v) => !Number.isFinite(v)))
        fail("nonfinite morph influence");
    }
  }
  apply(selections: readonly ComponentClipSelection[]) {
    const seen = new Set<string>();
    for (const s of selections) {
      const r = this.rigs.get(s.element_id)?.get(s.component_id);
      const stage = r?.component.stages.find((t) => t.element_id === s.element_id);
      if (
        !r ||
        seen.has(s.component_id) ||
        !Number.isSafeInteger(s.clip_tick) ||
        !stage?.frame_ticks.some(
          (start, frame, ticks) =>
            start !== null &&
            s.clip_tick >= start &&
            s.clip_tick <= r.component.terminal.clip_tick &&
            (frame === ticks.length - 1
              ? s.clip_tick === start
              : s.clip_tick < start + stage.source_frames[frame]!.delay + 1),
        )
      )
        fail("invalid or overlapping selection");
      seen.add(s.component_id);
    }
    this.reset();
    for (const s of selections) {
      const rig = this.rigs.get(s.element_id)!.get(s.component_id)!;
      this.roots.get(s.element_id)!.visible = true;
      this.sample(rig, s.clip_tick);
    }
  }
  reset() {
    for (const [id, rigs] of this.rigs) {
      const group = this.roots.get(id)!;
      group.visible = true;
      for (const rig of rigs.values()) {
        rig.player.sampleExternalTick(0);
        rig.player.object.visible = false;
      }
      group.visible = false;
    }
  }
  dispose() {
    for (const rigs of this.rigs.values()) for (const rig of rigs.values()) rig.player.dispose();
    this.rigs.clear();
    this.roots.clear();
  }
}
