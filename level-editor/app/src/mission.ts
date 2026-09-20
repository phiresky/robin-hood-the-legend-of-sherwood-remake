import * as THREE from "three";
import { gameToScene, type MapCamera, type ProtoLevel } from "@rle/shared";
import { isNotFound, readJson, subdir } from "./fs.ts";
import type { DatadirIndex } from "./datadir.ts";
import { decodeSpritePixels, placementHeight, spriteShadowPixels, spriteShadowStyle, viewedDirection } from "./entity-projection.ts";

import { bonusSprite, projectSpritePixel, sanitizedProfileName, spriteShape, type SpriteKind } from "./sprite-profiles.ts";

class MissingSpriteError extends Error {}

type RecordData = Record<string, unknown>;
function record(value: unknown, label: string): RecordData {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error(`${label}: expected object`);
  return value as RecordData;
}
function number(value: unknown, label: string): number {
  if (typeof value !== "number" || !Number.isFinite(value)) throw new Error(`${label}: expected finite number`);
  return value;
}
function string(value: unknown, label: string): string {
  if (typeof value !== "string" || !value) throw new Error(`${label}: expected name`);
  return value;
}
function rows(value: unknown, label: string): RecordData[] {
  if (!Array.isArray(value)) throw new Error(`${label}: expected array`);
  return value.map((v) => record(v, label));
}
export interface MissionData {
  name: string;
  map: string;
  data: RecordData;
}
export async function readMission(index: DatadirIndex, name: string): Promise<MissionData> {
  const data = record(await readJson(index.levelsDir, `${name}.rhm.json`), name);
  const header = record(data.header, `${name}.header`);
  return { name, map: string(header.map_filename, "mission map"), data };
}

async function directory(root: FileSystemDirectoryHandle, path: string[]): Promise<FileSystemDirectoryHandle> {
  // Asset names in converted datasets can differ in case from profile names.
  let current = root;
  for (const name of path) {
    if (!name || name === "." || name === ".." || /[\\/]/.test(name)) throw new Error(`Invalid asset path: ${name}`);
    try { current = await current.getDirectoryHandle(name); }
    catch (error) {
      if (!isNotFound(error)) throw error;
      let found: FileSystemDirectoryHandle | undefined;
      for await (const [key, entry] of current.entries()) {
        if (entry.kind === "directory" && key.toLowerCase() === name.toLowerCase()) found = entry as FileSystemDirectoryHandle;
      }
      if (!found) throw new MissingSpriteError(`Missing sprite directory ${path.join("/")}`);
      current = found;
    }
  }
  return current;
}
interface SpriteFrame {
  texture: THREE.Texture;
  geometry: THREE.BufferGeometry;
  shadow: THREE.Texture | null;
  bounds: { left: number; top: number; width: number; height: number };
}
interface ActorView {
  mesh: THREE.Mesh<THREE.BufferGeometry, THREE.MeshBasicMaterial>;
  frames: Map<number, SpriteFrame>;
  direction: number;
  shadow: THREE.Mesh | null;
}

/** A mission preview owns every loaded texture, geometry, and material, including
 * directions that are currently invisible. It never writes mission files. */
