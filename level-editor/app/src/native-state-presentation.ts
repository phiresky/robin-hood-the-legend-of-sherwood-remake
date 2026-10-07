import type { SourceContractClockBinding } from "./source-contract-clock-binding.ts";
import { decode } from "fast-png";
import {
  nativePresentationFrame,
  nativePresentationOrder,
  nativeBackgroundFrames,
  nativeTransientPatchFrame,
  nativePatchBackgroundFrame,
  type NativePresentationElement,
  validateNativeStatePresentation,
  type NativeImageResource,
  type NativeShadowKey,
  type NativeStatePresentationContract,
  type NativeBackgroundPhase,
} from "../../shared/src/native-state-presentation.ts";
import { missionStateDataHash, type MissionStateSource } from "./mission-state-layer.ts";
import { subdir } from "./fs.ts";

export interface NativePixels {
  width: number;
  height: number;
  data: Uint8Array;
}
export interface NativeLoopDrawSnapshot {
  revision: number;
  mission: string;
  origin: [number, number];
  background: NativePixels;
  /** Active pixel-empty effects still participate in display ordering. */
  orderOnly?: NativePresentationElement[];
  draws: {
    element: NativePresentationElement;
    frame: number;
    pixels: NativePixels;
    x: number;
    y: number;
    shadow?: NativeShadowKey;
  }[];
}
export type NativeResourceReader = (
  resource: Pick<NativeImageResource, "path" | "sha256">,
) => Promise<Uint8Array>;

export function nativeLibraryReader(root: FileSystemDirectoryHandle): NativeResourceReader {
  return async (resource) => {
    const pieces = resource.path.split("/"),
      name = pieces.pop()!;
    const directory = await subdir(root, pieces);
    if (!directory) throw new Error(`Missing native artwork resource: ${resource.path}`);
    return new Uint8Array(
      await (await (await directory.getFileHandle(name)).getFile()).arrayBuffer(),
    );
  };
}

export async function decodeNativeResource(
  resource: NativeImageResource,
  read: NativeResourceReader,
): Promise<NativePixels> {
  const bytes = await read(resource);
  const digest = Array.from(
    new Uint8Array(await crypto.subtle.digest("SHA-256", new Uint8Array(bytes))),
    (n) => n.toString(16).padStart(2, "0"),
  ).join("");
  if (digest !== resource.sha256)
    throw new Error(`Native artwork resource changed: ${resource.path}`);
  const image = decode(bytes, { checkCrc: true });
  if (
    image.width !== resource.width ||
    image.height !== resource.height ||
    image.depth !== 8 ||
    ![3, 4].includes(image.channels)
  )
    throw new Error(`Unsupported native artwork pixels: ${resource.path}`);
  const data = new Uint8Array(image.width * image.height * 4);
  for (let pixel = 0; pixel < image.width * image.height; pixel++) {
    data[pixel * 4] = image.data[pixel * image.channels]!;
    data[pixel * 4 + 1] = image.data[pixel * image.channels + 1]!;
    data[pixel * 4 + 2] = image.data[pixel * image.channels + 2]!;
    data[pixel * 4 + 3] = image.channels === 4 ? image.data[pixel * 4 + 3]! : 255;
  }
  return { width: image.width, height: image.height, data };
}

