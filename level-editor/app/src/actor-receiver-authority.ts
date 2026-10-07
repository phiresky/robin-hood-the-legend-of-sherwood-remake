import * as THREE from "three";
import { assetNodeKey, type Level3D } from "@rle/shared";
import {
  evaluatePhysicalReceiver,
  type PhysicalReceiverTriangle,
} from "./actor-shadow-receivers.ts";

export interface ActorReceiverAuthority {
  asset: string;
  model_sha256: string;
  physicalParts: readonly { node: string; meshes: readonly string[] }[];
}
/** Explicit audited physical meshes, separate from clipped contact appearances. */
export const northBankReceiverAuthority: ActorReceiverAuthority = {
  asset: "croisement02-north-woodland-bank",
  model_sha256: "14b11ad48d296b2a7ffcb76ac686ac9e52dd865a9b402619a25c431c13131829",
  physicalParts: Array.from({ length: 5 }, (_, i) => ({
    node: `building-${String(i).padStart(3, "0")}`,
    meshes: [`North Woodland Bank / North Woodland Bank part ${String(i).padStart(3, "0")}`],
  })),
};

export function currentReceiverMeshes(
  document: Level3D,
  placement: string,
  authority: ActorReceiverAuthority,
  meshesForPart: (id: string) => readonly THREE.Mesh[] | undefined,
) {
  const source = document.assetSources?.find((a) => a.id === authority.asset);
  if (!source || source.model_sha256 !== authority.model_sha256)
    throw new Error("Physical receiver model does not match its reviewed source");
  if (!document.groups.some((g) => g.id === placement && !g.hidden))
    throw new Error("Receiver placement is missing");
  const result: { id: string; mesh: THREE.Mesh }[] = [],
    seen = new Set<THREE.Mesh>();
  for (const part of authority.physicalParts) {
    const placements = document.objects.filter(
      (o) => o.group === placement && o.node === assetNodeKey(authority.asset, part.node),
    );
    if (placements.length !== 1 || placements[0]!.hidden)
      throw new Error("Physical receiver part is missing or hidden");
    const object = placements[0]!,
      meshes = meshesForPart(object.id);
    if (!meshes) throw new Error("Physical receiver meshes are not ready");
    for (const name of part.meshes) {
      const matches = meshes.filter((m) => m.name === name);
      if (matches.length !== 1 || !matches[0]!.visible || seen.has(matches[0]!))
        throw new Error("Physical receiver mesh identity is ambiguous or hidden");
      const mesh = matches[0]!;
      seen.add(mesh);
      mesh.updateWorldMatrix(true, false);
      result.push({ id: `${object.id}/${name}`, mesh });
    }
  }
  if (!result.length) throw new Error("Physical receiver authority is empty");
  const revision = JSON.stringify([
    authority.model_sha256,
    document.camera,
    result.map(({ id, mesh }) => [
      id,
      mesh.geometry.uuid,
      attributeVersion(mesh.geometry.getAttribute("position")),
      mesh.geometry.index?.version,
      mesh.matrixWorld.elements,
    ]),
  ]);
  return {
    revision,
    evaluate: () =>
      result.flatMap(({ id, mesh }): PhysicalReceiverTriangle[] =>
        evaluatePhysicalReceiver(mesh, id, (document.camera.elevation_deg * Math.PI) / 180),
      ),
  };
}

function attributeVersion(attribute: THREE.BufferAttribute | THREE.InterleavedBufferAttribute) {
  return attribute instanceof THREE.InterleavedBufferAttribute
    ? attribute.data.version
    : attribute.version;
}