export class MissionEntities {
  readonly root = new THREE.Group();
  readonly warnings: string[] = [];
  private actors: ActorView[] = [];
  private textures = new Set<THREE.Texture>();
  private geometries = new Set<THREE.BufferGeometry>();
  private materials = new Set<THREE.Material>();
  private disposed = false;
  get count() { return this.root.children.length; }
  update(camera: THREE.Camera) {
    const forward = camera.getWorldDirection(new THREE.Vector3()).negate();
    const perspective = (camera as THREE.PerspectiveCamera).isPerspectiveCamera;
    for (const actor of this.actors) {
      const view = perspective ? camera.position.clone().sub(actor.mesh.position) : forward;
      const azimuth = Math.atan2(view.x, view.z);
      const direction = viewedDirection(actor.direction, azimuth);
      const frame = actor.frames.get(direction) ?? actor.frames.get(-1);
      if (!frame) throw new Error("Missing validated entity direction");
      actor.mesh.geometry = frame.geometry;
      actor.mesh.material.map = frame.texture;
      // Every shape is reconstructed from its frame's fixed source-view angle.
      // Keep that projection stationary between directional frame transitions.
      actor.mesh.rotation.y = actor.frames.has(-1) ? 0 : (direction - actor.direction) * Math.PI / 8;
      // Authored shadows remain fixed in world space as the camera orbits.
      if (actor.shadow) actor.shadow.rotation.y = -actor.mesh.rotation.y;
    }
  }
  dispose() {
    if (this.disposed) return;
    this.disposed = true;
    this.root.removeFromParent();
    this.root.clear();
    for (const m of this.materials) m.dispose();
    for (const g of this.geometries) g.dispose();
    for (const t of this.textures) t.dispose();
    this.actors = [];
  }
  private marker(label: string, position: THREE.Vector3, color: number) {
    const geometry = new THREE.ConeGeometry(7, 20, 6);
    const material = new THREE.MeshBasicMaterial({ color });
    this.geometries.add(geometry); this.materials.add(material);
    const mesh = new THREE.Mesh(geometry, material);
    mesh.name = label;
    mesh.position.copy(position).y += 10;
    this.root.add(mesh);
  }
  static async load(index: DatadirIndex, mission: MissionData, level: ProtoLevel, camera: MapCamera, current: () => boolean = () => true): Promise<MissionEntities> {
    const result = new MissionEntities();
    try { await result.load(index, mission, level, camera, current); return result; }
    catch (error) { result.dispose(); throw error; }
  }
  private async load(index: DatadirIndex, mission: MissionData, level: ProtoLevel, camera: MapCamera, current: () => boolean) {
    if (!index.root) throw new Error("Reconnect the datadir to load mission sprites");
    const config = await subdir(index.root, ["Data", "Configuration"]);
    if (!config) throw new Error("Data/Configuration missing");
    const profiles = record(await readJson(config, "profile.cpf.json"), "profiles");
    const cache = new Map<string, Map<number, SpriteFrame>>();
    const ambianceValue = record(mission.data.header, "mission header").ambiance;
    const ambiance = ({ 1: "Day", 2: "Fog", 4: "Night", 8: "Attack", 16: "Custom1", 32: "Custom2", 64: "Custom3", 128: "Custom4" } as Record<number, string>)[number(ambianceValue, "mission ambiance")];
    if (!ambiance) throw new Error(`Unknown mission ambiance ${ambianceValue}`);
    const position = (entity: RecordData) => {
      const anchor = entity.editor_anchor;
      if (anchor !== undefined && (!Array.isArray(anchor) || anchor.length !== 2)) throw new Error("Invalid mobile editor anchor");
      const p = Array.isArray(anchor) ? { x: anchor[0], y: anchor[1] } : entity.position === undefined ? null : record(entity.position, "entity position");
      const x = number(p ? p.x : entity.position_x, "entity x");
      const y = number(p ? p.y : entity.position_y, "entity y");
      const support = entity.obstacle_index ?? entity.projection_area ?? 65535;
      const explicitZ = entity.position_z === undefined ? -1 : number(entity.position_z, "entity z");
      let z = Math.max(0, explicitZ);
      if (explicitZ < 0 && support !== 65535) {
        const obstacle = level.sight_obstacles[number(support, "entity support")];
        if (!obstacle) throw new Error(`Missing entity support #${support}`);
        z = placementHeight(x, y, obstacle);
      }
      const scene = gameToScene(camera, x, y + z, z);
      return new THREE.Vector3(scene[0], scene[2], -scene[1]);
    };
    for (const [group, orderKey, profileKey, idKey] of [
      ["soldiers", "soldier_order", "soldiers", "profile_number"],
      ["civilians", "civilian_order", "civilians", "profile_number"],
      ["pcs_to_rescue", "character_order", "characters", "profile_index"],
    ] as const) {
      for (const [i, entity] of rows(mission.data[group] ?? [], group).entries()) {
        if (!current()) throw new Error("Mission load superseded");
        const order = profiles[orderKey];
        if (!Array.isArray(order)) throw new Error(`Missing ${orderKey}`);
        const id = number(entity[idKey], `${group} profile`);
        const key = string(order[id], `${group} profile #${id}`);
        const profile = record(record(profiles[profileKey], profileKey)[key], key);
        const filename = string(profile.filename, `${key} filename`);
        const profileName = string(profile.profile_name, `${key} profile name`);
        const action = number(entity.action, `${key} action`);
        const direction = number(entity.direction, `${key} direction`);
        if (!Number.isInteger(direction) || direction < 0 || direction > 15) throw new Error(`Invalid direction for ${key}`);
        const cacheKey = `${filename}/${profileName}/${action}`;
        let frames = cache.get(cacheKey);
        if (!frames) {
          frames = await this.loadFrames(index.root, filename, profileName, action, camera, current, "character", ambiance);
          cache.set(cacheKey, frames);
        }
        const frame = frames.get(direction) ?? frames.get(-1)!;
        const material = new THREE.MeshBasicMaterial({ map: frame.texture, alphaTest: 0.25, side: THREE.DoubleSide });
        this.materials.add(material);
        const mesh = new THREE.Mesh(frame.geometry, material);
        mesh.name = `${group} ${i + 1}: ${key}`;
        mesh.position.copy(position(entity));
        const shadow = this.attachShadow(mesh, frame, entity, level, camera, ambiance);
        this.root.add(mesh);
        this.actors.push({ mesh, frames, direction, shadow });
      }
    }
    const addSprite = async (entity: RecordData, label: string, filename: string, profileName: string, kind: SpriteKind, action: number, at = position(entity)) => {
      if (!current()) throw new Error("Mission load superseded");
      const key = `${kind}/${filename}/${profileName}/${action}`;
      try {
        let frames = cache.get(key);
        if (!frames) {
          frames = await this.loadFrames(index.root!, filename, profileName, action, camera, current, kind, ambiance);
          cache.set(key, frames);
        }
        const direction = number(entity.direction ?? 0, `${label} direction`);
        const frame = frames.get(direction) ?? frames.get(-1);
        if (!frame) throw new Error(`${label}: missing direction ${direction}`);
        const material = new THREE.MeshBasicMaterial({ map: frame.texture, alphaTest: 0.25, side: THREE.DoubleSide });
        this.materials.add(material);
        const mesh = new THREE.Mesh(frame.geometry, material);
        mesh.name = label;
        mesh.position.copy(at);
        const shadow = this.attachShadow(mesh, frame, entity, level, camera, ambiance);
        this.root.add(mesh);
        this.actors.push({ mesh, frames, direction, shadow });
      } catch (error) {
        if (!(error instanceof MissingSpriteError)) throw error;
        const message = `${filename}: sprite missing from datadir (magenta marker)`;
        if (!this.warnings.includes(message)) this.warnings.push(message);
        this.marker(label, at, 0xff45ce);
      }
    };
    for (const [i, entity] of rows(mission.data.bonuses ?? [], "bonuses").entries()) {
      const [file, profile] = bonusSprite(number(entity.bonus_type, "bonus type"));
      const quantity = number(entity.quantity, "bonus quantity");
      if (!Number.isInteger(quantity) || quantity < 1 || quantity > 5) throw new Error("Bonus quantity must be 1–5");
      await addSprite(entity, `bonus ${i + 1}: ${file}`, file, profile, "pickup", 189 + quantity);
    }
    for (const [i, entity] of rows(mission.data.scrolls ?? [], "scrolls").entries()) {
      await addSprite(entity, `scroll ${i + 1}`, "BONUS_Parchment", "BONUS Parchemin", "pickup", number(entity.action, "scroll action"));
    }
    for (const [i, entity] of rows(mission.data.targets ?? [], "targets").entries()) {
      await addSprite(entity, `target ${i + 1}`, string(entity.filename, "target filename"), string(entity.profile_name, "target profile"), "scenery", number(entity.action, "target action"));
    }
    for (const [i, entity] of rows(mission.data.mobile_elements ?? [], "mobile elements").entries()) {
      const path = rows(mission.data.hiking_paths, "hiking paths")[number(entity.path_index, "mobile path")];
      if (!path) throw new Error(`Missing path for mobile ${i + 1}`);
      const waypoint = rows(path.waypoints, "waypoints")[number(entity.start_waypoint, "start waypoint")];
      if (!waypoint) throw new Error(`Missing start waypoint for mobile ${i + 1}`);
      for (const fx of rows(entity.sprites, "mobile sprites")) {
        if (fx.active === false) continue;
        const sprite = record(fx.sprite, "mobile sprite");
        const placement = { position_x: number(waypoint.x, "waypoint x") + number(sprite.position_x, "sprite x"), position_y: number(waypoint.y, "waypoint y") + number(sprite.position_y, "sprite y"), obstacle_index: entity.obstacle_index };
        await addSprite(placement, `mobile ${i + 1}`, string(sprite.frame_profile_name, "mobile filename"), string(sprite.profile_name, "mobile profile"), "scenery", 0);
      }
    }
    const spawns = rows(mission.data.beam_mes ?? [], "spawn points");
    for (const [i, entity] of spawns.entries()) this.marker(`spawn ${i + 1}`, position(entity), 0x58e4ad);
    if (spawns.length) this.warnings.push(`${spawns.length} spawn points shown as green markers`);
  }
  private attachShadow(mesh: THREE.Mesh, frame: SpriteFrame, entity: RecordData, level: ProtoLevel, camera: MapCamera, ambiance: string): THREE.Mesh | null {
    if (!frame.shadow) return null;
    const { left, top, width, height } = frame.bounds;
    const elevation = camera.elevation_deg * Math.PI / 180;
    const sin = Math.sin(elevation), cos = Math.cos(elevation);
    const baseZ = mesh.position.y * cos;
    const mapY = mesh.position.z * sin - baseZ;
    const supportIndex = entity.obstacle_index ?? entity.projection_area ?? 65535;
    const support = supportIndex === 65535 ? null : level.sight_obstacles[number(supportIndex, "shadow support")];
    if (supportIndex !== 65535 && !support) throw new Error(`Missing shadow support #${supportIndex}`);
    const geometry = new THREE.PlaneGeometry(width, height);
    const points = geometry.getAttribute("position");
    for (let i = 0; i < points.count; i++) {
      const x = points.getX(i) + left + width / 2;
      const up = points.getY(i) + top - height / 2;
      const z = support ? placementHeight(mesh.position.x + x, mapY - up, support) : 0;
      const dz = z - baseZ;
      points.setXYZ(i, x, dz / cos + 0.15, (dz - up) / sin);
    }
    geometry.computeBoundingSphere();
    const style = spriteShadowStyle(ambiance);
    const material = new THREE.MeshBasicMaterial({ map: frame.shadow, color: style.color, opacity: style.opacity, transparent: true, depthWrite: false, side: THREE.DoubleSide, polygonOffset: true, polygonOffsetFactor: -1, polygonOffsetUnits: -1 });
    const shadow = new THREE.Mesh(geometry, material);
    shadow.name = "authored shadow";
    this.geometries.add(geometry); this.materials.add(material);
    mesh.add(shadow);
    return shadow;
  }
  private async loadFrames(root: FileSystemDirectoryHandle, filename: string, profileName: string, action: number, camera: MapCamera, current: () => boolean, kind: SpriteKind, ambiance: string): Promise<Map<number, SpriteFrame>> {
    let dir: FileSystemDirectoryHandle | null = null;
    const paths = kind === "scenery" ? [...new Set([ambiance, "Day", ""])].map((a) => ["Data", "Animations", ...(a ? [a] : []), `${filename}.rhs.d`]) : [["Data", "Characters", `${filename}.rhs.d`]];
    for (const path of paths) {
      try { dir = await directory(root, path); break; }
      catch (error) { if (!(error instanceof MissingSpriteError)) throw error; }
    }
    if (!dir) throw new MissingSpriteError(`Missing sprite ${filename}`);
    const manifest = record(await readJson(dir, "manifest.json"), filename);
    const profile = rows(manifest.profiles, "sprite profiles").find((p) => p.name === profileName);
    if (!profile) throw new Error(`${filename}: sprite profile ${profileName} missing`);
    const allRows = rows(profile.rows, "sprite rows");
    let selected = allRows.filter((r) => r.action_id === action);
    if (!selected.length && kind === "character") {
      selected = allRows.filter((r) => r.action_id === 3);
      if (!selected.length) selected = allRows.filter((r) => r.action_id === 0);
      this.warnings.push(`${filename}: action ${action} unavailable; showing idle`);
    }
    if (!selected.length) throw new Error(`${filename}: no initial or idle animation`);
    const directions = new Set(selected.map((r) => r.direction));
    const single = selected.length === 1;
    if (!single && !Array.from({ length: 16 }, (_, i) => i).every((i) => directions.has(i))) throw new Error(`${filename}: incomplete 16-direction animation`);
    const frames = new Map<number, SpriteFrame>();
    for (const row of selected) {
      if (!current()) throw new Error("Mission load superseded");
      const frame = rows(row.frames, "sprite frames")[0];
      if (!frame) throw new Error(`${filename}: empty sprite row`);
      const rowPath = string(row.path, "sprite row path").split("/").filter((part) => part !== ".");
      let frameDir: FileSystemDirectoryHandle;
      try { frameDir = await directory(dir, rowPath); }
      catch (error) {
        if (!(error instanceof MissingSpriteError) || rows(manifest.profiles, "profiles").length === 1) throw error;
        frameDir = await directory(dir, [sanitizedProfileName(profileName), ...rowPath]);
      }
      const fileName = string(frame.file, "sprite frame file");
      if (/[\\/]/.test(fileName) || fileName === "..") throw new Error("Invalid sprite frame path");
      const file = await (await frameDir.getFileHandle(fileName)).getFile();
      const bitmap = await createImageBitmap(file);
      const width = bitmap.width, heightPx = bitmap.height;
      const canvas = new OffscreenCanvas(width, heightPx);
      const context = canvas.getContext("2d", { willReadFrequently: true });
      if (!context) { bitmap.close(); throw new Error("Cannot decode sprite pixels"); }
      try { context.drawImage(bitmap, 0, 0); } finally { bitmap.close(); }
      const pixels = context.getImageData(0, 0, width, heightPx);
      if (manifest.pixel_format !== undefined && manifest.pixel_format !== "rgba" && manifest.pixel_format !== "legacy_color_keys") throw new Error(`${filename}: unsupported pixel format`);
      const legacy = manifest.pixel_format !== "rgba";
      const shadowPixels = spriteShadowPixels(pixels.data, legacy);
      let shadow: THREE.Texture | null = null;
      if (shadowPixels) {
        const shadowCanvas = new OffscreenCanvas(width, heightPx);
        const shadowContext = shadowCanvas.getContext("2d");
        if (!shadowContext) throw new Error("Cannot decode shadow pixels");
        const image = shadowContext.createImageData(width, heightPx);
        image.data.set(shadowPixels);
        shadowContext.putImageData(image, 0, 0);
        shadow = new THREE.CanvasTexture(shadowCanvas);
        this.textures.add(shadow);
      }
      decodeSpritePixels(pixels.data, legacy);
      context.putImageData(pixels, 0, 0);
      const texture = new THREE.CanvasTexture(canvas);
      texture.colorSpace = THREE.SRGBColorSpace;
      texture.needsUpdate = true;
      this.textures.add(texture);
      const left = number(frame.offset_x, "frame offset x") - number(profile.center_x, "sprite center x");
      const top = number(profile.center_y, "sprite center y") - number(frame.offset_y, "frame offset y");
      const elevation = camera.elevation_deg * Math.PI / 180;
      const shape = spriteShape(kind, number(row.action_id, "sprite action"));
      const geometry = new THREE.PlaneGeometry(width, heightPx, Math.min(width, 32), Math.min(heightPx, 64));
      const attr = geometry.getAttribute("position");
      for (let i = 0; i < attr.count; i++) {
        const right = attr.getX(i) + left + width / 2;
        const up = attr.getY(i) + top - heightPx / 2;
        attr.setXYZ(i, ...projectSpritePixel(shape, right, up, { left, top, width, height: heightPx }, elevation));
      }
      geometry.userData.spriteShape = shape;
      geometry.computeBoundingSphere();
      this.geometries.add(geometry);
      frames.set(single ? -1 : number(row.direction, "sprite direction"), { texture, geometry, shadow, bounds: { left, top, width, height: heightPx } });
    }
    return frames;
  }
}