/** Source-over with explicitly declared destination darkening, clipped to the background. */
export function compositeNativePixels(
  target: NativePixels,
  source: NativePixels,
  x: number,
  y: number,
  shadow?: NativeShadowKey,
) {
  x = Math.floor(x);
  y = Math.floor(y);
  for (let sy = Math.max(0, -y); sy < Math.min(source.height, target.height - y); sy++) {
    for (let sx = Math.max(0, -x); sx < Math.min(source.width, target.width - x); sx++) {
      const from = (sy * source.width + sx) * 4,
        to = ((sy + y) * target.width + sx + x) * 4;
      const alpha = source.data[from + 3]!;
      if (alpha === 0) continue;
      if (shadow && shadow.rgb.every((value, c) => source.data[from + c] === value)) {
        if (alpha !== 255) throw new Error("Native shadow key must have binary alpha");
        const factor = Math.floor(((100 - shadow.strength_percent) * 256) / 100);
        for (let c = 0; c < 3; c++) {
          const bits = c === 1 && shadow.pixel_format === "rgb565" ? 6 : 5;
          const component = target.data[to + c]! >> (8 - bits);
          const darkened = (component * factor) >> 8;
          target.data[to + c] = (darkened << (8 - bits)) | (darkened >> (2 * bits - 8));
        }
        continue;
      }
      if (alpha === 255) {
        target.data.set(source.data.subarray(from, from + 4), to);
        continue;
      }
      const destinationAlpha = target.data[to + 3]!,
        remaining = (destinationAlpha * (255 - alpha)) / 255;
      const combined = alpha + remaining;
      for (let c = 0; c < 3; c++)
        target.data[to + c] = Math.round(
          (source.data[from + c]! * alpha + target.data[to + c]! * remaining) / combined,
        );
      target.data[to + 3] = Math.round(combined);
    }
  }
}

export async function verifyNativePresentationSource(
  contract: NativeStatePresentationContract,
  source: MissionStateSource,
) {
  validateNativeStatePresentation(contract);
  if (
    contract.mission !== source.name ||
    contract.camera_elevation_deg !== source.camera.elevation_deg ||
    (await missionStateDataHash(source.data)) !== contract.mission_data_sha256 ||
    (await missionStateDataHash(source.level)) !== contract.level_data_sha256
  )
    throw new Error("Native artwork preview source or camera changed");
  const targets = source.data.targets;
  if (!Array.isArray(targets)) throw new Error("Native artwork preview has no mission targets");
  const seenAnimations = new Set<number>();
  for (const element of contract.elements) {
    const raw =
      element.source.kind === "map-animation"
        ? source.level.animations[element.source.index]
        : targets[element.source.index];
    if (!raw || (await missionStateDataHash(raw)) !== element.source.sha256)
      throw new Error(`Native artwork element changed: ${element.id}`);
    const row = raw as unknown as Record<string, unknown>;
    const polyline = row[element.source.kind === "map-animation" ? "display_polyline" : "polyline"];
    if (JSON.stringify(polyline) !== JSON.stringify(element.polyline))
      throw new Error(`Native artwork polyline changed: ${element.id}`);
    if (element.source.kind === "map-animation") {
      seenAnimations.add(element.source.index);
      const sprite = row.sprite as Record<string, unknown>;
      if (
        element.display_position[0] !== sprite.position_x ||
        element.display_position[1] !== sprite.position_y ||
        element.active !== row.active
      )
        throw new Error(`Native artwork animation placement changed: ${element.id}`);
    } else if (
      element.display_position[0] !== row.position_x ||
      element.display_position[1] !== row.position_y ||
      element.sort_position[0] !== row.action_position_x ||
      element.sort_position[1] !== row.action_position_y
    )
      throw new Error(`Native artwork target anchors changed: ${element.id}`);
  }
  if (seenAnimations.size !== source.level.animations.length)
    throw new Error("Native artwork preview must bind every global map animation");
  for (let index = 0; index < targets.length; index++) {
    const raw = targets[index] as { polyline?: unknown[] };
    if (
      raw.polyline?.length &&
      !contract.elements.some((e) => e.source.kind === "mission-target" && e.source.index === index)
    )
      throw new Error("Native artwork preview is missing a global target ordering boundary");
  }
  for (const state of contract.background_states ?? []) {
    const rows =
      state.source.kind === "map-patch" ? source.level.patches : source.data.mission_patches;
    const raw = Array.isArray(rows) ? rows[state.source.index] : undefined;
    if (!raw || (await missionStateDataHash(raw)) !== state.source.sha256)
      throw new Error(`Native background source changed: ${state.id}`);
    const row = raw as unknown as Record<string, unknown>;
    const fx = row.element_fx as
      | { sprite?: { position_x?: number; position_y?: number; elevation?: number } }
      | undefined;
    if (
      row.integrate_in_background !== true ||
      row.definitive !== state.definitive ||
      row.start_animation_valid !== state.initial.length > 0 ||
      row.transition_animation_valid !== true ||
      row.end_animation_valid !== state.final.length > 0 ||
      fx?.sprite?.position_x !== state.display_position[0] ||
      fx?.sprite?.position_y !== state.display_position[1] ||
      fx?.sprite?.elevation !== 0
    )
      throw new Error(`Native background semantics changed: ${state.id}`);
  }
  for (const state of contract.patch_states ?? []) {
    const rows =
      state.source.kind === "map-patch" ? source.level.patches : source.data.mission_patches;
    const raw = Array.isArray(rows) ? rows[state.source.index] : undefined;
    if (!raw || (await missionStateDataHash(raw)) !== state.source.sha256)
      throw new Error(`Native patch source changed: ${state.id}`);
    const row = raw as unknown as Record<string, unknown>;
    const fx = row.element_fx as {
      sprite: { position_x: number; position_y: number; elevation: number; profile_name: string };
      active: boolean;
      display_polyline: [number, number][];
    };
    const sprite = fx?.sprite,
      center = state.profile.center;
    const creation =
      state.source.kind === "map-patch"
        ? state.source.index
        : source.level.patches.length +
          source.level.animations.length +
          targets.length +
          state.source.index;
    if (
      row.integrate_in_background !== state.integrate_in_background ||
      row.definitive !== state.definitive ||
      row.start_animation_valid !== state.initial.length > 0 ||
      row.transition_animation_valid !== true ||
      row.end_animation_valid !== state.final.length > 0 ||
      fx?.active !== true ||
      sprite?.profile_name !== state.profile.name ||
      sprite.position_x !== state.display_position[0] ||
      sprite.position_y !== state.display_position[1] ||
      sprite.elevation !== state.elevation ||
      state.creation_order !== creation ||
      state.sort_position[0] !== sprite.position_x + center[0] ||
      state.sort_position[1] !== sprite.position_y + center[1] ||
      state.display_order !== sprite.position_y + center[1] + sprite.elevation ||
      JSON.stringify(state.polyline) !== JSON.stringify(fx.display_polyline) ||
      (state.initial.length > 0 && !state.initial_loop) ||
      (state.final.length > 0 && !state.final_loop)
    )
      throw new Error(`Native patch semantics changed: ${state.id}`);
  }
  if (contract.painted_order_equivalence) {
    const { painted_order_equivalence, ...bound } = contract;
    if (
      (await missionStateDataHash({
        contract: bound,
        elements: painted_order_equivalence.elements,
      })) !== painted_order_equivalence.contract_sha256
    )
      throw new Error("Painted ordering contract changed");
  }
  nativePresentationOrder(
    contract.elements.map((e) =>
      !e.active && e.initial_frame
        ? { ...e, active: true, frames: [e.initial_frame], loop: false }
        : e,
    ),
    contract.painted_order_equivalence?.elements,
  );
}

