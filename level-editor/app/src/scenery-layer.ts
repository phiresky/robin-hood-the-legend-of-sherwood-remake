import * as THREE from "three";
import { gameToScene, partMatrix, sceneToGame, type Level3D } from "@rle/shared";
import type { GameplayAssetDescriptor } from "../../shared/src/asset-gameplay.ts";
import { compileSceneryAnimation } from "../../shared/src/compile-scenery-animation.ts";
import { libraryFile, readPinnedAssetDescriptors } from "./projection-library.ts";
import { loadSceneryFrames, sceneryFrameAtTick } from "./scenery-frames.ts";

type Frames = Awaited<ReturnType<typeof loadSceneryFrames>>;
interface Effect {
  mesh: THREE.Mesh<THREE.BufferGeometry, THREE.MeshBasicMaterial>;
  key: string;
  start: number;
  frames?: Frames;
  place?: () => void;
}

/** Borrowed map placements; owned preview resources stay outside the bake roots. */
export class SceneryLayer {
  readonly root = new THREE.Group();
  private library: FileSystemDirectoryHandle | null = null;
  private document: Level3D | null = null;
  private descriptors = new Map<string, GameplayAssetDescriptor>();
  private effects = new Map<string, Effect>();
  private resources = new Map<string, Promise<Frames>>();
  private sourceKey = "";
  private epoch = 0;
  private changed: () => void;
  private warning: (message: string) => void;
  private matrix: (part: string) => THREE.Matrix4 | undefined;
  constructor(
    changed: () => void,
    warning: (message: string) => void,
    matrix: (part: string) => THREE.Matrix4 | undefined = () => undefined,
  ) {
    this.changed = changed;
    this.warning = warning;
    this.matrix = matrix;
  }

  setLibrary(library: FileSystemDirectoryHandle | null) {
    if (library === this.library) return;
    const document = this.document;
    this.clear();
    this.library = library;
    if (document) this.sync(document);
  }
  clear() {
    this.epoch++;
    for (const effect of this.effects.values()) this.remove(effect);
    this.effects.clear();
    for (const resource of this.resources.values())
      void resource.then(
        (value) => value.dispose(),
        () => {},
      );
    this.resources.clear();
    this.descriptors.clear();
    this.sourceKey = "";
    this.document = null;
  }
  private remove(effect: Effect) {
    if (!effect.frames) effect.mesh.geometry.dispose();
    effect.mesh.material.dispose();
    effect.mesh.removeFromParent();
  }
  sync(document: Level3D) {
    this.document = document;
    const library = this.library;
    if (!library) return;
    const key = JSON.stringify(document.assetSources ?? []);
    if (key !== this.sourceKey) {
      this.sourceKey = key;
      const epoch = ++this.epoch;
      void readPinnedAssetDescriptors(library, document.assetSources ?? []).then(
        (descriptors) => {
          if (epoch !== this.epoch) return;
          this.descriptors = descriptors;
          if (this.document) this.reconcile(this.document);
        },
        (error) => {
          if (epoch === this.epoch) this.warning(`Scenery previews unavailable: ${String(error)}`);
        },
      );
    }
    this.reconcile(document);
  }
  private reconcile(document: Level3D) {
    const hidden = new Set(
      document.groups.filter((group) => group.hidden).map((group) => group.id),
    );
    const live = new Set<string>();
    for (const part of document.objects) {
      const match = /^asset:([^:]+):(.+)$/.exec(part.node);
      if (!match || (part.group && hidden.has(part.group))) continue;
      const descriptor = this.descriptors.get(match[1]!);
      const animations = descriptor?.gameplay?.animations;
      if (!descriptor || !animations?.length) continue;
      if (
        part.hidden &&
        !document.objects.some(
          (other) =>
            other.group === part.group &&
            part.group &&
            !other.hidden &&
            other.node.startsWith(`asset:${match[1]}:`),
        )
      )
        continue;
      const pin = document.assetSources?.find((source) => source.id === descriptor.id);
      for (const animation of animations) {
        if (animation.node !== match[2] || !animation.active) continue;
        const id = `${part.id}/${animation.id}`;
        const key = JSON.stringify([
          pin?.descriptor_sha256,
          descriptor.id,
          animation.id,
          document.camera,
        ]);
        live.add(id);
        let effect = this.effects.get(id);
        if (effect && effect.key !== key) {
          this.remove(effect);
          this.effects.delete(id);
          effect = undefined;
        }
        if (!effect) {
          const mesh = new THREE.Mesh(
            new THREE.SphereGeometry(4),
            new THREE.MeshBasicMaterial({
              color: 0xffcc55,
              side: THREE.DoubleSide,
              alphaTest: 0.25,
            }),
          );
          mesh.userData.sceneryPart = part.id;
          mesh.name = `${part.name}: ${animation.id}`;
          effect = { mesh, key, start: performance.now() };
          this.effects.set(id, effect);
          this.root.add(mesh);
          let resource = this.resources.get(key);
          if (!resource) {
            const library = this.library!;
            resource = animation.resourceDirectory
              ? loadSceneryFrames(
                  descriptor,
                  animation,
                  async (name) =>
                    new Uint8Array(await (await libraryFile(library, name)).arrayBuffer()),
                  document.camera.elevation_deg,
                )
              : Promise.reject(
                  new Error("no pinned sprite bank; add animation resources to this asset"),
                );
            this.resources.set(key, resource);
          }
          const own = effect;
          void resource.then(
            (frames) => {
              if (this.effects.get(id) !== own) return;
              own.mesh.geometry.dispose();
              own.frames = frames;
              own.start = performance.now();
              own.mesh.material.color.setHex(0xffffff);
              own.mesh.material.needsUpdate = true;
              this.update(own.start);
              this.changed();
            },
            (error) => {
              if (this.effects.get(id) !== own) return;
              own.mesh.material.color.setHex(0xff45ce);
              this.warning(`${mesh.name}: scenery preview unavailable: ${String(error)}`);
            },
          );
        }
        const own = effect;
        effect.place = () => {
          const matrix =
            this.matrix(part.id) ??
            new THREE.Matrix4().fromArray(partMatrix(document.camera, document, part));
          try {
            const compiled = compileSceneryAnimation(animation, (_node, point) => {
              const placed = new THREE.Vector3(
                ...gameToScene(document.camera, ...point),
              ).applyMatrix4(matrix);
              return sceneToGame(document.camera, [placed.x, placed.y, placed.z]);
            });
            const sprite = compiled.sprite;
            const [x, y, z] = gameToScene(
              document.camera,
              sprite.position_x + animation.center[0],
              sprite.position_y + animation.center[1] + sprite.elevation,
              sprite.elevation,
            );
            own.mesh.position.set(x, z, -y);
            own.mesh.visible = true;
          } catch (error) {
            if (own.mesh.visible) this.warning(`${own.mesh.name}: ${String(error)}`);
            own.mesh.visible = false;
          }
        };
      }
    }
    for (const [id, effect] of this.effects)
      if (!live.has(id)) {
        this.remove(effect);
        this.effects.delete(id);
      }
    this.update(performance.now());
    this.changed();
  }
  update(now: number) {
    for (const effect of this.effects.values()) {
      effect.place?.();
      if (!effect.frames) continue;
      const frames = effect.frames.frames;
      const frame =
        frames[
          sceneryFrameAtTick(
            frames.map((frame) => frame.delay),
            Math.max(0, Math.floor((now - effect.start) / 40)),
          )
        ]!;
      effect.mesh.geometry = frame.geometry;
      effect.mesh.material.map = frame.texture;
    }
  }
}
