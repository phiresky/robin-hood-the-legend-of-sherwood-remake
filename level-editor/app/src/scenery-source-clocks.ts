import type { SourceContractClockBinding } from "./source-contract-clock-binding.ts";
export interface ScenerySourceClockBinding {
  /** Exact placed part/animation key and descriptor pin; never derived from a display name. */
  effectId: string;
  descriptorSha256: string;
  sourceIndex: number;
  sourceSha256: string;
}
type Snapshot = ReturnType<SourceContractClockBinding["snapshot"]>;
/** Explicit reviewed placement correspondence. Resource loading cannot reset these cursors. */
export class ScenerySourceClocks {
  private bindings = new Map<string, ScenerySourceClockBinding>();
  private rows: Snapshot = [];
  clear() {
    this.bindings.clear();
    this.rows = [];
  }
  bind(bindings: readonly ScenerySourceClockBinding[]) {
    const next = new Map<string, ScenerySourceClockBinding>();
    for (const b of bindings) {
      if (
        !b.effectId ||
        next.has(b.effectId) ||
        !Number.isSafeInteger(b.sourceIndex) ||
        b.sourceIndex < 0 ||
        !/^[a-f0-9]{64}$/.test(b.descriptorSha256) ||
        !/^[a-f0-9]{64}$/.test(b.sourceSha256)
      )
        throw new Error("Invalid scenery source-clock correspondence");
      next.set(b.effectId, Object.freeze({ ...b }));
    }
    this.bindings = next;
  }
  sample(rows: Snapshot) {
    this.rows = rows;
  }
  frame(effectId: string, descriptorSha256: string | undefined, count: number) {
    const binding = this.bindings.get(effectId);
    if (!binding) return undefined;
    if (binding.descriptorSha256 !== descriptorSha256)
      throw new Error("Bound scenery descriptor changed");
    const matches = this.rows.filter(
      (r) =>
        r.source.kind === "map-animation" &&
        r.source.index === binding.sourceIndex &&
        r.source.sha256 === binding.sourceSha256,
    );
    if (matches.length !== 1) throw new Error("Missing or ambiguous scenery source clock");
    const row = matches[0]!;
    if (!row.active) return -1;
    if (!Number.isInteger(row.frame) || row.frame < 0 || row.frame >= count)
      throw new Error("Scenery resource frames differ from bound source clock");
    return row.frame;
  }
}