/** Resource ownership and integer timing; the containing viewport owns the only clock. */
export class NativeStatePresentation {
  private epoch = 0;
  private disposed = false;
  private images = new Map<string, NativePixels>();
  private contract: NativeStatePresentationContract | undefined;
  private offsets = new Map<string, number>();
  private activeElements = new Map<string, boolean>();
  private verifiedDisjointPair: [string, string] | undefined;
  private patches = new Map<string, { phase: NativeBackgroundPhase; offset: number }>();
  private backgrounds = new Map<string, { phase: NativeBackgroundPhase; offset: number }>();
  private seconds = 0;
  private currentTick = 0;
  private externalClock = false;
  private externalSignature = "";
  private playing = false;
  private revision = 0;
  private cached: { revision: number; pixels: NativePixels } | undefined;
  get ready() {
    return !!this.contract;
  }
  get tick() {
    return this.currentTick;
  }
  get mission() {
    return this.contract?.mission;
  }
  get isPlaying() {
    return this.playing;
  }
  clear() {
    this.epoch++;
    this.images.clear();
    this.contract = undefined;
    this.offsets.clear();
    this.activeElements.clear();
    this.backgrounds.clear();
    this.patches.clear();
    this.verifiedDisjointPair = undefined;
    this.currentTick = 0;
    this.externalClock = false;
    this.externalSignature = "";
    this.seconds = 0;
    this.playing = false;
    this.cached = undefined;
    this.revision++;
  }
  async set(
    contract: NativeStatePresentationContract,
    source: MissionStateSource,
    read: NativeResourceReader,
  ) {
    if (this.disposed) throw new Error("Native artwork preview is disposed");
    this.clear();
    const epoch = this.epoch,
      frozen = structuredClone(contract),
      frozenSource = structuredClone(source);
    try {
      await verifyNativePresentationSource(frozen, frozenSource);
    } catch (error) {
      if (this.disposed || epoch !== this.epoch) return false;
      throw error;
    }
    if (this.disposed || epoch !== this.epoch) return false;
    const resources = new Map<string, NativeImageResource>();
    const frames = [
      ...frozen.elements.flatMap((e) => [
        ...e.frames,
        ...(e.initial_frame ? [e.initial_frame] : []),
      ]),
      ...[...(frozen.background_states ?? []), ...(frozen.patch_states ?? [])].flatMap((s) => [
        ...s.initial,
        ...s.transition,
        ...s.final,
      ]),
    ];
    for (const resource of [frozen.background, ...frames]) {
      const prior = resources.get(resource.path);
      if (
        prior &&
        (prior.sha256 !== resource.sha256 ||
          prior.width !== resource.width ||
          prior.height !== resource.height)
      )
        throw new Error("Conflicting native artwork resource binding");
      resources.set(resource.path, resource);
    }
    for (const state of frozen.patch_states ?? []) {
      if (this.disposed || epoch !== this.epoch) return false;
      let bytes: Uint8Array;
      try {
        bytes = await read(state.profile);
      } catch (error) {
        if (this.disposed || epoch !== this.epoch) return false;
        throw error;
      }
      const hash = Array.from(
        new Uint8Array(await crypto.subtle.digest("SHA-256", new Uint8Array(bytes))),
        (n) => n.toString(16).padStart(2, "0"),
      ).join("");
      if (this.disposed || epoch !== this.epoch) return false;
      if (hash !== state.profile.sha256)
        throw new Error(`Native patch profile changed: ${state.id}`);
      const value = JSON.parse(new TextDecoder().decode(bytes));
      const profile = Array.isArray(value.profiles)
        ? value.profiles.find((p: { name: string }) => p.name === state.profile.name)
        : value;
      if (
        !profile ||
        profile.name !== state.profile.name ||
        profile.center_x !== state.profile.center[0] ||
        profile.center_y !== state.profile.center[1]
      )
        throw new Error(`Native patch center changed: ${state.id}`);
    }
    const images = new Map<string, NativePixels>();
    // Bound decoding work while allowing mission changes between files.
    for (const resource of resources.values()) {
      if (this.disposed || epoch !== this.epoch) return false;
      try {
        images.set(resource.path, await decodeNativeResource(resource, read));
      } catch (error) {
        if (this.disposed || epoch !== this.epoch) return false;
        throw error;
      }
    }
    if (this.disposed || epoch !== this.epoch) return false;
    for (const frame of frames) {
      if (!frame.shadow_key) continue;
      const data = images.get(frame.path)!.data;
      for (let pixel = 0; pixel < data.length; pixel += 4)
        if (
          data[pixel + 3] !== 0 &&
          data[pixel + 3] !== 255 &&
          frame.shadow_key.rgb.every((value, c) => data[pixel + c] === value)
        )
          throw new Error(`Native shadow key must have binary alpha: ${frame.path}`);
    }
    const pair = frozen.painted_order_equivalence?.elements;
    if (pair) {
      const [a, b] = pair.map((id) => frozen.elements.find((e) => e.id === id)!);
      if (!a || !b) throw new Error("Missing painted ordering element");
      for (const af of [...a.frames, ...(a.initial_frame ? [a.initial_frame] : [])])
        for (const bf of [...b.frames, ...(b.initial_frame ? [b.initial_frame] : [])]) {
          const ax = a.display_position[0] + af.offset[0],
            ay = a.display_position[1] + af.offset[1],
            bx = b.display_position[0] + bf.offset[0],
            by = b.display_position[1] + bf.offset[1];
          const ap = images.get(af.path)!,
            bp = images.get(bf.path)!;
          for (let y = Math.max(ay, by); y < Math.min(ay + af.height, by + bf.height); y++)
            for (let x = Math.max(ax, bx); x < Math.min(ax + af.width, bx + bf.width); x++)
              if (
                ap.data[((y - ay) * af.width + x - ax) * 4 + 3] &&
                bp.data[((y - by) * bf.width + x - bx) * 4 + 3]
              )
                throw new Error("Painted ordering pair overlaps");
        }
    }
    this.verifiedDisjointPair = pair;
    this.contract = frozen;
    this.images = images;
    this.revision++;
    return true;
  }
  setPlaying(value: boolean) {
    if (this.externalClock) throw new Error("External source clock owns playback");
    this.playing = value;
  }
  setElementState(id: string, active: boolean, tick = 0) {
    if (this.externalClock) throw new Error("External source clock owns element state");
    if (!this.contract?.elements.some((e) => e.id === id))
      throw new Error(`Unknown native preview element: ${id}`);
    if (typeof active !== "boolean" || !Number.isSafeInteger(tick) || tick < 0)
      throw new Error("Invalid native element state");
    this.activeElements.set(id, active);
    this.offsets.set(id, tick - this.currentTick);
    this.revision++;
  }
  setBackgroundState(id: string, phase: NativeBackgroundPhase, tick = 0) {
    const state = this.contract?.background_states?.find((s) => s.id === id);
    if (!state) throw new Error(`Unknown native background state: ${id}`);
    if (
      !["initial", "forward", "applied", "reverse"].includes(phase) ||
      !Number.isSafeInteger(tick) ||
      tick < 0 ||
      (phase === "reverse" && state.definitive)
    )
      throw new Error("Invalid native background state");
    this.backgrounds.set(id, { phase, offset: tick - this.currentTick });
    this.revision++;
  }
  setPatchState(id: string, phase: NativeBackgroundPhase, tick = 0) {
    const state = this.contract?.patch_states?.find((s) => s.id === id);
    if (!state) throw new Error(`Unknown native patch state: ${id}`);
    nativePatchBackgroundFrame(state, phase, tick);
    nativeTransientPatchFrame(state, phase, tick);
    this.patches.set(id, { phase, offset: tick - this.currentTick });
    this.revision++;
  }
  seek(tick: number, id?: string) {
    if (this.externalClock) throw new Error("External source clock owns seek");
    if (!this.contract) throw new Error("Native artwork preview is not loaded");
    if (!Number.isSafeInteger(tick) || tick < 0) throw new Error("Invalid native preview tick");
    this.playing = false;
    if (id !== undefined) {
      if (!this.contract.elements.some((e) => e.id === id))
        throw new Error(`Unknown native preview element: ${id}`);
      this.offsets.set(id, tick - this.currentTick);
    } else {
      this.currentTick = tick;
      this.seconds = 0;
      this.offsets.clear();
    }
    this.revision++;
  }
  /** Atomic read-only clock consumption; patch activation keeps its separate timing path. */
  sampleExternalClocks(rows: ReturnType<SourceContractClockBinding["snapshot"]>, focus: string) {
    if (!this.contract) throw new Error("Native artwork preview is not loaded");
    if (this.contract.patch_states?.length || this.contract.background_states?.length)
      throw new Error("Loop clock snapshots cannot own patch timing");
    const byId = new Map(rows.map((r) => [r.id, r]));
    if (byId.size !== rows.length || rows.length !== this.contract.elements.length)
      throw new Error("External source clock inventory differs");
    for (const e of this.contract.elements) {
      const r = byId.get(e.id);
      if (
        !r ||
        r.source.kind !== e.source.kind ||
        r.source.index !== e.source.index ||
        r.source.sha256 !== e.source.sha256 ||
        !Number.isSafeInteger(r.tick) ||
        r.tick < 0
      )
        throw new Error("External source clock identity differs");
    }
    const selected = byId.get(focus);
    if (!selected) throw new Error("Unknown loop clock focus");
    const signature = JSON.stringify(rows.map((r) => [r.id, r.tick, r.active, r.playing]));
    if (signature === this.externalSignature) return false;
    this.externalClock = true;
    this.externalSignature = signature;
    this.currentTick = selected.tick;
    this.seconds = 0;
    this.playing = rows.some((r) => r.active && r.playing);
    for (const r of rows) {
      this.offsets.set(r.id, r.tick - this.currentTick);
      this.activeElements.set(r.id, r.active);
    }
    this.revision++;
    return true;
  }
  advance(seconds: number) {
    if (!Number.isFinite(seconds) || seconds < 0)
      throw new Error("Invalid native preview elapsed time");
    if (this.externalClock || !this.playing || !this.contract) return false;
    this.seconds += seconds;
    const ticks = Math.floor(this.seconds * 25 + 1e-9);
    if (!ticks) return false;
    if (!Number.isSafeInteger(this.currentTick + ticks))
      throw new Error("Native preview tick overflow");
    this.seconds -= ticks / 25;
    this.currentTick += ticks;
    this.revision++;
    return true;
  }
  /** Borrowed decoded resources for an explicit ordered-loop compositor. No clock advancement. */
  loopSourceBinding() {
    if (
      !this.contract ||
      this.contract.patch_states?.length ||
      this.contract.background_states?.length
    )
      throw new Error("Native loop source is not ready or has unsupported patch ownership");
    const contract = this.contract;
    return {
      mission: contract.mission,
      mission_data_sha256: contract.mission_data_sha256,
      level_data_sha256: contract.level_data_sha256,
      background: { ...contract.background },
    };
  }
  loopDrawSnapshot(): NativeLoopDrawSnapshot {
    const contract = this.contract;
    if (!contract) throw new Error("Native artwork preview is not loaded");
    if (contract.patch_states?.length || contract.background_states?.length)
      throw new Error("Actor loop composition cannot infer patch or background state ownership");
    const draws: NativeLoopDrawSnapshot["draws"] = [];
    const orderOnly: NativePresentationElement[] = [];
    for (const original of contract.elements) {
      const active = this.activeElements.get(original.id) ?? original.active;
      const element =
        !active && original.initial_frame
          ? { ...original, active: true, frames: [original.initial_frame], loop: false }
          : { ...original, active };
      const frame = nativePresentationFrame(
        element,
        Math.max(0, this.currentTick + (this.offsets.get(element.id) ?? 0)),
      );
      if (frame < 0) {
        if (element.active && element.frames.length === 0) orderOnly.push(element);
        continue;
      }
      const image = element.frames[frame]!;
      draws.push({
        element,
        frame,
        pixels: this.images.get(image.path)!,
        x: element.display_position[0] + image.offset[0] - contract.origin[0],
        y: element.display_position[1] + image.offset[1] - contract.origin[1],
        shadow: image.shadow_key,
      });
    }
    return {
      revision: this.revision,
      mission: contract.mission,
      origin: [...contract.origin],
      background: this.images.get(contract.background.path)!,
      orderOnly,
      draws,
    };
  }
  pixels(): NativePixels {
    if (!this.contract) throw new Error("Native artwork preview is not loaded");
    if (this.cached?.revision === this.revision) return this.cached.pixels;
    const background = this.images.get(this.contract.background.path)!;
    const pixels = {
      width: background.width,
      height: background.height,
      data: new Uint8Array(background.data),
    };
    for (const state of this.contract.background_states ?? []) {
      const selected = this.backgrounds.get(state.id) ?? { phase: "initial" as const, offset: 0 };
      for (const frame of nativeBackgroundFrames(
        state,
        selected.phase,
        Math.max(0, this.currentTick + selected.offset),
      ))
        compositeNativePixels(
          pixels,
          this.images.get(frame.path)!,
          state.display_position[0] + frame.offset[0] - this.contract.origin[0],
          state.display_position[1] + frame.offset[1] - this.contract.origin[1],
          frame.shadow_key,
        );
    }
    for (const state of this.contract.patch_states ?? []) {
      const selected = this.patches.get(state.id) ?? { phase: "initial" as const, offset: 0 };
      const stamp = nativePatchBackgroundFrame(
        state,
        selected.phase,
        Math.max(0, this.currentTick + selected.offset),
      );
      if (stamp)
        compositeNativePixels(
          pixels,
          this.images.get(stamp.path)!,
          state.display_position[0] + stamp.offset[0] - this.contract.origin[0],
          state.display_position[1] + stamp.offset[1] - this.contract.origin[1],
          stamp.shadow_key,
        );
    }
    const elements: Omit<NativePresentationElement, "source">[] = this.contract.elements.map(
      (e) => {
        const active = this.activeElements.get(e.id) ?? e.active;
        return !active && e.initial_frame
          ? { ...e, active: true, frames: [e.initial_frame], loop: false }
          : { ...e, active };
      },
    );
    for (const state of [...(this.contract.patch_states ?? [])].sort(
      (a, b) => a.creation_order - b.creation_order,
    )) {
      const selected = this.patches.get(state.id) ?? { phase: "initial" as const, offset: 0 };
      const frame = nativeTransientPatchFrame(
        state,
        selected.phase,
        Math.max(0, this.currentTick + selected.offset),
      );
      if (!frame) continue;
      if (state.layer === "background")
        compositeNativePixels(
          pixels,
          this.images.get(frame.path)!,
          state.display_position[0] + frame.offset[0] - this.contract.origin[0],
          state.display_position[1] + frame.offset[1] - this.contract.origin[1],
          frame.shadow_key,
        );
      else elements.push({ ...state, active: true, frames: [frame], loop: false });
    }
    for (const element of nativePresentationOrder(elements, this.verifiedDisjointPair)) {
      const index = nativePresentationFrame(
        element,
        Math.max(0, this.currentTick + (this.offsets.get(element.id) ?? 0)),
      );
      if (index < 0) continue;
      const frame = element.frames[index]!;
      compositeNativePixels(
        pixels,
        this.images.get(frame.path)!,
        element.display_position[0] + frame.offset[0] - this.contract.origin[0],
        element.display_position[1] + frame.offset[1] - this.contract.origin[1],
        frame.shadow_key,
      );
    }
    this.cached = { revision: this.revision, pixels };
    return pixels;
  }
  dispose() {
    if (!this.disposed) {
      this.clear();
      this.disposed = true;
    }
  }
}

