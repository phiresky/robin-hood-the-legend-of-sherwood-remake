import type { SightObstacle } from "./level.ts";

export type AssetState = "initial" | "applied";

/** Explicit endpoint membership: canonical nodes in descriptors, object IDs in documents. */
export interface AssetStates {
  active: AssetState;
  initial: string[];
  applied: string[];
}

/** A standalone model exported from a reviewed map, with local obstacles. */
export interface ProjectionAssetDescriptor {
  /** Catalog metadata is authored here; index.json is generated from descriptors. */
  asset_type?: string;
  tags?: string[];
  editor_usage?: "map-background";
  version: 1;
  kind: "projection-mapped-asset";
  id: string;
  name: string;
  source_map: string;
  /** Export pivot in the source scene; used when replacing this asset revision. */
  source_origin_scene?: [number, number, number];
  model: string;
  /** Exact named scene in a shared multi-state GLB. */
  model_scene?: string;
  /** External resources use library-root-relative paths. */
  resources?: { path: string; sha256: string }[];
  states?: AssetStates;
  /** Independent static models sharing an origin; these do not imply animation. */
  state_variants?: Partial<
    Record<
      AssetState,
      {
        name: string;
        model: string;
        model_scene?: string;
        parts?: ProjectionAssetDescriptor["parts"];
      }
    >
  >;
  /** Additional complete appearances; the primary model remains separately insertable. */
  standalone_variants?: ProjectionAssetDescriptor["state_variants"];
  parts: ({
    node: string;
    name: string;
    default_hidden?: boolean;
    /** Asset-local appearance controls for this complete visual part. */
    appearance?: Pick<import("./patch-bindings.ts").PatchBinding, "hide" | "show">;
    /** Non-rendering coordinate frame for asset-local gameplay. */
    gameplay_only?: true;
    /** Keep visual component bounds without compiling them as physical obstacles. */
    collision?: "none";
    /** Directed local bottom edges joining flat, static physical volumes after placement. */
    sight_join_edges?: [import("./scene.ts").Vec3, import("./scene.ts").Vec3][];
    /** Whole horizontal faces that may join an adjacent stacked physical volume. */
    sight_join_caps?: ("top" | "bottom")[];
  } & (
    | {
        source_obstacle: number;
        source_components?: string[];
        mission_profile?: never;
        scenery?: never;
        obstacle_local_game: SightObstacle;
      }
    | {
        source_obstacle?: never;
        source_components?: never;
        /** Preview appearance reference. obstacle_local_game is editor bounds,
         * not collision or navigation; gameplay must be authored separately. */
        mission_profile: string;
        scenery?: never;
        obstacle_local_game: SightObstacle;
      }
    | {
        /** Authored scenery (`foliage-*`/`scenery-*`): visual only, the game has no obstacle. */
        source_obstacle?: never;
        source_components?: never;
        mission_profile?: never;
        scenery: true;
        obstacle_local_game?: never;
      }
  ))[];
}

export interface ProjectionAssetEntry {
  /** Generated editor view of the descriptor; the full descriptor is publication evidence. */
  editor?: ProjectionAssetDescriptor;
  descriptor_sha256?: string;
  /** Verified source hash supplied by deployments that omit original model bytes. */
  model_sha256?: string;
  asset_type?: string;
  tags?: string[];
  state_variant?: AssetState;
  editor_usage?: "map-background";
  id: string;
  name: string;
  source_map: string;
  descriptor: string;
  model: string;
  /** Exact named scene in a shared multi-state GLB. */
  model_scene?: string;
  /** Optional lightweight model used only by the asset browser preview. */
  preview_model?: string;
  /** Optional derived model for display, validated against `model` during index publication. */
  lossy_model?: string;
}

/** Paths are relative to the granted library root; hashes pin saved instances. */
export interface ExternalAssetSource {
  resources?: { path: string; sha256: string }[];
  state_variant?: AssetState;
  id: string;
  descriptor: string;
  model: string;
  /** Exact named scene in a shared multi-state GLB. */
  model_scene?: string;
  descriptor_sha256: string;
  model_sha256: string;
}

export function assetVariantId(id: string, variant: AssetState): string {
  return `${id}--state-${variant}`;
}

export function assetNodeKey(id: string, node: string): string {
  return `asset:${id}:${node}`;
}

export function safeLibraryPath(value: unknown): value is string {
  return (
    typeof value === "string" &&
    value.length > 0 &&
    !/[\\\0:#?%]/.test(value) &&
    value.split("/").every((part) => part !== "" && part !== "." && part !== "..")
  );
}
