import * as THREE from "three";
import {
  parseStoredMap,
  serializeStoredMap,
  createTerrainGrid,
  type Level3D,
  type GameplayAssetDescriptor,
} from "@rle/shared";
import { listProjectionAssets, prepareProjectionPlacement } from "../src/projection-library.ts";
import { insertProjectionAsset } from "../src/asset-commands.ts";
import { disposeObjectResources } from "../src/resources.ts";
import { applyPlacementPatches, PatchDisplay } from "../src/patch-display.ts";
import { bindBakeAppearances } from "../src/map-appearance-bake.ts";
import { EditorViewport } from "../src/editor-viewport.ts";
import { packageCompiledMap } from "../src/map-compile.ts";

function check(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}
function canonical(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(canonical);
  if (value !== null && typeof value === "object")
    return Object.fromEntries(
      Object.entries(value)
        .sort(([a], [b]) => a.localeCompare(b))
        .map(([key, item]) => [key, canonical(item)]),
    );
  return value;
}
const result = document.querySelector("#result")!;
try {
  const parameters = new URLSearchParams(location.search);
  const fragments = parameters.has("fragments");
  const onlyAsset = parameters.get("asset");
  const base = parameters.get("library") ?? "/library/";
  const directory = (prefix: string): FileSystemDirectoryHandle =>
    ({
      getDirectoryHandle: async (name: string) => directory(`${prefix}${name}/`),
      getFileHandle: async (name: string) => ({
        getFile: async () => {
          const response = await fetch(`${base}${prefix}${name}`);
          if (!response.ok) throw new Error(`Missing model resource ${prefix}${name}`);
          return new File([await response.arrayBuffer()], name);
        },
      }),
    }) as unknown as FileSystemDirectoryHandle;
  const library = directory("");
  const catalog = await listProjectionAssets(library);
  const results: string[] = [];
  for (const id of fragments
    ? ["croisement01-group-083", "croisement01-group-084"]
    : [
        "leicester-east-moat-drawbridge",
        "leicester-east-village-drawbridge",
        "leicester-south-drawbridge",
      ]) {
    if (onlyAsset && id !== onlyAsset) continue;
    const entry = catalog.find((entry) => entry.id === id);
    check(entry, `Missing ${id}`);
    const prepared = await prepareProjectionPlacement(library, entry, "Authored map");
    let adopted = false;
    let current: Level3D = {
      version: 1,
      map: "Authored map",
      camera: { kind: "oblique-orthographic", elevation_deg: 35 },
      size: [2000, 2000],
      groups: [],
      objects: [],
      sceneAssets: [],
      ...(fragments ? { terrain: createTerrainGrid([0, 0, 1400, 1000], 250, 0) } : {}),
    };
    const viewport = new EditorViewport({
      document: () => current,
      selection: () => null,
      level: () => null,
      showObstacles: () => false,
      showElevation: () => false,
      onSelection: () => {},
      commitTransform: () => {},
    });
    try {
      check(prepared.additionalAssets.length === (fragments ? 0 : 1), "Unexpected endpoint models");
      viewport.replaceMap(new THREE.Group(), null, new Map());
      adopted = viewport.adoptAsset(
        prepared.reference,
        prepared.asset,
        prepared.sources,
        prepared.additionalAssets.map((member) => member.reference),
      );
      check(adopted, "New family must register");
      for (const x of [400, 1000])
        current = insertProjectionAsset(
          current,
          prepared.descriptor,
          prepared.reference,
          [x, 500, 0],
          prepared.additionalAssets,
          prepared.appearanceIds,
        ).document;
      if (fragments) current.groups[1]!.transform.rot_deg = 37;
      viewport.syncViews(current);
      const descriptors = new Map<string, GameplayAssetDescriptor>([
        [prepared.reference.id, prepared.descriptor],
        ...prepared.additionalAssets.map(
          (member) => [member.reference.id, member.descriptor] as const,
        ),
      ]);
      const reopened = parseStoredMap(serializeStoredMap(current, descriptors), descriptors);
      check(
        JSON.stringify(canonical(reopened)) === JSON.stringify(canonical(current)),
        "Placement round trip changed the document",
      );
      const root = new THREE.Group(),
        available = new Set(prepared.sources.keys());
      for (const part of current.objects) {
        const source = prepared.sources.get(part.node);
        check(source, `Missing placed model part ${part.node}`);
        const model = source.clone(true);
        applyPlacementPatches(model, current, part, available);
        const wrapper = new THREE.Group();
        wrapper.userData.map_bake_object_id = part.id;
        wrapper.userData.placement = part.group;
        wrapper.add(model);
        root.add(wrapper);
      }
      const transitions = current.groups.flatMap((group) =>
        descriptors.get(id)!.gameplay!.movementTransitions!.map((transition) => ({
          id: `${group.id}/${id}/${transition.id}`,
        })),
      );
      bindBakeAppearances(root, current, descriptors, transitions);
      const display = new PatchDisplay();
      const flags = () => {
        const values: boolean[] = [];
        root.traverse((node) => values.push(node.visible));
        return values;
      };
      display.apply(root);
      const initial = flags();
      const secondFlags = () => {
        const values: boolean[] = [];
        for (const wrapper of root.children)
          if (wrapper.userData.placement === current.groups[1]!.id)
            wrapper.traverse((node) => values.push(node.visible));
        return JSON.stringify(values);
      };
      const secondInitial = secondFlags();
      const controller = descriptors
        .get(id)!
        .gameplay!.movementTransitions!.find((transition) =>
          transition.appearances?.includes(fragments ? "activate-fragment" : "state"),
        )!;
      display.set(`${current.groups[0]!.id}/${id}/${controller.id}`, true);
      display.apply(root);
      check(
        flags().some((flag, index) => flag !== initial[index]),
        `Applied endpoint did not change: ${JSON.stringify({ id, groups: current.groups, metadata: root.children.map((wrapper) => wrapper.children.map((node) => node.userData)) })}`,
      );
      check(secondFlags() === secondInitial, "Switching one placement changed its copy");
      display.clear();
      display.apply(root);
      check(
        JSON.stringify(flags()) === JSON.stringify(initial),
        "Endpoint reset changed visibility",
      );
      if (fragments) {
        for (const first of [false, true])
          for (const second of [false, true]) {
            display.set(`${current.groups[0]!.id}/${id}/${controller.id}`, first);
            display.set(`${current.groups[1]!.id}/${id}/${controller.id}`, second);
            display.apply(root);
            check(
              root.children[0]!.children[0]!.visible === first &&
                root.children[1]!.children[0]!.visible === second,
              "Fragment copies must support all independent control combinations",
            );
          }
        display.clear();
        display.apply(root);
        const baked = await viewport.bakeMapAsync(current, descriptors);
        const controls = baked.compiled.descriptor.asset_geometry?.movement_transitions ?? [];
        check(
          controls.length === 2 && controls.every((control) => control.has_appearance),
          `Both independent controls must export appearance bindings: ${JSON.stringify(baked.compiled.warnings)}`,
        );
        check(
          new Set(baked.appearance.flatMap((region) => region.patches)).size === 2,
          "Both copied appearances must be rendered",
        );
        for (const region of baked.appearance) {
          const initial = region.states[0]!;
          check(
            region.states
              .slice(1)
              .some(
                (state) =>
                  state.color.some((value, i) => value !== initial.color[i]) &&
                  state.depth.some((value, i) => value !== initial.depth[i]),
              ),
            "Applied fragment must change rendered color and depth",
          );
        }
        const zip = await packageCompiledMap(baked.compiled, baked.pixels, baked.appearance);
        check(zip.length > 0, "Appearance archive must package successfully");
        (window as unknown as { __bakeZip: Uint8Array }).__bakeZip = zip;
      }
      results.push(`${id}: ${current.objects.length} placed parts, both pins saved, reset exact`);
    } finally {
      viewport.dispose();
      if (!adopted) disposeObjectResources([prepared.asset]);
    }
  }
  result.textContent = `PASS ${results.join("; ")}`;
} catch (error) {
  result.textContent = `FAIL ${error instanceof Error ? error.stack : String(error)}`;
}
