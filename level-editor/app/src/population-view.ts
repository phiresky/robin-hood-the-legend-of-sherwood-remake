import * as THREE from "three";
import {
  gameToScene,
  safePopulationPath,
  type MapCamera,
  type Population,
  type PopulationSprite,
  type PopulationSpriteCatalog,
  type PopulationSpriteFrame,
} from "@rle/shared";
import { viewedDirection } from "./entity-projection.ts";
import { projectSpritePixel } from "./sprite-profiles.ts";
import { routePose, routeSchedule } from "./population-motion.ts";

export interface SceneEntities {
  root: THREE.Group;
  readonly count: number;
  readonly warnings: string[];
  update(camera: THREE.Camera, lockOrientations?: boolean): void;
  dispose(): void;
  setPlaying?(value: boolean): void;
  setRoutesVisible?(value: boolean): void;
}
async function file(root: FileSystemDirectoryHandle, path: string) {
  if (!safePopulationPath(path)) throw new Error("Unsafe population sprite path");
  const parts = path.split("/");
  let dir = root;
  for (const name of parts.slice(0, -1)) dir = await dir.getDirectoryHandle(name);
  return (await dir.getFileHandle(parts.at(-1)!)).getFile();
}
export class PopulationView implements SceneEntities {
  readonly root = new THREE.Group();
  readonly warnings: string[] = [];
  readonly routesRoot = new THREE.Group();
  private epoch = performance.now();
  private elapsed = 0;
  private playing = true;
  private textures = new Map<string, THREE.Texture>();
  private geometries = new Map<PopulationSpriteFrame, THREE.BufferGeometry>();
  private materials: THREE.Material[] = [];
  private extras: THREE.BufferGeometry[] = [];
  private actors: {
    mesh: THREE.Mesh<THREE.BufferGeometry, THREE.MeshBasicMaterial>;
    sprite: PopulationSprite;
    direction: number;
    phase: number;
    offset: [number, number];
    route?: ReturnType<typeof routeSchedule>;
  }[] = [];
  get count() {
    return this.population.actors.length + this.population.items.length;
  }
  private population: Population;
  private mapCamera: MapCamera;
  private constructor(population: Population, mapCamera: MapCamera) {
    this.population = population;
    this.mapCamera = mapCamera;
    this.routesRoot.visible = false;
    this.root.add(this.routesRoot);
  }
  setPlaying(value: boolean) {
    if (value === this.playing) return;
    this.elapsed = this.time();
    this.epoch = performance.now();
    this.playing = value;
  }
  setRoutesVisible(value: boolean) {
    this.routesRoot.visible = value;
  }
  private time() {
    return this.elapsed + (this.playing ? (performance.now() - this.epoch) / 1000 : 0);
  }
  static async load(
    root: FileSystemDirectoryHandle,
    population: Population,
    camera: MapCamera,
    current: () => boolean = () => true,
  ) {
    const view = new PopulationView(population, camera);
    try {
      await view.load(root, current);
      return view;
    } catch (error) {
      view.dispose();
      if (error instanceof Error && error.name === "NotFoundError") {
        const unavailable = new PopulationView({ ...population, actors: [], items: [] }, camera);
        unavailable.warnings.push(
          `Legacy population preview unavailable: ${error.message}. Saved population data is retained; editable Mission characters are unaffected.`,
        );
        return unavailable;
      }
      throw error;
    }
  }
  private async load(root: FileSystemDirectoryHandle, current: () => boolean) {
    const catalog = JSON.parse(
      await (await file(root, this.population.spriteCatalog)).text(),
    ) as PopulationSpriteCatalog;
    if (catalog.version !== 1 || !catalog.sprites)
      throw new Error("Invalid population sprite catalog");
    const required = [
      ...new Set([...this.population.actors, ...this.population.items].map((a) => a.sprite)),
    ];
    const loadSprite = async (id: string) => {
      if (!current()) throw new Error("Population load superseded");
      const sprite = catalog.sprites[id];
      if (!sprite) throw new Error("Missing population sprite: " + id);
      const blob = await file(root, sprite.image),
        url = URL.createObjectURL(blob);
      try {
        const texture = await new THREE.TextureLoader().loadAsync(url);
        texture.colorSpace = THREE.SRGBColorSpace;
        texture.magFilter = THREE.NearestFilter;
        texture.minFilter = THREE.LinearFilter;
        texture.generateMipmaps = false;
        this.textures.set(id, texture);
      } finally {
        URL.revokeObjectURL(url);
      }
    };
    let next = 0,
      failed = false,
      failure: unknown;
    await Promise.all(
      Array.from({ length: Math.min(4, required.length) }, async () => {
        while (!failed && next < required.length) {
          const id = required[next++]!;
          try {
            await loadSprite(id);
          } catch (error) {
            if (!failed) {
              failed = true;
              failure = error;
            }
          }
        }
      }),
    );
    if (failed) throw failure;
    if (!current()) throw new Error("Population load superseded");
    const schedules = new Map(
      this.population.routes.map((r) => [r.id, routeSchedule(r, this.mapCamera)]),
    );
    for (const entry of [...this.population.actors, ...this.population.items]) {
      const sprite = catalog.sprites[entry.sprite]!;
      const material = new THREE.MeshBasicMaterial({
        map: this.textures.get(entry.sprite),
        alphaTest: 0.3,
        side: THREE.DoubleSide,
      });
      this.materials.push(material);
      const mesh = new THREE.Mesh(new THREE.BufferGeometry(), material);
      this.extras.push(mesh.geometry);
      mesh.name = entry.name;
      const p = gameToScene(this.mapCamera, ...entry.position);
      mesh.position.set(p[0], p[2], -p[1]);
      const actor = "role" in entry ? entry : null;
      const schedule = actor?.route ? schedules.get(actor.route) : undefined;
      if (schedule && !sprite.walk)
        throw new Error("Patrolling actor lacks walk animation: " + entry.name);
      const start = schedule?.legs[0]?.from.position,
        end = schedule?.legs[Math.floor(schedule.legs.length / 2) - 1]?.to.position;
      const dx = start && end ? end[0] - start[0] : 0,
        dy =
          start && end
            ? (end[1] - start[1]) / Math.sin((this.mapCamera.elevation_deg * Math.PI) / 180)
            : 0;
      const length = Math.hypot(dx, dy) || 1,
        offset = actor?.routeOffset ?? 0;
      this.actors.push({
        mesh,
        sprite,
        direction: actor?.direction ?? 0,
        phase: actor?.phase ?? 0,
        offset: [(dy / length) * offset, (dx / length) * offset],
        route: schedule,
      });
      this.root.add(mesh);
      if (actor) {
        const shadowGeometry = new THREE.CircleGeometry(10, 16);
        shadowGeometry.rotateX(-Math.PI / 2);
        const shadowMaterial = new THREE.MeshBasicMaterial({
          color: 0x171a14,
          transparent: true,
          opacity: 0.32,
          depthWrite: false,
          polygonOffset: true,
          polygonOffsetFactor: -2,
        });
        const shadow = new THREE.Mesh(shadowGeometry, shadowMaterial);
        shadow.position.y = 0.6;
        shadow.scale.z = 1.4;
        mesh.add(shadow);
        this.extras.push(shadowGeometry);
        this.materials.push(shadowMaterial);
      }
    }
    for (const route of this.population.routes) {
      const points = route.points.map((pt) => {
        const p = gameToScene(this.mapCamera, ...pt.position);
        return new THREE.Vector3(p[0], p[2] + 3, -p[1]);
      });
      if (route.mode === "loop") points.push(points[0]!.clone());
      const geometry = new THREE.BufferGeometry().setFromPoints(points),
        material = new THREE.LineBasicMaterial({
          color: 0x69dada,
          depthTest: false,
          transparent: true,
          opacity: 0.65,
        });
      const line = new THREE.Line(geometry, material);
      line.name = route.name;
      line.renderOrder = 10;
      this.routesRoot.add(line);
      this.extras.push(geometry);
      this.materials.push(material);
    }
    this.epoch = performance.now();
  }
  private geometry(frame: PopulationSpriteFrame, sprite: PopulationSprite) {
    const cached = this.geometries.get(frame);
    if (cached) return cached;
    const [x, y, w, h] = frame.rect,
      [left, top] = frame.offset;
    const geometry = new THREE.PlaneGeometry(w, h, 8, 16),
      position = geometry.getAttribute("position"),
      uv = geometry.getAttribute("uv");
    for (let i = 0; i < position.count; i++) {
      const right = position.getX(i) + left + w / 2,
        up = position.getY(i) + top - h / 2;
      position.setXYZ(
        i,
        ...projectSpritePixel(
          sprite.kind === "character" ? "upright-character" : "low-object",
          right,
          up,
          { left, top, width: w, height: h },
          (this.mapCamera.elevation_deg * Math.PI) / 180,
        ),
      );
      uv.setXY(
        i,
        (x + uv.getX(i) * w) / sprite.width,
        1 - (y + (1 - uv.getY(i)) * h) / sprite.height,
      );
    }
    geometry.computeBoundingSphere();
    this.geometries.set(frame, geometry);
    return geometry;
  }
  update(camera: THREE.Camera, lock = false) {
    const time = this.time(),
      forward = camera.getWorldDirection(new THREE.Vector3()).negate();
    for (const actor of this.actors) {
      const pose = actor.route ? routePose(actor.route, time + actor.phase) : null;
      if (pose) {
        const p = gameToScene(this.mapCamera, ...pose.position);
        p[0] += actor.offset[0];
        p[1] += actor.offset[1];
        actor.mesh.position.set(p[0], p[2], -p[1]);
        actor.direction = pose.direction;
      }
      const view = (camera as THREE.PerspectiveCamera).isPerspectiveCamera
        ? camera.position.clone().sub(actor.mesh.position)
        : forward;
      const azimuth = Math.atan2(view.x, view.z),
        direction = viewedDirection(actor.direction, azimuth);
      const animation = pose?.walking ? actor.sprite.walk! : actor.sprite.idle;
      const frames = animation[String(direction)] ?? animation["-1"];
      if (!frames?.length) throw new Error("Missing population direction: " + actor.mesh.name);
      const duration = frames.reduce((n, f) => n + f.duration, 0);
      let cursor = (time + actor.phase) % duration,
        frame = frames.at(-1)!;
      for (const f of frames) {
        if (cursor < f.duration) {
          frame = f;
          break;
        }
        cursor -= f.duration;
      }
      actor.mesh.geometry = this.geometry(frame, actor.sprite);
      actor.mesh.rotation.y =
        actor.sprite.kind === "pickup"
          ? 0
          : lock
            ? ((direction - actor.direction) * Math.PI) / 8
            : azimuth;
    }
  }
  dispose() {
    this.root.removeFromParent();
    this.root.clear();
    for (const t of this.textures.values()) t.dispose();
    for (const g of this.geometries.values()) g.dispose();
    for (const g of this.extras) g.dispose();
    for (const m of this.materials) m.dispose();
    this.textures.clear();
    this.geometries.clear();
    this.extras = [];
    this.materials = [];
    this.actors = [];
  }
}
