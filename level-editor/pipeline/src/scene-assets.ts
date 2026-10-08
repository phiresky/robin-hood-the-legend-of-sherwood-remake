import fs from "node:fs/promises";
import path from "node:path";
import { createHash } from "node:crypto";
import { NodeIO } from "@gltf-transform/core";
import { ALL_EXTENSIONS } from "@gltf-transform/extensions";
import {
  safeLibraryPath,
  assetPartAppearanceExtras,
  parseProjectionAssetDescriptor,
  selectGlbScene,
  selectGltfScene,
  resolveGltfResources,
  patchBindingExtras,
  endpointPatchRule,
  remapPatchExtras,
  type SceneAssetSource,
  type Level3D,
} from "@rle/shared";

export async function readSceneAsset(
  library: string,
  reference: SceneAssetSource,
  verified = new Set<string>(),
) {
  const checked = async (name: string, expected: string) => {
    if (!safeLibraryPath(name)) throw new Error(`Unsafe asset path: ${name}`);
    const bytes = await fs.readFile(path.join(library, name));
    if (
      !verified.has(name + ":" + expected) &&
      createHash("sha256").update(bytes).digest("hex") !== expected
    )
      throw new Error(`Asset changed: ${name}`);
    verified.add(name + ":" + expected);
    return bytes;
  };
  let bytes = await checked(reference.model, reference.model_sha256);
  const descriptor = reference.descriptor
    ? JSON.parse((await checked(reference.descriptor, reference.descriptor_sha256!)).toString())
    : undefined;
  if (!reference.model.endsWith(".gltf"))
    bytes = Buffer.from(
      selectGlbScene(
        bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength),
        reference.model_scene,
        reference.model,
      ),
    );
  const json = reference.model.endsWith(".gltf")
    ? resolveGltfResources(
        reference.model,
        selectGltfScene(JSON.parse(bytes.toString()), reference.model_scene),
      )
    : JSON.parse(bytes.toString("utf8", 20, 20 + bytes.readUInt32LE(12)));
  const resources: Record<string, Uint8Array<ArrayBuffer>> = {};
  if (descriptor?.kind === "projection-mapped-asset") {
    const parts = parseProjectionAssetDescriptor(descriptor).parts;
    for (const node of json.nodes ?? []) {
      const part = parts.find((part) => part.node === node.name);
      if (part?.appearance)
        node.extras = assetPartAppearanceExtras(node.extras ?? {}, part.appearance);
    }
  }
  for (const resource of reference.resources)
    resources[resource.path] = new Uint8Array(await checked(resource.path, resource.sha256));
  return { json, resources, bytes };
}
export async function loadSceneModel(library: string, reference: SceneAssetSource) {
  const { json, resources, bytes } = await readSceneAsset(library, reference);
  const io = new NodeIO().registerExtensions(ALL_EXTENSIONS);
  if (reference.model.endsWith(".gltf")) return io.readJSON({ json, resources });
  // NodeIO's binaryToJSON rejects external GLB resources. Supply the checked
  // external files and embedded BIN together through its JSON interface.
  if ((json.buffers ?? []).some((buffer: { uri?: string }) => buffer.uri === undefined)) {
    const start = 20 + bytes.readUInt32LE(12);
    if (start + 8 > bytes.length || bytes.readUInt32LE(start + 4) !== 0x004e4942)
      throw new Error("Missing GLB binary chunk");
    const uri = "__embedded_glb_buffer__";
    resources[uri] = new Uint8Array(
      bytes.subarray(start + 8, start + 8 + bytes.readUInt32LE(start)),
    );
    for (const buffer of json.buffers) if (buffer.uri === undefined) buffer.uri = uri;
  }
  return io.readJSON({ json, resources });
}

/** Assemble only node metadata for ownership and native patch verification. */
export async function sceneAssetNodes(library: string, document: Level3D) {
  const verified = new Set<string>();
  const availableNodes = new Set<string>();
  const descriptors = new Set<string>();
  for (const source of document.assetSources ?? []) {
    if (!source.descriptor || descriptors.has(source.descriptor)) continue;
    if (!safeLibraryPath(source.descriptor))
      throw new Error(`Unsafe asset path: ${source.descriptor}`);
    const bytes = await fs.readFile(path.join(library, source.descriptor));
    if (createHash("sha256").update(bytes).digest("hex") !== source.descriptor_sha256)
      throw new Error(`Asset descriptor changed: ${source.id}`);
    const descriptor = JSON.parse(bytes.toString());
    for (const part of descriptor.parts ?? [])
      availableNodes.add(`asset:${descriptor.id}:${part.node}`);
    for (const [state, variant] of Object.entries(descriptor.state_variants ?? {}) as Array<
      [string, { parts?: { node: string }[] }]
    >)
      for (const part of variant.parts ?? descriptor.parts ?? [])
        availableNodes.add(`asset:${descriptor.id}--state-${state}:${part.node}`);
    descriptors.add(source.descriptor);
  }
  const nodes: any[] = [
    { name: "map", children: [], extras: structuredClone(document.sceneMetadata ?? {}) },
  ];
  for (const reference of [
    ...document.sceneAssets,
    ...(document.assetSources ?? []).map((ref) => ({
      ...ref,
      role: "objects" as const,
      resources: ref.resources ?? [],
    })),
  ]) {
    const resources = reference.resources.filter(
      (resource) => !verified.has(resource.path + ":" + resource.sha256),
    );
    const { json } = await readSceneAsset(library, { ...reference, resources }, verified);
    if (document.assetSources?.some((ref) => ref.id === reference.id)) {
      for (const part of document.objects.filter((part) =>
        part.node.startsWith(`asset:${reference.id}:`),
      )) {
        const name = part.node.slice(`asset:${reference.id}:`.length);
        for (const node of json.nodes ?? []) {
          const patches = part.group
            ? document.groups.find((group) => group.id === part.group)?.patches?.[reference.id]
            : part.patches?.[reference.id];
          if (patches) node.extras = remapPatchExtras(node.extras ?? {}, patches);
          if (node.name === name && part.group) {
            const group = document.groups.find((item) => item.id === part.group);
            const rule = endpointPatchRule(part.node, availableNodes, group?.patches);
            if (rule) node.extras = { ...node.extras, ...patchBindingExtras(rule) };
          }
          if (node.name === name) node.name = part.node;
        }
      }
    }
    const offset = nodes.length;
    nodes.push(
      ...(json.nodes ?? []).map((node: any) => ({
        ...node,
        ...(node.children
          ? { children: node.children.map((index: number) => index + offset) }
          : {}),
      })),
    );
    let roots: number[] = json.scenes[json.scene ?? 0].nodes ?? [];
    if (roots.length === 1 && json.nodes[roots[0]!].name === "map")
      roots = json.nodes[roots[0]!].children ?? [];
    nodes[0].children.push(...roots.map((index) => index + offset));
  }
  return { nodes };
}
