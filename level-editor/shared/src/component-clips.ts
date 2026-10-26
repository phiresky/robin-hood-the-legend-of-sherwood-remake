import {
  nativePresentationFrame,
  type NativePresentationFrame,
} from "./native-state-presentation.ts";
import type {
  StateDeliveryContract,
  PhysicalStateBinding,
  PhysicalStageClock,
  NativeRemainderFrame,
} from "./state-delivery.ts";

type Family = StateDeliveryContract["families"][number];
export interface PhysicalElementComponentClips {
  kind: "native-element-component-clips";
  source_motion_sha256: string;
  native_passthrough_element_ids: string[];
  components: {
    id: string;
    binding: PhysicalStateBinding;
    clip: string;
    nodes: string[];
    stages: {
      element_id: string;
      source_element_sha256: string;
      /** Exact source resources, offsets and delays; a digest alone cannot verify timing. */
      source_frames: NativePresentationFrame[];
      first_source_frame: number;
      clip_start_tick: number;
      frame_ticks: (number | null)[];
      /** Explicit source-bound component absence, not an inferred transparent whole sprite. */
      gaps: { frame: number; source_sha256: string; reason: "component-absent" }[];
    }[];
    visibility_phases: { first_tick: number; last_tick: number; active_nodes: string[] }[];
    terminal: {
      stage_element_id: string;
      source_frame: number;
      clip_tick: number;
      active_nodes: string[];
      retired_nodes: string[];
    };
  }[];
}
export type ComponentClip = PhysicalElementComponentClips["components"][number];
export type ComponentClipSelection = {
  component_id: string;
  element_id: string;
  clip_tick: number;
};
const fail = (why: string): never => {
  throw new Error(`State delivery: component clips ${why}`);
};
const keys = (x: object, expected: string[]) =>
  !!x &&
  typeof x === "object" &&
  JSON.stringify(Object.keys(x).sort()) === JSON.stringify(expected.sort());
const names = (a: unknown, empty = false): a is string[] =>
  Array.isArray(a) &&
  (empty || !!a.length) &&
  a.every((x) => typeof x === "string" && !!x) &&
  new Set(a).size === a.length;
const tick = (x: unknown): x is number =>
  Number.isSafeInteger(x) && (x as number) >= 0 && (x as number) < 2 ** 24;
