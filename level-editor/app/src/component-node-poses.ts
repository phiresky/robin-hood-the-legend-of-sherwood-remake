import * as THREE from "three";
import { ReviewedNodePoses } from "./reviewed-node-poses.ts";
import type { PhysicalElementComponentPoses } from "../../shared/src/state-delivery.ts";

type Selection = { component_id: string; element_id: string; node_frame: number };

/** Separate render roots preserve native layer order while components retain their resources. */
export class ComponentNodePoses {
  readonly roots = new Map<string, THREE.Group>();
  private rigs = new Map<
    string,
    Map<
      string,
      {
        poses: ReviewedNodePoses;
        frames: number;
        meshes: { mesh: THREE.Object3D; visible: boolean }[];
      }
    >
  >();

  constructor(applied: THREE.Group, motion: PhysicalElementComponentPoses) {
    const all = motion.components.flatMap((c) =>
      c.stages.flatMap((s) => s.node_frames).find((f) => f !== null)!,
    );
    // Check every component together, including ambiguous names and nested mesh ownership.
    new ReviewedNodePoses(applied, [all]);
    const names = new Set(all.map((p) => p.node));
    applied.traverse((object) => {
      if ((object as THREE.Mesh).isMesh && !names.has(object.name))
        throw new Error(`Unowned physical component mesh: ${object.name}`);
    });
    for (const owner of new Set(
      motion.components.flatMap((c) => c.stages.map((s) => s.element_id)),
    )) {
      const root = applied.clone(true);
      root.visible = false;
      root.traverse((object) => {
        if ((object as THREE.Mesh).isMesh) object.visible = false;
      });
      const rigs = new Map<
        string,
        {
          poses: ReviewedNodePoses;
          frames: number;
          meshes: { mesh: THREE.Object3D; visible: boolean }[];
        }
      >();
      for (const component of motion.components) {
        if (!component.stages.some((s) => s.element_id === owner)) continue;
        const frames = component.stages.flatMap((s) => s.node_frames.filter((f) => f !== null));
        rigs.set(component.id, {
          poses: new ReviewedNodePoses(root, frames),
          frames: frames.length,
          meshes: frames[0]!.map((p) => ({
            mesh: root.getObjectByName(p.node)!,
            visible: applied.getObjectByName(p.node)!.visible,
          })),
        });
      }
      this.roots.set(owner, root);
      this.rigs.set(owner, rigs);
    }
  }

  apply(selections: readonly Selection[]) {
    const seen = new Set<string>();
    for (const selected of selections) {
      const rig = this.rigs.get(selected.element_id)?.get(selected.component_id);
      if (
        !rig ||
        seen.has(selected.component_id) ||
        !Number.isSafeInteger(selected.node_frame) ||
        selected.node_frame < 0 ||
        selected.node_frame >= rig.frames
      )
        throw new Error("Invalid or overlapping physical component selection");
      seen.add(selected.component_id);
    }
    this.reset();
    for (const selected of selections) {
      const rig = this.rigs.get(selected.element_id)!.get(selected.component_id)!;
      rig.poses.apply(selected.node_frame);
      for (const { mesh, visible } of rig.meshes) mesh.visible = visible;
      this.roots.get(selected.element_id)!.visible = true;
    }
  }

  reset() {
    for (const [owner, rigs] of this.rigs) {
      this.roots.get(owner)!.visible = false;
      for (const rig of rigs.values()) {
        rig.poses.reset();
        for (const { mesh } of rig.meshes) mesh.visible = false;
      }
    }
  }
}
