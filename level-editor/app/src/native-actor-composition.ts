import type * as THREE from "three";
import { NativePixelCompositor } from "./native-pixel-compositor.ts";
import {
  prepareActorPixels,
  type ActorMaskSnapshot,
  type ActorMaskQuery,
} from "./native-actor-pixels.ts";
import type { NativeLoopDrawSnapshot, NativePixels } from "./native-state-presentation.ts";
import type { MissionSpritePreview } from "./mission.ts";
import type { NativeShadowKey } from "../../shared/src/native-state-presentation.ts";
import { mergeNativeDisplay, type NativeOrderedDraw } from "./native-display-merge.ts";

export interface CurrentActorDraw {
  identity: string;
  active: boolean;
  sprite: MissionSpritePreview;
  maskQuery: ActorMaskQuery;
  masks: readonly ActorMaskSnapshot[];
  /** Current preview order, including an explicitly supplied relative override. */
  displayOrder: number;
}
export interface ActorCompositionAuthority {
  epoch: number;
  mission: string;
  /** Full verified construction inventory, never local contract subset ranks. */
  creationRanks: ReadonlyMap<string, number>;
  backgroundEffects: ReadonlySet<string>;
}
export interface CurrentActorSnapshot {
  epoch: number;
  mission: string;
  tick: number;
  actors: readonly CurrentActorDraw[];
}
type Draw = { pixels: NativePixels; x: number; y: number; shadow?: NativeShadowKey };

/** Explicit preview snapshots only: this class owns neither AI nor an animation clock. */
export class NativeActorComposition {
  private readonly gpu = new NativePixelCompositor();
  private readonly authority: ActorCompositionAuthority;
  private disposed = false;
  constructor(authority: ActorCompositionAuthority) {
    if (!Number.isSafeInteger(authority.epoch) || authority.epoch < 0 || !authority.mission)
      throw new Error("Invalid actor source authority");
    const ranks = [...authority.creationRanks.values()];
    if (
      ranks.some((n) => !Number.isSafeInteger(n) || n < 0) ||
      new Set(ranks).size !== ranks.length
    )
      throw new Error("Full construction inventory has missing or duplicate ranks");
    this.authority = {
      ...authority,
      creationRanks: new Map(authority.creationRanks),
      backgroundEffects: new Set(authority.backgroundEffects),
    };
  }
  prepare(loop: NativeLoopDrawSnapshot, snapshot: CurrentActorSnapshot) {
    if (this.disposed) throw new Error("Actor composition is disposed");
    if (
      snapshot.epoch !== this.authority.epoch ||
      snapshot.mission !== this.authority.mission ||
      loop.mission !== snapshot.mission ||
      !Number.isSafeInteger(snapshot.tick) ||
      snapshot.tick < 0
    )
      throw new Error("Actor preview snapshot belongs to a retired or different source");
    const background: { identity: string; rank: number; draw: Draw }[] = [];
    const ordered: (NativeOrderedDraw & { draw: Draw | null })[] = [];
    const ids = new Set<string>();
    const rank = (id: string) => {
      if (ids.has(id)) throw new Error(`Duplicate dynamic preview identity: ${id}`);
      ids.add(id);
      const result = this.authority.creationRanks.get(id);
      if (result === undefined) throw new Error(`Unresolved full construction rank: ${id}`);
      return result;
    };
    for (const row of loop.draws) {
      const identity = `${row.element.source.kind}:${row.element.source.index}`,
        creation = rank(identity);
      const draw = { pixels: row.pixels, x: row.x, y: row.y, shadow: row.shadow };
      if (this.authority.backgroundEffects.has(identity))
        background.push({ identity, rank: creation, draw });
      else {
        if (!Number.isFinite(Math.fround(row.element.display_order)))
          throw new Error("Invalid effect order");
        ordered.push({
          identity,
          rank: creation,
          order: Math.fround(row.element.display_order),
          mapPosition: row.element.sort_position,
          polyline: row.element.polyline,
          draw,
        });
      }
    }
    for (const element of loop.orderOnly ?? []) {
      if (!element.active || element.frames.length !== 0)
        throw new Error("Order-only effect must be active with explicitly empty artwork");
      const identity = `${element.source.kind}:${element.source.index}`;
      ordered.push({
        identity,
        rank: rank(identity),
        order: element.display_order,
        mapPosition: element.sort_position,
        polyline: element.polyline,
        draw: null,
      });
    }
    const actorReceipts: {
      identity: string;
      applied: ReturnType<typeof prepareActorPixels>["applied"];
      shadowPixels: number;
    }[] = [];
    for (const actor of snapshot.actors) {
      const creation = rank(actor.identity);
      if (typeof actor.active !== "boolean" || !Number.isFinite(Math.fround(actor.displayOrder)))
        throw new Error("Invalid current actor state");
      if (!actor.active || !actor.sprite.visible) continue;
      if (!actor.sprite.frame.legacy)
        throw new Error("Actor masking requires an untouched legacy source frame");
      const prepared = prepareActorPixels(actor.sprite.frame.pixels, actor.maskQuery, actor.masks);
      ordered.push({
        identity: actor.identity,
        rank: creation,
        order: Math.fround(actor.displayOrder),
        mapPosition: actor.maskQuery.mapPosition,
        polyline: [],
        draw: {
          pixels: prepared.pixels,
          x: actor.maskQuery.screenOrigin[0] - loop.origin[0],
          y: actor.maskQuery.screenOrigin[1] - loop.origin[1],
          shadow: prepared.shadow,
        },
      });
      actorReceipts.push({
        identity: actor.identity,
        applied: prepared.applied,
        shadowPixels: prepared.shadowPixels,
      });
    }
    background.sort((a, b) => a.rank - b.rank);
    const merged = mergeNativeDisplay(ordered);
    return {
      background: loop.background,
      draws: [...background, ...merged.rows].flatMap((row) => (row.draw ? [row.draw] : [])),
      identities: [...background, ...merged.rows].map((row) => row.identity),
      tieProof: merged.tieProof,
      actors: actorReceipts,
    };
  }
  compose(
    renderer: THREE.WebGLRenderer,
    loop: NativeLoopDrawSnapshot,
    snapshot: CurrentActorSnapshot,
  ) {
    const prepared = this.prepare(loop, snapshot);
    return {
      pixels: this.gpu.compose(renderer, prepared.background, prepared.draws),
      identities: prepared.identities,
      actors: prepared.actors,
      tieProof: prepared.tieProof,
    };
  }
  dispose() {
    if (!this.disposed) {
      this.disposed = true;
      this.gpu.dispose();
    }
  }
}
