import * as THREE from "three";
import { gameToScene, type MapCamera, type ProtoLevel } from "@rle/shared";
import {
  validateMissionStateContract,
  type MissionStateContract,
} from "../../shared/src/mission-state.ts";
import { SceneAssetLoader, captureLoadedStateAppearance } from "./scene-assets.ts";
import type { SourceContractClockBinding } from "./source-contract-clock-binding.ts";
import { StateAppearancePlayer } from "./state-appearance-player.ts";
import { placementHeight } from "./entity-projection.ts";
import { disposeObjectResources } from "./resources.ts";

export interface MissionStateSource {
  name: string;
  data: Record<string, unknown>;
  level: ProtoLevel;
  camera: MapCamera;
}
export async function missionStateDataHash(value: unknown): Promise<string> {
  const bytes = new TextEncoder().encode(JSON.stringify(value));
  const hash = await crypto.subtle.digest("SHA-256", bytes);
  return Array.from(new Uint8Array(hash), (n) => n.toString(16).padStart(2, "0")).join("");
}

export async function resolveMissionStateTargets(
  contract: MissionStateContract,
  source: MissionStateSource,
) {
  validateMissionStateContract(contract);
  if (
    source.name !== contract.mission ||
    source.camera.elevation_deg !== contract.camera_elevation_deg ||
    (await missionStateDataHash(source.data)) !== contract.mission_data_sha256 ||
    (await missionStateDataHash(source.level)) !== contract.level_data_sha256
  )
    throw new Error("Mission state source or camera does not match pinned contract");
  const targets = source.data.targets;
  if (!Array.isArray(targets)) throw new Error("Mission state source has no target array");
  const finite = (v: unknown, label: string): number => {
    if (typeof v !== "number" || !Number.isFinite(v))
      throw new Error(`Mission state invalid ${label}`);
    return v;
  };
  return Promise.all(
    contract.targets.map(async (binding) => {
      const raw = targets[binding.target_index];
      if (
        !raw ||
        typeof raw !== "object" ||
        Array.isArray(raw) ||
        (await missionStateDataHash(raw)) !== binding.target_sha256
      )
        throw new Error(`Mission state target changed: ${binding.id}`);
      const target = raw as Record<string, unknown>;
      const x = finite(target.position_x, "target x"),
        y = finite(target.position_y, "target y");
      let z = finite(target.position_z, "target z");
      const obstacleIndex = finite(target.obstacle_index, "target support");
      if (!Number.isInteger(obstacleIndex) || obstacleIndex < 0 || obstacleIndex > 65535)
        throw new Error("Mission state invalid support index");
      if (z < 0) {
        if (obstacleIndex === 65535) z = 0;
        else {
          const obstacle = source.level.sight_obstacles[obstacleIndex];
          if (!obstacle) throw new Error(`Mission state missing support ${obstacleIndex}`);
          z = placementHeight(x, y, obstacle);
        }
      }
      const action = finite(target.action, "initial action");
      if (!binding.actions.some((a) => a.action === action))
        throw new Error(`Mission state unbound initial action: ${binding.id}`);
      const actionPosition = [
        finite(target.action_position_x, "action x"),
        finite(target.action_position_y, "action y"),
      ];
      const [sx, sy, sz] = gameToScene(source.camera, x, y + z, z);
      return {
        binding,
        target,
        action,
        physicalPositionGame: [x, y + z, z],
        displayPosition: [x, y],
        actionPosition,
        position: new THREE.Vector3(sx, sz, -sy).sub(new THREE.Vector3(...binding.model_origin)),
      };
    }),
  );
}

