import { encode } from "fast-png";
import { missionStateDataHash, type MissionStateSource } from "./mission-state-layer.ts";
import type { NativeLoopPreviewContract } from "../../shared/src/state-delivery.ts";
import type { SourceCoverVisibility } from "../../shared/src/source-cover-visibility.ts";
/** Small in-memory source records and artwork; no local asset library required. */
export async function sourceCoverFixture() {
  const files = new Map<string, Uint8Array>();
  async function image(path: string, color: number[]) {
    const data = encode({
      width: 2,
      height: 2,
      channels: 4,
      data: new Uint8Array([...color, ...color, ...color, ...color]),
    });
    files.set(path, data);
    const sha256 = Array.from(
      new Uint8Array(await crypto.subtle.digest("SHA-256", new Uint8Array(data))),
      (n) => n.toString(16).padStart(2, "0"),
    ).join("");
    return { path, sha256, width: 2, height: 2 };
  }
  const patches = [0, 1, 2].map((index) => ({
    index,
    start_animation_valid: true,
    transition_animation_valid: false,
    end_animation_valid: false,
  }));
  const animation = {
    sprite: { position_x: 0, position_y: 0, elevation: 0 },
    active: true,
    display_polyline: [],
    blit_type: 0,
  };
  const source = {
    name: "test-mission",
    data: { targets: [] },
    level: { animations: [animation], patches, sight_obstacles: [] },
    camera: { kind: "oblique-orthographic", elevation_deg: 35 },
  } as unknown as MissionStateSource;
  const common = {
    version: 1 as const,
    mission: source.name,
    mission_data_sha256: await missionStateDataHash(source.data),
    level_data_sha256: await missionStateDataHash(source.level),
    camera_elevation_deg: 35,
  };
  const asset = {
    id: "test-hall",
    model_sha256: "a".repeat(64),
    descriptor_sha256: "b".repeat(64),
  };
  const group = {
    id: "placed-test-hall",
    transform: { dx: 0, dy: 0, dz: 0, rot_deg: 0 },
    patches: { [asset.id]: { "appearance-1": "patch-001", "appearance-2": "patch-002" } },
  };
  const bindings = await Promise.all(
    [1, 2].map(async (index) => ({
      source: {
        kind: "map-patch" as const,
        index,
        sha256: await missionStateDataHash(patches[index]),
      },
      placement: {
        group_id: group.id,
        transform: group.transform,
        asset_id: asset.id,
        appearance_id: `appearance-${index}`,
        patch_id: `patch-00${index}`,
        model_sha256: asset.model_sha256,
        descriptor_sha256: asset.descriptor_sha256,
      },
    })),
  );
  const cover: SourceCoverVisibility = {
    ...bindings[1]!,
    native_element_id: "animation-000",
    physical_target_id: "hall000-flames-reflections",
    visible_when: "applied",
  };
  const variants = await Promise.all(
    [[], ["patch-001"], ["patch-002"], ["patch-001", "patch-002"]].map(async (applied, i) => ({
      applied,
      image: await image(`background-${i}.png`, [20 + i * 30, 40, 60, 255]),
    })),
  );
  const frame = await image("effect.png", [220, 100, 0, 120]);
  const c: NativeLoopPreviewContract = {
    version: 1,
    scope: "controlled-native-loop-preview",
    focus_element_id: "animation-000",
    cover_visibility: [cover],
    native: {
      ...common,
      scope: "map-art-and-listed-effects",
      origin: [0, 0],
      background: variants[0]!.image,
      cover_artwork: { bindings, variants },
      elements: [
        {
          id: "animation-000",
          source: {
            kind: "map-animation",
            index: 0,
            sha256: await missionStateDataHash(animation),
          },
          active: true,
          loop: true,
          frames: Array.from({ length: 6 }, () => ({ ...frame, offset: [0, 0], delay: 2 })),
          display_position: [0, 0],
          sort_position: [0, 0],
          display_order: 0,
          creation_order: 0,
          polyline: [],
        },
      ],
    },
    physical: {
      ...common,
      targets: [
        {
          id: cover.physical_target_id,
          source_kind: "map-animation",
          target_index: 0,
          target_sha256: await missionStateDataHash(animation),
          representation: "physical",
          model_origin: [0, 0, 0],
          source: {
            id: "test-effect",
            role: "objects",
            model: "effect.glb",
            model_sha256: "c".repeat(64),
            resources: [],
          },
          actions: [{ action: 0, clip: "effect-loop", timing: { mode: "loop", cycleTicks: 18 } }],
        },
      ],
    },
  };
  const document = {
    groups: [group],
    assetSources: [asset],
    objects: [{ group: group.id, node: `asset:${asset.id}:roof` }],
  } as any;
  const reader = async (r: { path: string }) => {
    const data = files.get(r.path);
    if (!data) throw new Error(`Missing synthetic resource ${r.path}`);
    return data;
  };
  return { c, source, cover, document, reader };
}
