import { encode } from "fast-png";
import type { GameplayAssetDescriptor } from "../../shared/src/asset-gameplay.ts";

export async function sceneryThumbnailFixture(legacy = true, animated = false) {
  const directory = "effects/fire.rhs.d";
  const manifest = {
    pixel_format: legacy ? "legacy_color_keys" : "rgba",
    profiles: [
      {
        name: "burning",
        width: 2,
        height: 1,
        center_x: 1,
        center_y: 1,
        rows: [
          {
            action_id: 0,
            action_done: 283,
            average_speed: 0,
            hotspot_x: 0,
            hotspot_y: 0,
            path: "idle",
            frames: [
              { file: "0.png", delay: 2, distance: 0, offset_x: 0, offset_y: 0, sound_id: 65535 },
            ],
          },
        ],
      },
    ],
  };
  const files = new Map([
    [`${directory}/manifest.json`, new TextEncoder().encode(JSON.stringify(manifest))],
    [
      `${directory}/idle/0.png`,
      encode({
        width: 2,
        height: 1,
        channels: 4,
        data: new Uint8Array([0, 248, 0, 255, 255, 0, 0, 255]),
      }),
    ],
  ]);
  if (animated) {
    manifest.profiles[0]!.rows[0]!.frames.push({
      file: "1.png",
      delay: 4,
      distance: 0,
      offset_x: 3,
      offset_y: -2,
      sound_id: 65535,
    });
    files.set(`${directory}/manifest.json`, new TextEncoder().encode(JSON.stringify(manifest)));
    files.set(
      `${directory}/idle/1.png`,
      encode({
        width: 2,
        height: 1,
        channels: 4,
        data: new Uint8Array([0, 0, 255, 255, 0, 0, 255, 255]),
      }),
    );
  }
  const resources = await Promise.all(
    [...files].map(async ([path, bytes]) => ({
      path,
      sha256: [...new Uint8Array(await crypto.subtle.digest("SHA-256", new Uint8Array(bytes)))]
        .map((n) => n.toString(16).padStart(2, "0"))
        .join(""),
    })),
  );
  const descriptor: GameplayAssetDescriptor = {
    version: 1,
    kind: "projection-mapped-asset",
    id: "fire",
    name: "Fire",
    source_map: "Authored",
    model: "model.glb",
    parts: [{ node: "scenery-effect", name: "Fire", scenery: true, gameplay_only: true }],
    resources,
    gameplay: {
      version: 1,
      collision: "none",
      surfaces: [],
      doors: [],
      animations: [
        {
          id: "fire",
          node: "scenery-effect",
          anchor: [0, 0, 0],
          file: "fire",
          profile: "burning",
          center: [1, 1],
          active: true,
          forceDisplay: false,
          shadow: false,
          displayPolyline: [],
          resourceDirectory: directory,
        },
      ],
    },
  };
  const read = async (name: string) => {
    const bytes = files.get(name);
    if (!bytes) throw new Error(`Missing ${name}`);
    return bytes;
  };
  return { descriptor, files, read };
}