type PlayerEntry = {
  player: StateAppearancePlayer;
  binding: MissionStateContract["targets"][number];
};
/** Mission-scoped preview instances. The viewport owns the only elapsed-time clock. */
export class MissionStateLayer {
  readonly root = new THREE.Group();
  readonly players = new Map<string, PlayerEntry>();
  private epoch = 0;
  private retired: (() => void) | undefined;
  private disposed = false;
  private clocks: SourceContractClockBinding | undefined;
  private replaced = new Set<number>();
  constructor(
    privateChanged: (targets: ReadonlySet<number>) => void = () => {},
    privateError: (message: string) => void = () => {},
    privateLoader: (
      root: FileSystemDirectoryHandle,
    ) => Pick<SceneAssetLoader, "load" | "dispose"> = (root) => new SceneAssetLoader(root),
  ) {
    this.changed = privateChanged;
    this.error = privateError;
    this.loader = privateLoader;
  }
  private readonly changed: (targets: ReadonlySet<number>) => void;
  private readonly error: (message: string) => void;
  private readonly loader: (
    root: FileSystemDirectoryHandle,
  ) => Pick<SceneAssetLoader, "load" | "dispose">;
  get replacedTargets(): ReadonlySet<number> {
    return this.replaced;
  }
  clear() {
    this.epoch++;
    for (const [id, { player }] of this.players) {
      this.clocks?.detachPhysical(id);
      player.dispose();
    }
    this.clocks = undefined;
    this.players.clear();
    this.root.clear();
    this.replaced = new Set();
    this.changed(this.replaced);
    this.retired?.();
    this.retired = undefined;
  }
  async set(
    contract: MissionStateContract,
    library: FileSystemDirectoryHandle,
    source: MissionStateSource,
    clocks?: SourceContractClockBinding,
  ): Promise<void> {
    if (this.disposed) throw new Error("Mission state layer is disposed");
    this.clear();
    this.clocks = clocks;
    const epoch = this.epoch;
    const current = () => !this.disposed && epoch === this.epoch;
    const frozen = structuredClone(contract),
      frozenSource = structuredClone(source);
    let resolved: Awaited<ReturnType<typeof resolveMissionStateTargets>>;
    try {
      resolved = await resolveMissionStateTargets(frozen, frozenSource);
    } catch (error) {
      if (current()) this.error(String(error));
      throw error;
    }
    if (!current()) return;
    const loader = this.loader(library),
      owned: THREE.Object3D[] = [];
    let pending = true,
      retired = false,
      released = false;
    const release = () => {
      if (!retired || pending || released) return;
      released = true;
      disposeObjectResources(owned);
      loader.dispose();
    };
    this.retired = () => {
      retired = true;
      release();
    };
    try {
      await Promise.all(
        resolved.map(async (row) => {
          const token = clocks?.physicalToken(row.binding.id);
          let player: StateAppearancePlayer | undefined;
          try {
            const asset = await loader.load(row.binding.source);
            owned.push(asset);
            if (!current()) return;
            const template = captureLoadedStateAppearance(asset);
            if (!template) throw new Error("Pinned state model has no animation clips");
            player = new StateAppearancePlayer(template, !!clocks);
            // Validate every declared action before replacing the existing source target.
            for (const action of row.binding.actions) player.select(action.clip, action.timing);
            const action = row.binding.actions.find((a) => a.action === row.action)!;
            player.select(action.clip, action.timing);
            player.object.position.copy(row.position);
            player.object.userData = {
              mission: frozen.mission,
              targetIndex: row.binding.target_index,
              sourceTarget: row.target,
              physicalPositionGame: row.physicalPositionGame,
              displayPosition: row.displayPosition,
              actionPosition: row.actionPosition,
              representation: row.binding.representation,
            };
            if (clocks && token) {
              const consumer = player;
              clocks.attachPhysical(
                row.binding.id,
                {
                  internallyPlaying: () => consumer.playing,
                  sampleExternalTick: (tick) => consumer.sampleExternalTick(tick),
                },
                token,
              );
            }
            this.players.set(row.binding.id, { player, binding: row.binding });
            this.root.add(player.object);
            this.replaced.add(row.binding.target_index);
            this.changed(new Set(this.replaced));
          } catch (error) {
            player?.dispose();
            if (current()) this.error(`${row.binding.id}: ${String(error)}`);
          }
        }),
      );
    } finally {
      pending = false;
      release();
    }
  }
  selectAction(id: string, action: number) {
    const entry = this.players.get(id);
    if (!entry) throw new Error(`Mission state missing instance ${id}`);
    const binding = entry.binding.actions.find((a) => a.action === action);
    if (!binding) throw new Error(`Mission state missing action ${action}`);
    entry.player.select(binding.clip, binding.timing);
    this.clocks?.seekPhysical(id, 0);
  }
  seek(id: string, tick: number) {
    const entry = this.players.get(id);
    if (!entry) throw new Error(`Mission state missing instance ${id}`);
    if (this.clocks) this.clocks.seekPhysical(id, tick);
    else {
      entry.player.pause();
      entry.player.seek(tick);
    }
  }
  setPlaying(playing: boolean) {
    for (const [id, { player }] of this.players) {
      if (this.clocks) {
        this.clocks.setPhysicalPlaying(id, playing);
        continue;
      }
      if (playing) player.play();
      else player.pause();
    }
  }
  advance(seconds: number) {
    let changed = false;
    if (this.root.visible)
      for (const { player } of this.players.values()) {
        if (player.externallyClocked) continue;
        const tick = player.tick;
        player.advance(seconds);
        changed ||= tick !== player.tick;
      }
    return changed;
  }
  dispose() {
    if (this.disposed) return;
    this.clear();
    this.root.removeFromParent();
    this.disposed = true;
  }
}
