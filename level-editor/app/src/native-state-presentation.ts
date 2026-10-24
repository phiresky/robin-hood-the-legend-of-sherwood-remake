import {
  validateStateClockSamples,
  type StateClockBatch,
} from "../../shared/src/state-event-clocks.ts";
import type { SourceContractClockBinding } from "./source-contract-clock-binding.ts";
import { decode } from "fast-png";
import {
  nativePresentationFrame,
  nativePresentationOrder,
  nativeBackgroundFrames,
  nativeTransientPatchFrame,
  nativePatchBackgroundFrame,
  type NativePresentationElement,
  type NativeInitialContextMode,
  type NativePatchState,
  validateNativeStatePresentation,
  type NativeImageResource,
  type NativeShadowKey,
  type NativeStatePresentationContract,
  type NativeBackgroundPhase,
} from "../../shared/src/native-state-presentation.ts";
import type { NativeRemainderBinding } from "../../shared/src/state-delivery.ts";
import { missionStateDataHash, type MissionStateSource } from "./mission-state-layer.ts";
import { subdir } from "./fs.ts";

export interface NativePixels {
  width: number;
  height: number;
  data: Uint8Array;
}
export type NativeLoopDrawElement = Omit<NativePresentationElement, "source"> & {
  source: NativePresentationElement["source"] | NativePatchState["source"];
};
export interface NativeLoopDrawSnapshot {
  revision: number;
  mission: string;
  origin: [number, number];
  background: NativePixels;
  /** Active pixel-empty effects still participate in display ordering. */
  orderOnly?: NativeLoopDrawElement[];
  draws: {
    element: NativeLoopDrawElement;
    frame: number;
    /** Present on production snapshots; retained separately from stationary initial artwork. */
    sourceTick?: number;
    sourceActive?: boolean;
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
  signal?: AbortSignal,
): Promise<NativePixels> {
  signal?.throwIfAborted();
  const bytes = await read(resource);
  signal?.throwIfAborted();
  const digest = Array.from(
    new Uint8Array(await crypto.subtle.digest("SHA-256", new Uint8Array(bytes))),
    (n) => n.toString(16).padStart(2, "0"),
  ).join("");
  signal?.throwIfAborted();
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

/** Decode one hash-pinned grey source-domain mask against its decoded source frame.
 * Value 0 must mark exactly the transparent source pixels; values above 3 are rejected.
 */
export async function decodeNativeRemainderMask(
  resource: NativeImageResource,
  read: NativeResourceReader,
  source: NativePixels,
): Promise<Uint8Array> {
  const bytes = await read(resource);
  const digest = Array.from(
    new Uint8Array(await crypto.subtle.digest("SHA-256", new Uint8Array(bytes))),
    (n) => n.toString(16).padStart(2, "0"),
  ).join("");
  if (digest !== resource.sha256)
    throw new Error(`Native remainder mask changed: ${resource.path}`);
  const image = decode(bytes, { checkCrc: true });
  if (
    image.width !== resource.width ||
    image.height !== resource.height ||
    image.width !== source.width ||
    image.height !== source.height ||
    image.depth !== 8 ||
    image.channels !== 1 ||
    image.palette
  )
    throw new Error(`Unsupported native remainder mask: ${resource.path}`);
  const values = new Uint8Array(image.width * image.height);
  for (let pixel = 0; pixel < values.length; pixel++) {
    const value = image.data[pixel]!;
    if (value > 3 || (value === 0) !== (source.data[pixel * 4 + 3] === 0))
      throw new Error(`Native remainder mask differs from source coverage: ${resource.path}`);
    values[pixel] = value;
  }
  return values;
}

/** Clear every source pixel whose reviewed mask value is not kept. */
export function maskNativeRemainder(
  source: NativePixels,
  mask: Uint8Array,
  drawn: readonly number[],
): NativePixels {
  if (mask.length !== source.width * source.height)
    throw new Error("Native remainder mask size differs");
  const data = new Uint8Array(source.data);
  for (let pixel = 0; pixel < mask.length; pixel++)
    if (!drawn.includes(mask[pixel]!)) data.fill(0, pixel * 4, pixel * 4 + 4);
  return { width: source.width, height: source.height, data };
}

/** A physical layer rendered in the background frame for one remainder element. */
export type NativeRemainderLayers = ReadonlyMap<string, NativePixels>;

/** Keep resource order independent of I/O completion order, with four jobs at most. */
async function decodeNativeResources(
  resources: NativeImageResource[],
  read: NativeResourceReader,
  signal: AbortSignal,
): Promise<Map<string, NativePixels> | undefined> {
  type Result = { image: NativePixels } | { error: unknown };
  const pending = new Map<number, Promise<Result>>();
  const images = new Map<string, NativePixels>();
  let failed = false;
  let cancel!: () => void;
  const cancelled = new Promise<undefined>((resolve) => {
    cancel = () => resolve(undefined);
  });
  signal.addEventListener("abort", cancel, { once: true });
  const start = (index: number) => {
    // Observe every rejection immediately, including reads that finish after retirement.
    pending.set(
      index,
      decodeNativeResource(resources[index]!, read, signal).then(
        (image) => ({ image }),
        (error: unknown) => {
          failed = true;
          return { error };
        },
      ),
    );
  };
  try {
    if (signal.aborted) return undefined;
    for (let index = 0; index < Math.min(4, resources.length); index++) start(index);
    for (let index = 0; index < resources.length; index++) {
      const result = await Promise.race([pending.get(index)!, cancelled]);
      if (!result || signal.aborted) return undefined;
      if ("error" in result) throw result.error;
      pending.delete(index);
      images.set(resources[index]!.path, result.image);
      if (!failed && index + 4 < resources.length) start(index + 4);
    }
    return images;
  } finally {
    signal.removeEventListener("abort", cancel);
  }
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

/** Source elevation selects a drawing stage, not a physical model altitude. */
function backgroundEffectIds(
  contract: NativeStatePresentationContract,
  source: MissionStateSource,
) {
  const ids = new Map<string, number>();
  for (const element of contract.elements) {
    if (element.source.kind !== "map-animation") continue;
    const elevation = source.level.animations[element.source.index]?.sprite.elevation;
    if (!Number.isFinite(elevation))
      throw new Error(`Missing native effect elevation: ${element.id}`);
    if (elevation === 0)
      ids.set(element.id, (source.level.patches?.length ?? 0) + element.source.index);
  }
  return ids;
}

export async function verifyNativePresentationSource(
  contract: NativeStatePresentationContract,
  source: MissionStateSource,
  initialContext?: NativeInitialContextMode,
) {
  validateNativeStatePresentation(contract);
  if (initialContext !== undefined && initialContext !== "immutable-source-initial")
    throw new Error("Invalid immutable source initial context");
  const initialRanks = initialContext ? initialContextRanks(source) : undefined;
  if (
    contract.mission !== source.name ||
    contract.camera_elevation_deg !== source.camera.elevation_deg ||
    (await missionStateDataHash(source.data)) !== contract.mission_data_sha256 ||
    (await missionStateDataHash(source.level)) !== contract.level_data_sha256
  )
    throw new Error("Native artwork preview source or camera changed");
  for (const binding of contract.cover_artwork?.bindings ?? []) {
    const patch = source.level.patches[binding.source.index];
    if (
      !patch ||
      (await missionStateDataHash(patch)) !== binding.source.sha256 ||
      !patch.start_animation_valid ||
      patch.transition_animation_valid ||
      patch.end_animation_valid
    )
      throw new Error("Native cover artwork source changed");
  }
  const targets = source.data.targets;
  if (!Array.isArray(targets)) throw new Error("Native artwork preview has no mission targets");
  const seenAnimations = new Set<number>();
  for (const element of contract.elements) {
    if (
      initialRanks &&
      element.creation_order !== initialRanks.get(`${element.source.kind}:${element.source.index}`)
    )
      throw new Error(`Immutable initial element construction rank differs: ${element.id}`);
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
    const creation = initialRanks
      ? initialRanks.get(`${state.source.kind}:${state.source.index}`)
      : state.source.kind === "map-patch"
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
  const backgroundIds = backgroundEffectIds(contract, source);
  nativePresentationOrder(
    contract.elements
      .filter((e) => !backgroundIds.has(e.id))
      .map((e) =>
        !e.active && e.initial_frame
          ? { ...e, active: true, frames: [e.initial_frame], loop: false }
          : e,
      ),
    contract.painted_order_equivalence?.elements,
  );
}

/** Exact supported construction stream; retained context never acquires an animation clock. */
function initialContextRanks(source: MissionStateSource) {
  if (
    JSON.stringify(
      (source.level as typeof source.level & { element_chunk_order?: unknown }).element_chunk_order,
    ) !== JSON.stringify(["Animation", "Patch"]) ||
    JSON.stringify(source.data.element_chunk_order) !==
      JSON.stringify(["Patch", "Element", "Bonus", "Scroll", "Mobile"]) ||
    JSON.stringify(source.data.element_group_order) !==
      JSON.stringify(["Animal", "BeamMe", "Civilian", "PcToRescue", "Soldier", "Target"])
  )
    throw new Error("Unsupported immutable initial context construction order");
  const ranks = new Map<string, number>();
  let rank = 0;
  const add = (kind: string, rows: unknown) => {
    if (!Array.isArray(rows)) throw new Error(`Missing immutable context source rows: ${kind}`);
    rows.forEach((_, index) => ranks.set(`${kind}:${index}`, rank++));
  };
  add("map-animation", source.level.animations);
  add("map-patch", source.level.patches);
  add("mission-patch", source.data.mission_patches);
  for (const family of ["civilians", "pcs_to_rescue", "soldiers", "targets"])
    add(family === "targets" ? "mission-target" : family, source.data[family]);
  return ranks;
}

/** Resource ownership and integer timing; the containing viewport owns the only clock. */
export class NativeStatePresentation {
  private epoch = 0;
  private disposed = false;
  private images = new Map<string, NativePixels>();
  private resourceLoad: AbortController | undefined;
  private contract: NativeStatePresentationContract | undefined;
  private initialContext: NativeInitialContextMode | undefined;
  private initialElements: NativeLoopDrawElement[] = [];
  private initialRanks = new Map<string, number>();
  private offsets = new Map<string, number>();
  private coverApplied = new Set<string>();
  private visibleElements = new Map<string, boolean>();
  private activeElements = new Map<string, boolean>();
  private backgroundEffectIds = new Map<string, number>();
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
  private remainders = new Map<string, (NativePixels | undefined)[]>();
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
    this.resourceLoad?.abort();
    this.resourceLoad = undefined;
    this.images.clear();
    this.contract = undefined;
    this.initialContext = undefined;
    this.initialElements = [];
    this.initialRanks.clear();
    this.offsets.clear();
    this.activeElements.clear();
    this.backgroundEffectIds.clear();
    this.visibleElements.clear();
    this.coverApplied.clear();
    this.backgrounds.clear();
    this.patches.clear();
    this.verifiedDisjointPair = undefined;
    this.currentTick = 0;
    this.externalClock = false;
    this.externalSignature = "";
    this.seconds = 0;
    this.playing = false;
    this.cached = undefined;
    this.remainders.clear();
    this.revision++;
  }
  async set(
    contract: NativeStatePresentationContract,
    source: MissionStateSource,
    read: NativeResourceReader,
    initialContext?: NativeInitialContextMode,
  ) {
    if (this.disposed) throw new Error("Native artwork preview is disposed");
    this.clear();
    if (initialContext !== undefined && initialContext !== "immutable-source-initial")
      throw new Error("Invalid immutable source initial context");
    const epoch = this.epoch,
      frozen = structuredClone(contract),
      frozenSource = structuredClone(source);
    try {
      await verifyNativePresentationSource(frozen, frozenSource, initialContext);
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
    for (const resource of [
      frozen.background,
      ...(frozen.cover_artwork?.variants.map((v) => v.image) ?? []),
      ...frames,
    ]) {
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
    const load = new AbortController();
    this.resourceLoad = load;
    let images: Map<string, NativePixels> | undefined;
    try {
      images = await decodeNativeResources([...resources.values()], read, load.signal);
    } catch (error) {
      load.abort();
      if (this.disposed || epoch !== this.epoch) return false;
      throw error;
    } finally {
      if (this.resourceLoad === load) this.resourceLoad = undefined;
    }
    if (!images || this.disposed || epoch !== this.epoch) return false;
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
    this.backgroundEffectIds = backgroundEffectIds(frozen, frozenSource);
    if (initialContext) {
      this.initialRanks = initialContextRanks(frozenSource);
      const rank = (source: NativeLoopDrawElement["source"]) => {
        const value = this.initialRanks.get(`${source.kind}:${source.index}`);
        if (value === undefined) throw new Error("Missing immutable initial context rank");
        return value;
      };
      for (const element of frozen.elements)
        if (this.backgroundEffectIds.has(element.id))
          this.backgroundEffectIds.set(element.id, rank(element.source));
      for (const state of [...(frozen.background_states ?? []), ...(frozen.patch_states ?? [])]) {
        const rows =
          state.source.kind === "map-patch"
            ? frozenSource.level.patches
            : frozenSource.data.mission_patches;
        const raw = (rows as { element_fx: { active: boolean } }[])[state.source.index]!;
        if (typeof raw.element_fx?.active !== "boolean")
          throw new Error("Missing initial patch activity");
        const patch = "layer" in state;
        const frame = patch
          ? nativeTransientPatchFrame(state, "initial", 0)
          : nativeBackgroundFrames(state, "initial", 0)[0];
        if (!raw.element_fx.active || !frame) continue;
        const background = !patch || state.layer === "background";
        if (background) this.backgroundEffectIds.set(state.id, rank(state.source));
        this.initialElements.push({
          ...state,
          active: true,
          loop: false,
          frames: [frame],
          creation_order: rank(state.source),
          sort_position: patch ? state.sort_position : [...state.display_position],
          display_order: patch ? state.display_order : state.display_position[1],
          polyline: patch ? state.polyline : [],
        });
      }
      this.initialContext = initialContext;
    }
    this.contract = frozen;
    this.images = images;
    this.revision++;
    return true;
  }
  /** Load reviewed per-frame remainders after the artwork; unbound frames stay whole. */
  async setRemainders(bindings: readonly NativeRemainderBinding[], read: NativeResourceReader) {
    const contract = this.contract;
    if (!contract) throw new Error("Native artwork preview is not loaded");
    if (this.initialContext) throw new Error("Immutable source initial context has no remainder");
    const epoch = this.epoch;
    const loaded = new Map<string, (NativePixels | undefined)[]>();
    for (const binding of bindings) {
      const element = contract.elements.find((e) => e.id === binding.element_id);
      if (
        !element ||
        loaded.has(element.id) ||
        element.loop ||
        binding.frames.length !== element.frames.length
      )
        throw new Error(`Invalid native remainder element: ${binding.element_id}`);
      const frames: (NativePixels | undefined)[] = [];
      for (const [index, frame] of binding.frames.entries()) {
        if (!frame) {
          frames.push(undefined);
          continue;
        }
        const source = this.images.get(element.frames[index]!.path)!;
        const mask = await decodeNativeRemainderMask(frame.mask, read, source);
        if (this.disposed || epoch !== this.epoch) return false;
        frames.push(maskNativeRemainder(source, mask, binding.drawn_values));
      }
      loaded.set(element.id, frames);
    }
    this.remainders = loaded;
    this.revision++;
    return true;
  }
  /** The complete source frame every remainder physical layer is rendered into. */
  remainderFrame() {
    if (!this.contract) throw new Error("Native artwork preview is not loaded");
    const background = this.currentBackground();
    return {
      origin: [...this.contract.origin] as [number, number],
      width: background.width,
      height: background.height,
      elevation: this.contract.camera_elevation_deg,
    };
  }
  /** Remainder elements whose currently selected source frame is bound. */
  remainderSelection(): { element_id: string; frame: number; tick: number }[] {
    const contract = this.contract;
    if (!contract) throw new Error("Native artwork preview is not loaded");
    const selected: { element_id: string; frame: number; tick: number }[] = [];
    for (const [id, frames] of this.remainders) {
      const element = contract.elements.find((e) => e.id === id)!;
      if (this.visibleElements.get(id) === false) continue;
      if (!(this.activeElements.get(id) ?? element.active)) continue;
      const tick = Math.max(0, this.currentTick + (this.offsets.get(id) ?? 0));
      const frame = nativePresentationFrame(element, tick);
      if (frame >= 0 && frames[frame]) selected.push({ element_id: id, frame, tick });
    }
    return selected;
  }
  coverArtworkBindings() {
    return this.contract?.cover_artwork?.bindings ?? [];
  }
  setCoverArtworkState(id: string, applied: boolean) {
    if (!this.contract?.cover_artwork?.bindings.some((b) => b.placement.patch_id === id))
      throw new Error("Unknown native cover artwork state");
    if (this.coverApplied.has(id) === applied) return false;
    if (applied) this.coverApplied.add(id);
    else this.coverApplied.delete(id);
    this.revision++;
    return true;
  }
  private currentBackground() {
    if (!this.contract) throw new Error("Native artwork preview is not loaded");
    const art = this.contract.cover_artwork;
    const resource = art
      ? art.variants.find(
          (v) =>
            v.applied.length === this.coverApplied.size &&
            v.applied.every((id) => this.coverApplied.has(id)),
        )!.image
      : this.contract.background;
    return this.images.get(resource.path)!;
  }
  /** Verify the actual hash-checked decoded frames before omitting invisible initial geometry. */
  assertTransparentPatchInitial(id: string) {
    const patch = this.contract?.patch_states?.find((p) => p.id === id);
    if (!patch?.initial.length) throw new Error("Missing bound initial patch frames");
    for (const frame of patch.initial) {
      const pixels = this.images.get(frame.path);
      if (!pixels) throw new Error("Missing decoded initial patch frame");
      for (let offset = 3; offset < pixels.data.length; offset += 4)
        if (pixels.data[offset] !== 0)
          throw new Error(`Initial patch frame is not transparent: ${frame.path}`);
    }
  }
  /** Preflight ownership and the loaded source before a delivery ledger consumes its sequence. */
  assertCanSampleStateClocks(batch: StateClockBatch) {
    if (!this.contract) throw new Error("Native artwork preview is not loaded");
    if (this.initialContext)
      throw new Error("Immutable source initial context cannot consume state events");
    if (this.externalClock) throw new Error("External source clock already owns playback");
    validateStateClockSamples(this.contract, batch);
  }
  setPlaying(value: boolean) {
    if (this.externalClock) throw new Error("External source clock owns playback");
    this.playing = value;
  }
  setElementState(id: string, active: boolean, tick = 0) {
    if (this.initialContext && this.contract?.elements.some((e) => e.id === id && !e.loop))
      throw new Error("Immutable source initial context cannot change element state");
    if (this.externalClock) throw new Error("External source clock owns element state");
    if (!this.contract?.elements.some((e) => e.id === id))
      throw new Error(`Unknown native preview element: ${id}`);
    if (typeof active !== "boolean" || !Number.isSafeInteger(tick) || tick < 0)
      throw new Error("Invalid native element state");
    this.activeElements.set(id, active);
    this.offsets.set(id, tick - this.currentTick);
    this.revision++;
  }
  setElementVisible(id: string, visible: boolean) {
    if (this.initialContext || this.externalClock)
      throw new Error("Source context or external clock owns element visibility");
    if (!this.contract?.elements.some((e) => e.id === id) || typeof visible !== "boolean")
      throw new Error("Invalid source element visibility");
    this.visibleElements.set(id, visible);
    this.revision++;
  }
  setBackgroundState(id: string, phase: NativeBackgroundPhase, tick = 0) {
    if (this.initialContext)
      throw new Error("Immutable source initial context cannot change background state");
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
    if (this.initialContext)
      throw new Error("Immutable source initial context cannot change patch state");
    const state = this.contract?.patch_states?.find((s) => s.id === id);
    if (!state) throw new Error(`Unknown native patch state: ${id}`);
    nativePatchBackgroundFrame(state, phase, tick);
    nativeTransientPatchFrame(state, phase, tick);
    this.patches.set(id, { phase, offset: tick - this.currentTick });
    this.revision++;
  }
  seek(tick: number, id?: string) {
    if (id && this.initialContext && this.contract?.elements.some((e) => e.id === id && !e.loop))
      throw new Error("Immutable source initial context cannot be sought");
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
    if (
      !this.initialContext &&
      (this.contract.patch_states?.length || this.contract.background_states?.length)
    )
      throw new Error("Loop clock snapshots cannot own patch timing");
    const byId = new Map(rows.map((r) => [r.id, r]));
    if (byId.size !== rows.length || rows.length !== this.contract.elements.length)
      throw new Error("External source clock inventory differs");
    for (const e of this.contract.elements) {
      const r = byId.get(e.id);
      if (
        this.initialContext &&
        !e.loop &&
        r &&
        (r.tick !== 0 || r.active !== e.active || r.playing || r.visible === false)
      )
        throw new Error("Immutable source initial context cannot consume a changed clock");
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
    const signature = JSON.stringify(
      rows.map((r) => [r.id, r.tick, r.active, r.playing, r.visible]),
    );
    if (signature === this.externalSignature) return false;
    this.externalClock = true;
    this.externalSignature = signature;
    this.currentTick = selected.tick;
    this.seconds = 0;
    this.playing = rows.some((r) => r.active && r.playing);
    for (const r of rows) {
      this.offsets.set(r.id, r.tick - this.currentTick);
      this.activeElements.set(r.id, r.active);
      this.visibleElements.set(r.id, r.visible !== false);
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
      (!this.initialContext &&
        (this.contract.patch_states?.length || this.contract.background_states?.length))
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
    if (
      !this.initialContext &&
      (contract.patch_states?.length || contract.background_states?.length)
    )
      throw new Error("Actor loop composition cannot infer patch or background state ownership");
    const draws: NativeLoopDrawSnapshot["draws"] = [];
    const orderOnly: NativeLoopDrawElement[] = [];
    for (const original of [...contract.elements, ...this.initialElements]) {
      if (this.visibleElements.get(original.id) === false) continue;
      const active = this.activeElements.get(original.id) ?? original.active;
      const element =
        !active && original.initial_frame
          ? { ...original, active: true, frames: [original.initial_frame], loop: false }
          : { ...original, active };
      const sourceTick =
        this.initialContext && !original.loop
          ? 0
          : Math.max(0, this.currentTick + (this.offsets.get(element.id) ?? 0));
      const frame = nativePresentationFrame(element, sourceTick);
      if (frame < 0) {
        if (element.active && element.frames.length === 0) orderOnly.push(element);
        continue;
      }
      const image = element.frames[frame]!;
      draws.push({
        element,
        frame,
        sourceTick,
        sourceActive: active,
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
      background: this.currentBackground(),
      orderOnly,
      draws,
    };
  }
  /** Compose explicitly substituted physical effect pixels with the same source order. */
  composeLoopSnapshot(snapshot: NativeLoopDrawSnapshot): NativePixels {
    if (
      !this.contract ||
      snapshot.mission !== this.contract.mission ||
      snapshot.revision !== this.revision
    )
      throw new Error("Retired physical loop snapshot");
    if (
      !this.initialContext &&
      (this.contract.patch_states?.length || this.contract.background_states?.length)
    )
      throw new Error("Physical loop substitution cannot own patch composition");
    const pixels = { ...snapshot.background, data: new Uint8Array(snapshot.background.data) };
    const byId = new Map(snapshot.draws.map((d) => [d.element.id, d]));
    const elements = [...snapshot.draws.map((d) => d.element), ...(snapshot.orderOnly ?? [])];
    const backgrounds = elements
      .filter((e) => e.active && this.backgroundEffectIds.has(e.id))
      .sort((a, b) => this.backgroundEffectIds.get(a.id)! - this.backgroundEffectIds.get(b.id)!);
    const ordered = nativePresentationOrder(
      elements.filter((e) => !this.backgroundEffectIds.has(e.id)),
      this.verifiedDisjointPair,
    );
    for (const element of [...backgrounds, ...ordered]) {
      const draw = byId.get(element.id);
      if (draw) compositeNativePixels(pixels, draw.pixels, draw.x, draw.y, draw.shadow);
    }
    return pixels;
  }
  /** Bound remainder frames draw their physical layer, then the kept native pixels on top. */
  remainderPixels(layers: NativeRemainderLayers): NativePixels {
    if (!this.contract) throw new Error("Native artwork preview is not loaded");
    if (!this.remainders.size) throw new Error("Native artwork has no reviewed remainder");
    const expected = this.remainderSelection().map((s) => s.element_id);
    if (
      layers.size !== expected.length ||
      expected.some((id) => {
        const layer = layers.get(id);
        return (
          !layer ||
          layer.width !== this.currentBackground().width ||
          layer.height !== this.currentBackground().height
        );
      })
    )
      throw new Error("Native remainder physical layers differ from bound frames");
    return this.compose(layers);
  }
  pixels(): NativePixels {
    if (!this.contract) throw new Error("Native artwork preview is not loaded");
    if (this.cached?.revision === this.revision) return this.cached.pixels;
    const pixels = this.compose();
    this.cached = { revision: this.revision, pixels };
    return pixels;
  }
  private compose(layers?: NativeRemainderLayers): NativePixels {
    if (!this.contract) throw new Error("Native artwork preview is not loaded");
    if (this.initialContext) return this.composeLoopSnapshot(this.loopDrawSnapshot());
    const background = this.currentBackground();
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
    const elements: Omit<NativePresentationElement, "source">[] = this.contract.elements
      .filter((e) => this.visibleElements.get(e.id) !== false)
      .map((e) => {
        const active = this.activeElements.get(e.id) ?? e.active;
        return !active && e.initial_frame
          ? { ...e, active: true, frames: [e.initial_frame], loop: false }
          : { ...e, active };
      });
    const backgroundIds = new Map(this.backgroundEffectIds);
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
      if (state.layer === "background") backgroundIds.set(state.id, state.creation_order);
      elements.push({ ...state, active: true, frames: [frame], loop: false });
    }
    // Refresh background FX in registration order before the ordinary display list.
    // Rebuilding from the clean receiver also removes the preceding frame's pixels.
    const backgroundEffects = elements
      .filter((e) => e.active && backgroundIds.has(e.id))
      .sort((a, b) => backgroundIds.get(a.id)! - backgroundIds.get(b.id)!);
    const ordered = nativePresentationOrder(
      elements.filter((e) => !backgroundIds.has(e.id)),
      this.verifiedDisjointPair,
    );
    for (const element of [...backgroundEffects, ...ordered]) {
      const index = nativePresentationFrame(
        element,
        Math.max(0, this.currentTick + (this.offsets.get(element.id) ?? 0)),
      );
      if (index < 0) continue;
      const frame = element.frames[index]!;
      const original = this.contract.elements.find((e) => e.id === element.id);
      const remainder =
        layers && original?.frames === element.frames
          ? this.remainders.get(element.id)?.[index]
          : undefined;
      if (remainder) {
        compositeNativePixels(pixels, layers!.get(element.id)!, 0, 0);
        compositeNativePixels(
          pixels,
          remainder,
          element.display_position[0] + frame.offset[0] - this.contract.origin[0],
          element.display_position[1] + frame.offset[1] - this.contract.origin[1],
          frame.shadow_key,
        );
        continue;
      }
      compositeNativePixels(
        pixels,
        this.images.get(frame.path)!,
        element.display_position[0] + frame.offset[0] - this.contract.origin[0],
        element.display_position[1] + frame.offset[1] - this.contract.origin[1],
        frame.shadow_key,
      );
    }
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
