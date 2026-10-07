import { verifyNativePresentationSource } from "./native-state-presentation.ts";
import type { MissionStateSource } from "./mission-state-layer.ts";
import {
  nativePresentationOrder,
  nativePresentationFrame,
  type NativeStatePresentationContract,
  type NativePresentationElement,
} from "../../shared/src/native-state-presentation.ts";

export interface EffectDraw {
  element: NativePresentationElement;
  phase: number;
  tick: number;
  stage: "background" | "ordered";
  masking: "off";
  composition: "isolated-internal-depth-source-over";
  requiresDestinationKeying: boolean;
}
export interface EffectSchedule {
  background: EffectDraw[];
  ordered: EffectDraw[];
}
/** Native source effects only; actors require their own validated masking policy. */
export class NativeEffectComposition {
  private contract: NativeStatePresentationContract;
  private backgroundIds: ReadonlySet<string>;
  private keyedIds: ReadonlySet<string>;
  private constructor(
    contract: NativeStatePresentationContract,
    backgroundIds: ReadonlySet<string>,
    keyedIds: ReadonlySet<string>,
  ) {
    this.contract = contract;
    this.backgroundIds = backgroundIds;
    this.keyedIds = keyedIds;
  }
  static async bind(contract: NativeStatePresentationContract, source: MissionStateSource) {
    const c = structuredClone(contract),
      s = structuredClone(source);
    await verifyNativePresentationSource(c, s);
    if (c.patch_states?.length || c.background_states?.length)
      throw new Error("State patch/background ownership requires an explicit additional binding");
    const backgrounds = new Set<string>(),
      keyed = new Set<string>();
    for (const e of c.elements)
      if (e.source.kind === "map-animation") {
        const raw = s.level.animations[e.source.index]!;
        if (raw.blit_type !== 0 || e.frames.some((f) => f.shadow_key)) keyed.add(e.id);
        if (!Number.isFinite(raw.sprite.elevation))
          throw new Error(`Missing source effect elevation: ${e.id}`);
        if (raw.sprite.elevation === 0) backgrounds.add(e.id);
      }
    return new NativeEffectComposition(c, backgrounds, keyed);
  }
  schedule(ticks: ReadonlyMap<string, number>): EffectSchedule {
    const selected = (e: NativePresentationElement): EffectDraw[] => {
      if (!e.active || !e.frames.length) return [];
      const tick = ticks.get(e.id);
      if (tick === undefined || !Number.isSafeInteger(tick) || tick < 0)
        throw new Error(`Missing authoritative effect clock: ${e.id}`);
      const phase = nativePresentationFrame(e, tick);
      return phase < 0
        ? []
        : [
            {
              element: structuredClone(e),
              phase,
              tick,
              stage: this.backgroundIds.has(e.id) ? "background" : "ordered",
              masking: "off",
              composition: "isolated-internal-depth-source-over",
              requiresDestinationKeying: this.keyedIds.has(e.id),
            },
          ];
    };
    // Background registration follows validated source insertion order, not display Y.
    const backgrounds = this.contract.elements
      .filter((e) => this.backgroundIds.has(e.id))
      .sort((a, b) => a.creation_order - b.creation_order);
    const ordered = nativePresentationOrder(
      this.contract.elements.filter((e) => !this.backgroundIds.has(e.id)),
    );
    return { background: backgrounds.flatMap(selected), ordered: ordered.flatMap(selected) };
  }
}

/** Renderer port: isolated draw keeps the effect's own depth; scene depth is never globally cleared. */
export interface EffectCompositionBackend<T> {
  ordinaryDepth(): void;
  supportsDestinationKeying(draw: EffectDraw): boolean;
  /** A throwing preparation releases its own incomplete allocation. */
  prepare(draw: EffectDraw): T;
  releasePrepared(draw: EffectDraw, prepared: T): void;
  capture(): unknown;
  staticSceneWithoutBoundEffects(): void;
  compositeBackground(draw: EffectDraw, prepared: T): void;
  compositeOrdered(draw: EffectDraw, prepared: T): void;
  restore(state: unknown): void;
}
/** Backend owns scratch resources; this transaction never changes models, materials or clocks. */
export function executeEffectComposition<T>(
  exactNativeCamera: boolean,
  schedule: () => EffectSchedule,
  backend: EffectCompositionBackend<T>,
) {
  if (!exactNativeCamera) {
    backend.ordinaryDepth();
    return "ordinary-depth" as const;
  }
  const plan = schedule();
  const rows = [...plan.background, ...plan.ordered];
  for (const draw of rows)
    if (draw.requiresDestinationKeying && !backend.supportsDestinationKeying(draw))
      throw new Error(`Destination keying not proven for ${draw.element.id}`);
  const prepared = new Map<EffectDraw, T>();
  const saved = backend.capture();
  let failed = false,
    primaryError: unknown;
  const cleanupFailures: unknown[] = [];
  try {
    for (const draw of rows) prepared.set(draw, backend.prepare(draw));
    backend.staticSceneWithoutBoundEffects();
    for (const draw of plan.background) backend.compositeBackground(draw, prepared.get(draw)!);
    for (const draw of plan.ordered) backend.compositeOrdered(draw, prepared.get(draw)!);
  } catch (error) {
    failed = true;
    primaryError = error;
  } finally {
    for (const [draw, value] of prepared) {
      try {
        backend.releasePrepared(draw, value);
      } catch (error) {
        cleanupFailures.push(error);
      }
    }
    try {
      backend.restore(saved);
    } catch (error) {
      cleanupFailures.push(error);
    }
  }
  if (cleanupFailures.length)
    throw new AggregateError(
      failed ? [primaryError, ...cleanupFailures] : cleanupFailures,
      "Effect transaction cleanup failed",
    );
  if (failed) throw primaryError;
  return "native-source-order" as const;
}