/** Isolated source-view gestures never reach the physical editing canvas. */
export class NativeArtworkSurface {
  readonly element: HTMLDivElement;
  readonly canvas: HTMLCanvasElement;
  private readonly source = document.createElement("canvas");
  private readonly label = document.createElement("span");
  private readonly listeners = new AbortController();
  private readonly observer: ResizeObserver;
  private image: NativePixels | undefined;
  private scale = 1;
  private x = 0;
  private y = 0;
  private drag: { x: number; y: number } | undefined;
  private readonly previousPosition: string | undefined;
  constructor(parent: HTMLElement) {
    if (getComputedStyle(parent).position === "static") {
      this.previousPosition = parent.style.position;
      parent.style.position = "relative";
    }
    this.element = document.createElement("div");
    this.element.style.cssText = "position:absolute;inset:0;z-index:4;background:#202020";
    this.canvas = document.createElement("canvas");
    this.canvas.tabIndex = 0;
    this.canvas.setAttribute("aria-label", "Original artwork preview; pan and zoom");
    this.canvas.style.cssText = "width:100%;height:100%;display:block;touch-action:none";
    const label = this.label;
    label.textContent = "Original artwork · map animation and mission effects";
    label.style.cssText =
      "position:absolute;left:8px;top:8px;padding:4px 8px;background:#111d;color:white;font:12px sans-serif;pointer-events:none";
    this.element.append(this.canvas, label);
    parent.append(this.element);
    const signal = this.listeners.signal;
    this.canvas.addEventListener(
      "pointerdown",
      (e) => {
        e.stopPropagation();
        this.canvas.focus();
        this.drag = { x: e.clientX, y: e.clientY };
        this.canvas.setPointerCapture(e.pointerId);
      },
      { signal },
    );
    this.canvas.addEventListener(
      "pointermove",
      (e) => {
        e.stopPropagation();
        if (!this.drag) return;
        this.x += e.clientX - this.drag.x;
        this.y += e.clientY - this.drag.y;
        this.drag = { x: e.clientX, y: e.clientY };
        this.draw();
      },
      { signal },
    );
    this.canvas.addEventListener(
      "pointerup",
      (e) => {
        e.stopPropagation();
        this.drag = undefined;
        if (this.canvas.hasPointerCapture(e.pointerId))
          this.canvas.releasePointerCapture(e.pointerId);
      },
      { signal },
    );
    this.canvas.addEventListener(
      "pointercancel",
      () => {
        this.drag = undefined;
      },
      { signal },
    );
    this.canvas.addEventListener(
      "wheel",
      (e) => {
        e.preventDefault();
        e.stopPropagation();
        const rect = this.canvas.getBoundingClientRect(),
          px = e.clientX - rect.left,
          py = e.clientY - rect.top;
        const next = Math.max(0.02, Math.min(64, this.scale * Math.exp(-e.deltaY * 0.001)));
        this.x = px - ((px - this.x) * next) / this.scale;
        this.y = py - ((py - this.y) * next) / this.scale;
        this.scale = next;
        this.draw();
      },
      { signal, passive: false },
    );
    this.canvas.addEventListener("keydown", (e) => e.stopPropagation(), { signal });
    this.canvas.addEventListener("contextmenu", (e) => e.preventDefault(), { signal });
    this.observer = new ResizeObserver(() => this.draw());
    this.observer.observe(parent);
  }
  setActorPreview(enabled: boolean) {
    this.label.textContent = enabled
      ? "Editor actor preview · original artwork"
      : "Original artwork · map animation and mission effects";
    this.canvas.setAttribute(
      "aria-label",
      enabled ? "Editor actor preview; pan and zoom" : "Original artwork preview; pan and zoom",
    );
  }
  update(image: NativePixels) {
    if (this.image === image) return;
    const first = !this.image;
    this.image = image;
    this.source.width = image.width;
    this.source.height = image.height;
    this.source
      .getContext("2d")!
      .putImageData(
        new ImageData(new Uint8ClampedArray(image.data), image.width, image.height),
        0,
        0,
      );
    if (first) {
      const rect = this.canvas.getBoundingClientRect();
      this.scale = Math.min(rect.width / image.width, rect.height / image.height);
      this.x = (rect.width - image.width * this.scale) / 2;
      this.y = (rect.height - image.height * this.scale) / 2;
    }
    this.draw();
  }
  private draw() {
    const rect = this.canvas.getBoundingClientRect(),
      ratio = window.devicePixelRatio || 1;
    this.canvas.width = Math.max(1, Math.round(rect.width * ratio));
    this.canvas.height = Math.max(1, Math.round(rect.height * ratio));
    const context = this.canvas.getContext("2d")!;
    context.scale(ratio, ratio);
    context.imageSmoothingEnabled = false;
    context.fillStyle = "#202020";
    context.fillRect(0, 0, rect.width, rect.height);
    if (this.image)
      context.drawImage(
        this.source,
        this.x,
        this.y,
        this.image.width * this.scale,
        this.image.height * this.scale,
      );
  }
  dispose() {
    this.listeners.abort();
    this.observer.disconnect();
    if (this.previousPosition !== undefined && this.element.parentElement)
      this.element.parentElement.style.position = this.previousPosition;
    this.element.remove();
    this.source.width = this.source.height = 1;
  }
}
