import type { StateDeliveryContract } from "../src/state-delivery.ts";
import type { PhysicalElementComponentClips } from "../src/component-clips.ts";
export function componentClipFixture() {
  const binding = (id: string) => ({
    id,
    role: "objects" as const,
    model: id + ".glb",
    model_sha256: "a".repeat(64),
    resources: [],
  });
  const frame = {
    path: "frame.png",
    sha256: "a".repeat(64),
    width: 1,
    height: 1,
    offset: [0, 0] as [number, number],
    delay: 0,
  };
  const element = (id: string, index: number) => ({
    id,
    source: { kind: "mission-target" as const, index, sha256: "a".repeat(64) },
    active: false,
    loop: false,
    frames: [{ ...frame }, { ...frame, delay: 2 }, { ...frame }],
    display_position: [0, 0] as [number, number],
    sort_position: [0, 0] as [number, number],
    display_order: index,
    creation_order: index,
    polyline: [],
  });
  const e = element("cart", 0),
    native = element("actors", 1);
  const motion: PhysicalElementComponentClips = {
    kind: "native-element-component-clips",
    source_motion_sha256: "b".repeat(64),
    native_passthrough_element_ids: ["actors"],
    components: [
      {
        id: "body",
        binding: binding("clip"),
        clip: "collapse",
        nodes: ["old", "wreck"],
        stages: [
          {
            element_id: e.id,
            source_element_sha256: e.source.sha256,
            source_frames: structuredClone(e.frames),
            first_source_frame: 0,
            clip_start_tick: 0,
            frame_ticks: [0, 1, 4],
            gaps: [],
          },
        ],
        visibility_phases: [
          { first_tick: 0, last_tick: 0, active_nodes: ["old"] },
          { first_tick: 1, last_tick: 4, active_nodes: ["wreck"] },
        ],
        terminal: {
          stage_element_id: e.id,
          source_frame: 2,
          clip_tick: 4,
          active_nodes: ["wreck"],
          retired_nodes: ["old"],
        },
      },
    ],
  };
  const contract: StateDeliveryContract = {
    version: 1,
    scope: "controlled-state-preview",
    native: {
      version: 1,
      mission: "test",
      mission_data_sha256: "a".repeat(64),
      level_data_sha256: "a".repeat(64),
      camera_elevation_deg: 35,
      scope: "map-art-and-listed-effects",
      background: frame,
      origin: [0, 0],
      elements: [e, native],
    },
    families: [
      {
        id: "trap",
        element_ids: ["cart", "actors"],
        background_ids: [],
        body_terminal_tick: 4,
        physical: { initial: { kind: "absent" }, applied: [binding("applied")], motion },
      },
    ],
  };
  return {
    contract,
    motion,
    component: motion.components[0]!,
    element: e,
    family: contract.families[0]!,
  };
}