const hash = (x: unknown) => typeof x === "string" && /^[a-f0-9]{64}$/.test(x);
const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);
export function validateComponentClips(
  contract: StateDeliveryContract,
  family: Family,
  motion: PhysicalElementComponentClips,
) {
  if (
    !keys(motion, [
      "kind",
      "source_motion_sha256",
      "native_passthrough_element_ids",
      "components",
    ]) ||
    !hash(motion.source_motion_sha256) ||
    !names(motion.native_passthrough_element_ids, true) ||
    !Array.isArray(motion.components) ||
    !motion.components.length ||
    family.patch_ids?.length ||
    family.hidden_initial_element_ids?.length ||
    !Array.isArray(family.physical.applied)
  )
    fail("invalid motion");
  const ids = new Set<string>(),
    nodes = new Set<string>(),
    owners = new Set<string>();
  for (const c of motion.components) {
    if (
      !keys(c, ["id", "binding", "clip", "nodes", "stages", "visibility_phases", "terminal"]) ||
      !names([c.id]) ||
      !names([c.clip]) ||
      ids.has(c.id) ||
      !names(c.nodes) ||
      !Array.isArray(c.stages) ||
      !c.stages.length
    )
      fail("invalid component");
    ids.add(c.id);
    for (const n of c.nodes) {
      if (nodes.has(n)) fail("duplicate node ownership");
      nodes.add(n);
    }
    const stageOwners = new Set<string>();
    for (const s of c.stages) {
      if (
        !keys(s, [
          "element_id",
          "source_element_sha256",
          "source_frames",
          "first_source_frame",
          "clip_start_tick",
          "frame_ticks",
          "gaps",
        ])
      )
        fail("invalid stage");
      const e = contract.native.elements.find((e) => e.id === s.element_id);
      if (
        !e ||
        e.loop ||
        !family.element_ids.includes(s.element_id) ||
        stageOwners.has(s.element_id) ||
        s.source_element_sha256 !== e.source.sha256 ||
        !same(s.source_frames, e.frames) ||
        !tick(s.first_source_frame) ||
        s.first_source_frame >= e.frames.length ||
        !tick(s.clip_start_tick) ||
        !Array.isArray(s.frame_ticks) ||
        s.frame_ticks.length !== e.frames.length ||
        !Array.isArray(s.gaps)
      )
        fail("source coverage or timing differs");
      stageOwners.add(s.element_id);
      owners.add(s.element_id);
      const gaps = new Set<number>();
      for (const g of s.gaps) {
        if (
          !keys(g, ["frame", "source_sha256", "reason"]) ||
          !tick(g.frame) ||
          g.frame >= e!.frames.length ||
          gaps.has(g.frame) ||
          g.reason !== "component-absent" ||
          g.source_sha256 !== e!.frames[g.frame]!.sha256
        )
          fail("invalid gap evidence");
        gaps.add(g.frame);
      }
      let at = s.clip_start_tick;
      for (let i = 0; i < e!.frames.length; i++) {
        const value = s.frame_ticks[i];
        if (value === null) {
          if (!gaps.has(i)) fail("unexplained gap");
        } else if (gaps.has(i) || i < s.first_source_frame || !tick(value) || value !== at)
          fail("source delay mapping differs");
        if (i >= s.first_source_frame) at += e!.frames[i]!.delay + 1;
      }
      if (
        s.frame_ticks[s.first_source_frame] !== s.clip_start_tick ||
        s.frame_ticks.slice(0, s.first_source_frame).some((x) => x !== null)
      )
        fail("unbound first frame");
    }
    const t = c.terminal,
      last = c.stages.at(-1)!;
    if (
      !keys(t, [
        "stage_element_id",
        "source_frame",
        "clip_tick",
        "active_nodes",
        "retired_nodes",
      ]) ||
      t.stage_element_id !== last.element_id ||
      t.source_frame !== last.frame_ticks.length - 1 ||
      !tick(t.clip_tick) ||
      last.frame_ticks[t.source_frame] !== t.clip_tick ||
      !names(t.active_nodes) ||
      !names(t.retired_nodes, true) ||
      !same([...t.active_nodes, ...t.retired_nodes].sort(), [...c.nodes].sort())
    )
      fail("invalid terminal ownership");
    if (!Array.isArray(c.visibility_phases) || !c.visibility_phases.length)
      fail("missing visibility phases");
    let next = 0;
    for (const p of c.visibility_phases) {
      if (
        !keys(p, ["first_tick", "last_tick", "active_nodes"]) ||
        p.first_tick !== next ||
        !tick(p.last_tick) ||
        p.last_tick < p.first_tick ||
        !names(p.active_nodes, true) ||
        p.active_nodes.some((n) => !c.nodes.includes(n))
      )
        fail("invalid visibility coverage");
      next = p.last_tick + 1;
    }
    if (
      next !== t.clip_tick + 1 ||
      !same([...c.visibility_phases.at(-1)!.active_nodes].sort(), [...t.active_nodes].sort()) ||
      c.stages.some((s) => s.frame_ticks.some((v) => v !== null && v > t.clip_tick))
    )
      fail("terminal visibility differs");
  }
  if (
    motion.native_passthrough_element_ids.some((id) => owners.has(id)) ||
    !same(
      [...owners, ...motion.native_passthrough_element_ids].sort(),
      [...family.element_ids].sort(),
    )
  )
    fail("explicit element coverage differs");
}
export function componentClipSelections(
  motion: PhysicalElementComponentClips,
  elementId: string,
  frame: number,
): ComponentClipSelection[] {
  if (!tick(frame)) fail("invalid source frame");
  return motion.components.flatMap((c) => {
    const s = c.stages.find((s) => s.element_id === elementId);
    if (!s) return [];
    if (frame >= s.frame_ticks.length) fail("source frame out of range");
    const value = s.frame_ticks[frame];
    return value === null ? [] : [{ component_id: c.id, element_id: elementId, clip_tick: value! }];
  });
}
export function nativeRemainderComponentClips(
  family: Family,
  elementId: string,
  frame: number,
  pairing: NativeRemainderFrame["physical"],
) {
  const m = family.physical.motion;
  if (
    m?.kind !== "native-element-component-clips" ||
    typeof pairing !== "object" ||
    !keys(pairing, ["kind", "element_id", "frame", "components"]) ||
    pairing.kind !== "source-frame-components" ||
    pairing.element_id !== elementId ||
    pairing.frame !== frame
  )
    fail("invalid remainder pairing");
  const selected = componentClipSelections(m as PhysicalElementComponentClips, elementId, frame);
  if (
    !selected.length ||
    !same(
      (pairing as { components: string[] }).components,
      selected.map((s) => s.component_id),
    )
  )
    fail("remainder component coverage differs");
  return selected;
}
export function physicalElementComponentClipPose(
  contract: StateDeliveryContract,
  familyId: string,
  samples: readonly PhysicalStageClock[],
) {
  const family = contract.families.find((f) => f.id === familyId),
    m = family?.physical.motion;
  if (m?.kind !== "native-element-component-clips") fail("missing motion");
  if (
    new Set(samples.map((s) => s.element_id)).size !== samples.length ||
    samples.some(
      (s) =>
        !family!.element_ids.includes(s.element_id) ||
        typeof s.active !== "boolean" ||
        (s.visible !== undefined && typeof s.visible !== "boolean") ||
        !tick(s.tick),
    )
  )
    fail("invalid source clocks");
  const selected: ComponentClipSelection[] = [];
  for (const s of samples) {
    if (!s.active || s.visible === false) continue;
    const e = contract.native.elements.find((e) => e.id === s.element_id)!;
    const frame = nativePresentationFrame(e, s.tick);
    const frameStart = e.frames.slice(0, frame).reduce((sum, f) => sum + f.delay + 1, 0);
    // Physical playback advances within a held source image; the last image holds its pose.
    const elapsed = frame === e.frames.length - 1 ? 0 : s.tick - frameStart;
    selected.push(
      ...componentClipSelections(m as PhysicalElementComponentClips, s.element_id, frame).map(
        (selection) => ({ ...selection, clip_tick: selection.clip_tick + elapsed }),
      ),
    );
  }
  if (new Set(selected.map((s) => s.component_id)).size !== selected.length)
    fail("overlapping component owners");
  const ownedSamples = samples.filter((s) =>
    (m as PhysicalElementComponentClips).components.some((c) =>
      c.stages.some((stage) => stage.element_id === s.element_id),
    ),
  );
  const activated = ownedSamples.some((s) => s.active);
  return {
    endpoint: activated ? ("applied" as const) : ("initial" as const),
    translation: [0, 0, 0] as [number, number, number],
    component_clips: selected,
    visible: activated
      ? !!selected.length
      : ownedSamples.some((s) => s.visible !== false) || !ownedSamples.length,
  };
}
