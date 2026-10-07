import type * as THREE from "three";
import { bindInitialActorSource, queryActorMasks } from "./native-actor-source.ts";
import { NativeActorComposition, type CurrentActorSnapshot } from "./native-actor-composition.ts";
import type { MissionSpritePreview } from "./mission.ts";
import type {
  NativeLoopDrawSnapshot,
  NativeStatePresentation,
} from "./native-state-presentation.ts";
import type { MissionStateSource } from "./mission-state-layer.ts";

export interface EditorActorBinding {
  identity: string;
  preview:
    | { kind: "editable"; editorId: string }
    | { kind: "source"; family: string; index: number };
  active: boolean;
  layer: number;
  drawHidden: boolean;
  outlineColor: number;
  displayOrderOverride?: number;
}
export interface EditorActorFrames {
  editable: readonly { editorId: string; sprite: MissionSpritePreview }[];
  source: readonly MissionSpritePreview[];
}
/** Explicit preview bindings; source stream identity and edited placement remain separate. */
export class EditorActorPreview {
  private readonly composition: NativeActorComposition;
  private readonly binding: Awaited<ReturnType<typeof bindInitialActorSource>>;
  private readonly actors: EditorActorBinding[];
  private readonly elevation: number;
  private membership: boolean[];
  private revision = 0;
  private cached:
    | { signature: string; result: ReturnType<NativeActorComposition["compose"]> }
    | undefined;
  private disposed = false;
  private constructor(
    binding: Awaited<ReturnType<typeof bindInitialActorSource>>,
    actors: readonly EditorActorBinding[],
    elevation: number,
  ) {
    this.binding = binding;
    this.actors = structuredClone(actors) as EditorActorBinding[];
    this.elevation = elevation;
    this.membership = [...binding.membership];
    const ids = new Set<string>();
    for (const actor of this.actors) {
      if (
        ids.has(actor.identity) ||
        !binding.records.has(actor.identity) ||
        !/^(soldiers|civilians|pcs_to_rescue):\d+$/.test(actor.identity) ||
        !Number.isInteger(actor.layer) ||
        typeof actor.active !== "boolean" ||
        typeof actor.drawHidden !== "boolean" ||
        !Number.isInteger(actor.outlineColor) ||
        actor.outlineColor < 0 ||
        actor.outlineColor > 65535 ||
        [0x7c0, 31].includes(actor.outlineColor) ||
        (actor.displayOrderOverride !== undefined && !Number.isFinite(actor.displayOrderOverride))
      )
        throw new Error(`Unsupported or duplicate current actor binding: ${actor.identity}`);
      ids.add(actor.identity);
      if (
        actor.preview.kind === "source" &&
        actor.identity !== `${actor.preview.family}:${actor.preview.index}`
      )
        throw new Error("Actor source locator and identity differ");
      if (
        actor.preview.kind === "editable" &&
        (!actor.identity.startsWith("soldiers:") ||
          actor.preview.editorId !== `import-soldier-${actor.identity.split(":")[1]}`)
      )
        throw new Error("Editable source actor identity has no verified import binding");
    }
    this.composition = new NativeActorComposition(binding.authority);
  }
  static async bind(
    source: MissionStateSource,
    native: NativeStatePresentation,
    epoch: number,
    actors: readonly EditorActorBinding[],
  ) {
    const binding = await bindInitialActorSource(source, native.loopSourceBinding(), epoch);
    return new EditorActorPreview(binding, actors, (source.camera.elevation_deg * Math.PI) / 180);
  }
  setMaskMembership(active: readonly boolean[]) {
    if (this.disposed) throw new Error("Actor preview is disposed");
    if (
      active.length !== this.membership.length ||
      active.some((value) => typeof value !== "boolean")
    )
      throw new Error("Invalid current mask membership");
    this.membership = [...active];
    this.revision++;
  }
  snapshot(tick: number, frames: EditorActorFrames): CurrentActorSnapshot {
    if (this.disposed) throw new Error("Actor preview is disposed");
    const sin = Math.sin(this.elevation),
      cos = Math.cos(this.elevation);
    return {
      epoch: this.binding.authority.epoch,
      mission: this.binding.authority.mission,
      tick,
      actors: this.actors.map((actor) => {
        const locator = actor.preview;
        const matches =
          locator.kind === "editable"
            ? frames.editable
                .filter((row) => row.editorId === locator.editorId)
                .map((row) => row.sprite)
            : frames.source.filter(
                (sprite) =>
                  sprite.sourceMember?.family === locator.family &&
                  sprite.sourceMember.index === locator.index,
              );
        if (matches.length !== 1)
          throw new Error(`Current actor resource is missing or ambiguous: ${actor.identity}`);
        const sprite = matches[0]!,
          [x, sceneHeight, sceneDepth] = sprite.position;
        if (
          !sprite.position.every(Number.isFinite) ||
          !Number.isInteger(sprite.direction) ||
          sprite.direction < 0 ||
          sprite.direction > 15
        )
          throw new Error("Invalid current actor placement/direction");
        const mapPosition: [number, number] = [x, sceneDepth * sin - sceneHeight * cos];
        const origin: [number, number] = [
          Math.floor(x + sprite.bounds.left),
          Math.floor(mapPosition[1] - sprite.bounds.top),
        ];
        const box: [number, number, number, number] = [
          ...origin,
          origin[0] + sprite.bounds.width,
          origin[1] + sprite.bounds.height,
        ];
        return {
          identity: actor.identity,
          active: actor.active,
          sprite,
          displayOrder: actor.displayOrderOverride ?? Math.fround(sceneDepth * sin),
          masks: queryActorMasks(
            this.binding.masks,
            this.membership,
            { layer: actor.layer, mapPosition },
            box,
            this.binding.gridSize,
          ),
          maskQuery: {
            layer: actor.layer,
            mapPosition,
            screenOrigin: origin,
            drawHidden: actor.drawHidden,
            outlineColor: actor.outlineColor,
            depth: 16 as const,
            shadowKey: 31,
            shadowStrength: 40,
          },
        };
      }),
    };
  }
  compose(
    renderer: THREE.WebGLRenderer,
    loop: NativeLoopDrawSnapshot,
    tick: number,
    frames: EditorActorFrames,
  ) {
    const snapshot = this.snapshot(tick, frames);
    const signature = JSON.stringify([
      loop.revision,
      this.revision,
      snapshot.actors.map((actor) => [
        actor.identity,
        actor.active,
        actor.sprite.visible,
        actor.sprite.position,
        actor.sprite.direction,
        actor.sprite.frame.resourceId,
        actor.sprite.frame.filename,
        actor.sprite.frame.profile,
        actor.sprite.frame.action,
        actor.sprite.frame.direction,
        actor.sprite.frame.frame,
        actor.sprite.bounds,
        actor.displayOrder,
      ]),
    ]);
    if (this.cached?.signature === signature) return this.cached.result;
    const result = this.composition.compose(renderer, loop, snapshot);
    this.cached = { signature, result };
    return result;
  }
  dispose() {
    if (!this.disposed) {
      this.disposed = true;
      this.cached = undefined;
      this.composition.dispose();
    }
  }
}
