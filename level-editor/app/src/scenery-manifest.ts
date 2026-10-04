import { decode } from "fast-png";
import { safeLibraryPath } from "@rle/shared";

function record(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value))
    throw new Error("expected sprite record");
  return value as Record<string, unknown>;
}
function list(value: unknown, label: string): unknown[] {
  if (!Array.isArray(value) || value.length === 0 || value.length > 65535)
    throw new Error(`invalid ${label}`);
  return value;
}
function finite(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(Math.fround(value));
}
function uint(value: unknown, maximum = 65535): value is number {
  return typeof value === "number" && Number.isInteger(value) && value >= 0 && value <= maximum;
}

/** Admit the row and frame shapes consumed by the native custom-sprite loader. */
export async function validateSceneryManifest(
  value: unknown,
  files: Readonly<Record<string, Uint8Array>>,
  onFrame: () => void = () => {},
) {
  const manifest = record(value);
  if (manifest.pixel_format !== "rgba" && manifest.pixel_format !== "legacy_color_keys")
    throw new Error("invalid sprite pixel format");
  const names = new Set<string>();
  const images = new Set<string>();
  const profiles: {
    name: string;
    center_x: number;
    center_y: number;
    preview: string;
    rows: {
      action: number;
      direction: number;
      frames: { path: string; delay: number; offsetX: number; offsetY: number }[];
    }[];
  }[] = [];
  for (const entry of list(manifest.profiles, "sprite profiles")) {
    const profile = record(entry);
    if (typeof profile.name !== "string" || !profile.name || names.has(profile.name))
      throw new Error("invalid or duplicate sprite profile name");
    names.add(profile.name);
    if (
      !finite(profile.width) ||
      !finite(profile.height) ||
      profile.width <= 0 ||
      profile.height <= 0 ||
      !finite(profile.center_x) ||
      !finite(profile.center_y)
    )
      throw new Error("invalid sprite profile geometry");
    const result: (typeof profiles)[number] = {
      name: profile.name,
      center_x: profile.center_x,
      center_y: profile.center_y,
      preview: "",
      rows: [],
    };
    profiles.push(result);
    const directions = new Map<number, number[]>();
    for (const item of list(profile.rows, "sprite rows")) {
      const row = record(item);
      if (
        !uint(row.action_id, 282) ||
        !uint(row.action_done) ||
        !finite(row.average_speed) ||
        !finite(row.hotspot_x) ||
        !finite(row.hotspot_y) ||
        typeof row.path !== "string"
      )
        throw new Error("invalid sprite row metadata");
      const slots = directions.get(row.action_id) ?? [];
      const direction = row.direction ?? slots.length;
      if (!uint(direction)) throw new Error("invalid sprite direction");
      slots.push(direction);
      directions.set(row.action_id, slots);
      const validatedRow: (typeof result.rows)[number] = {
        action: row.action_id,
        direction,
        frames: [],
      };
      result.rows.push(validatedRow);
      for (const item of list(row.frames, "sprite frames")) {
        const frame = record(item);
        if (
          !uint(frame.delay) ||
          !uint(frame.distance) ||
          !uint(frame.sound_id) ||
          !finite(frame.offset_x) ||
          !finite(frame.offset_y) ||
          typeof frame.file !== "string"
        )
          throw new Error("invalid sprite frame metadata");
        const path = row.path && row.path !== "." ? `${row.path}/${frame.file}` : frame.file;
        const bytes = files[path];
        if (!safeLibraryPath(path) || !bytes) throw new Error(`missing pinned frame ${path}`);
        validatedRow.frames.push({
          path,
          delay: frame.delay,
          offsetX: frame.offset_x,
          offsetY: frame.offset_y,
        });
        if (!result.preview) result.preview = path;
        if (images.has(path)) continue;
        // Reject impossible runtime dimensions before the PNG decoder allocates its pixel buffer.
        if (bytes.length < 24 || bytes.slice(0, 8).join() !== "137,80,78,71,13,10,26,10")
          throw new Error(`invalid sprite PNG: ${path}`);
        const header = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
        if (header.getUint32(8) !== 13 || header.getUint32(12) !== 0x49484452)
          throw new Error(`invalid sprite PNG header: ${path}`);
        const width = header.getUint32(16),
          height = header.getUint32(20);
        if (
          !width ||
          !height ||
          width > 65535 ||
          height > 65535 ||
          width * height > 64 * 1024 * 1024
        )
          throw new Error(`sprite dimensions outside runtime limits: ${path}`);
        decode(bytes, { checkCrc: true });
        images.add(path);
        // Release the main thread between image decodes, including cancellation checks.
        await new Promise((resolve) => setTimeout(resolve, 0));
        onFrame();
      }
    }
    for (const slots of directions.values())
      if (slots.sort((a, b) => a - b).some((direction, index) => direction !== index))
        throw new Error("sprite action has duplicate or missing directions");
    // Runtime rows keep first-seen action order, with directions sorted within each action.
    const actions = new Map([...directions.keys()].map((action, index) => [action, index]));
    result.rows.sort(
      (a, b) => actions.get(a.action)! - actions.get(b.action)! || a.direction - b.direction,
    );
    result.preview = result.rows[0]!.frames[0]!.path;
  }
  return { profiles };
}
