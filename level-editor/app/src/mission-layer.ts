import * as THREE from "three";
import { gameToScene, type Level3D } from "@rle/shared";
import { disposeObjectResources } from "./resources.ts";
import { MissionEntities, type MissionSpritePreview } from "./mission.ts";
import type { PhysicalReceiverTriangle } from "./actor-shadow-receivers.ts";
import type { MissionCharacterProfile } from "./mission-character-catalog.ts";

interface Actor {
  key: string;
  ring: THREE.Mesh<THREE.RingGeometry, THREE.MeshBasicMaterial>;
  view?: MissionEntities;
  pending: boolean;
  warning?: string;
}

/** Character previews and selection outlines never enter the baked map artwork. */
export class MissionLayer {
  readonly root = new THREE.Group();
  readonly spritesRoot = new THREE.Group();
  private library: FileSystemDirectoryHandle | null = null;
  private profiles: readonly MissionCharacterProfile[] = [];
  private status: (loading: boolean, warnings: string[]) => void = () => {};
  private actors = new Map<string, Actor>();
  private document: Level3D | null = null;
  private selected = "";
  private epoch = 0;

  setLibrary(
    root: FileSystemDirectoryHandle | null,
    profiles: readonly MissionCharacterProfile[],
    status: (loading: boolean, warnings: string[]) => void,
  ) {
    this.status = status;
    if (this.library === root && this.profiles === profiles) return;
    const document = this.document,
      selected = this.selected;
    this.clear();
    this.library = root;
    this.profiles = profiles;
    if (document) this.sync(document, selected);
  }

  setVisible(visible: boolean) {
    this.root.visible = visible;
    this.spritesRoot.visible = visible;
  }

  private report() {
    const actors = [...this.actors.values()];
    this.status(
      actors.some((actor) => actor.pending),
      actors.flatMap((actor) => (actor.warning ? [actor.warning] : [])),
    );
  }

  clear() {
    this.epoch++;
    for (const actor of this.actors.values()) actor.view?.dispose();
    this.actors.clear();
    disposeObjectResources([this.root]);
    this.root.clear();
    this.spritesRoot.clear();
    this.document = null;
    this.report();
  }

  sync(document: Level3D, selected = "") {
    this.document = document;
    this.selected = selected;
    const entries = [
      ...(document.mission?.spawnPoints ?? []).map((entry) => ({ ...entry, kind: "pc" as const })),
      ...(document.mission?.soldiers ?? []).map((entry) => ({ ...entry, kind: "npc" as const })),
    ];
    const live = new Set(entries.map((entry) => entry.id));
    for (const [id, actor] of this.actors) {
      if (live.has(id)) continue;
      actor.view?.dispose();
      disposeObjectResources([actor.ring]);
      actor.ring.removeFromParent();
      this.actors.delete(id);
    }
    for (const entry of entries) {
      const key = JSON.stringify([entry.kind, entry.profile, document.camera]);
      let actor = this.actors.get(entry.id);
      if (actor && actor.key !== key) {
        actor.view?.dispose();
        disposeObjectResources([actor.ring]);
        actor.ring.removeFromParent();
        this.actors.delete(entry.id);
        actor = undefined;
      }
      if (!actor) {
        const ring = new THREE.Mesh(
          new THREE.RingGeometry(11, 14, 24),
          new THREE.MeshBasicMaterial({ depthTest: false, side: THREE.DoubleSide }),
        );
        ring.userData.missionId = entry.id;
        ring.renderOrder = 1100;
        actor = { key, ring, pending: false };
        this.actors.set(entry.id, actor);
        this.root.add(ring);
        const profile = this.profiles.find(
          (profile) => profile.kind === entry.kind && profile.profile === entry.profile,
        );
        if (entry.kind === "pc" && entry.profile === undefined) {
          // Campaign slots intentionally have no fixed character sprite.
        } else if (!this.library || !profile) {
          actor.warning = `${entry.name}: character sprite unavailable (${!this.library ? "open a library with game-data" : `unknown ${entry.kind.toUpperCase()} profile ${entry.profile}`}).`;
        } else {
          actor.pending = true;
          const own = actor,
            epoch = this.epoch;
          const current = () => epoch === this.epoch && this.actors.get(entry.id) === own;
          void MissionEntities.loadCharacter(this.library, profile, document.camera, current).then(
            (view) => {
              if (!current()) {
                view.dispose();
                return;
              }
              own.view = view;
              own.pending = false;
              own.warning = view.warnings.length
                ? `${entry.name}: ${view.warnings.join("; ")}`
                : undefined;
              view.root.userData.missionId = entry.id;
              this.spritesRoot.add(view.root);
              if (this.document) this.sync(this.document, this.selected);
            },
            (error) => {
              if (!current()) return;
              own.pending = false;
              own.warning = `${entry.name}: character sprite unavailable: ${String(error)}`;
              if (this.document) this.sync(this.document, this.selected);
            },
          );
        }
      }
      const [x, y, z] = gameToScene(document.camera, ...entry.position);
      actor.ring.position.set(x, y, z + 1);
      actor.ring.material.color.setHex(
        entry.id === selected
          ? 0xffdd55
          : actor.warning
            ? 0xff45ce
            : entry.kind === "pc"
              ? 0x55bbff
              : 0xff6655,
      );
      actor.view?.setCharacterPose(new THREE.Vector3(x, z, -y), entry.direction);
    }
    this.report();
  }

  update(camera: THREE.Camera, lockOrientations: boolean) {
    if (!this.spritesRoot.visible) return;
    for (const actor of this.actors.values()) actor.view?.update(camera, lockOrientations);
  }

  /** Editable identity stays separate from any verified source-member binding. */
  previewSprites(
    nativeDirection = false,
  ): { editorId: string; epoch: number; sprite: MissionSpritePreview }[] {
    return [...this.actors].flatMap(([editorId, actor]) =>
      (actor.view?.previewSprites(nativeDirection) ?? []).map((sprite) => ({
        editorId,
        epoch: this.epoch,
        sprite: { ...sprite, visible: this.spritesRoot.visible && sprite.visible },
      })),
    );
  }

  bindCharacterReceivers(
    editorId: string,
    revision: string,
    triangles: readonly PhysicalReceiverTriangle[],
    elevation: number,
  ) {
    const view = this.actors.get(editorId)?.view;
    if (!view) throw new Error("Character preview is not ready");
    view.bindCharacterReceivers(editorId, revision, triangles, elevation);
  }
  clearCharacterReceivers(editorId: string) {
    this.actors.get(editorId)?.view?.clearCharacterReceivers();
  }
  characterReceiverStatus(editorId: string) {
    return this.actors.get(editorId)?.view?.characterReceiverStatus();
  }

  hit(raycaster: THREE.Raycaster): string | undefined {
    if (!this.root.visible || !this.spritesRoot.visible) return undefined;
    const hits = raycaster.intersectObjects([this.root, this.spritesRoot], true);
    for (const hit of hits) {
      let object: THREE.Object3D | null = hit.object;
      while (object) {
        if (typeof object.userData.missionId === "string") return object.userData.missionId;
        object = object.parent;
      }
    }
    return undefined;
  }
}
