import type { StateDeliveryContract } from "../../shared/src/state-delivery.ts";
import type { StateClockBatch } from "../../shared/src/state-event-clocks.ts";
import type { StateRuntimeBinding } from "../../shared/src/state-runtime-adapter.ts";
import type { RuntimeCaptureIdentity } from "../../shared/src/state-runtime-observation.ts";
import { RuntimeObservationSession } from "../../shared/src/state-runtime-observation-session.ts";

export interface RecordedCaptureManifest extends RuntimeCaptureIdentity {
  version: 1;
  kind: "recorded-state-observations";
  authority: {
    scope: "bound-state-metadata";
    receipt: { file: string; sha256: string };
    limitations: string[];
    binary_mission_sha256: string;
    binary_level_sha256: string;
  };
  bindings: StateRuntimeBinding[];
  assets: { file: string; sha256: string };
  snapshots: { file: string; sha256: string }[];
}
export interface CaptureFile {
  name: string;
  arrayBuffer(): Promise<ArrayBuffer>;
}

/** Explicit snapshots only: no host-clock interpolation or inferred gameplay events. */
export class RecordedStateCaptureSource {
  private invalid = false;
  private busy = false;
  private session: RuntimeObservationSession | null = null;
  private manifest: RecordedCaptureManifest | null = null;
  private files = new Map<string, CaptureFile>();
  private assets: Uint8Array | null = null;
  private cursor = 0;
  private readonly contract: StateDeliveryContract;
  private readonly sink: (batch: StateClockBatch) => void;
  constructor(contract: StateDeliveryContract, sink: (batch: StateClockBatch) => void) {
    this.sink = sink;
    this.contract = structuredClone(contract);
  }
  invalidate() {
    this.invalid = true;
    this.session?.invalidate();
  }
  private assertCurrent() {
    if (this.invalid) throw new Error("Recorded capture requires reload");
  }
  get remaining() {
    return (this.manifest?.snapshots.length ?? 0) - this.cursor;
  }
  get authority() {
    return this.manifest ? structuredClone(this.manifest.authority) : null;
  }
  async load(files: readonly CaptureFile[]) {
    this.assertCurrent();
    if (this.busy || this.manifest) throw new Error("Recorded capture already loaded or loading");
    this.busy = true;
    try {
      for (const file of files) {
        if (!/^[^/\\]+\.json$/.test(file.name) || this.files.has(file.name))
          throw new Error("Capture files require unique JSON basenames");
        this.files.set(file.name, file);
      }
      const descriptor = this.files.get("runtime-capture.json");
      if (!descriptor)
        throw new Error("Select runtime-capture.json and its capture files together");
      const m = JSON.parse(
        new TextDecoder("utf-8", { fatal: true }).decode(await descriptor.arrayBuffer()),
      ) as RecordedCaptureManifest;
      this.assertCurrent();
      const hash = (s: unknown) => typeof s === "string" && /^[a-f0-9]{64}$/.test(s);
      if (
        m.version !== 1 ||
        m.kind !== "recorded-state-observations" ||
        m.mission !== this.contract.native.mission ||
        m.mission_data_sha256 !== this.contract.native.mission_data_sha256 ||
        m.level_data_sha256 !== this.contract.native.level_data_sha256 ||
        m.authority?.scope !== "bound-state-metadata" ||
        !m.authority.receipt ||
        !Array.isArray(m.authority.limitations) ||
        !m.authority.limitations.length ||
        m.authority.limitations.some((s) => typeof s !== "string" || !s.trim()) ||
        !hash(m.authority.binary_mission_sha256) ||
        !hash(m.authority.binary_level_sha256) ||
        !Array.isArray(m.bindings) ||
        !Array.isArray(m.snapshots) ||
        !m.snapshots.length
      )
        throw new Error("Capture identity or scoped authority does not match the loaded preview");
      const names = new Set<string>();
      for (const row of [m.assets, m.authority.receipt, ...m.snapshots]) {
        if (
          !row ||
          !hash(row.sha256) ||
          typeof row.file !== "string" ||
          row.file === "runtime-capture.json" ||
          !this.files.has(row.file) ||
          names.has(row.file)
        )
          throw new Error("Capture inventory is missing, duplicated, or unpinned");
        names.add(row.file);
      }
      const receiptBytes = new Uint8Array(
        await this.files.get(m.authority.receipt.file)!.arrayBuffer(),
      );
      const digest = Array.from(
        new Uint8Array(await crypto.subtle.digest("SHA-256", receiptBytes)),
        (b) => b.toString(16).padStart(2, "0"),
      ).join("");
      if (digest !== m.authority.receipt.sha256)
        throw new Error("Capture qualification receipt hash mismatch");
      const receipt = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(receiptBytes));
      // This records the capture owner's reviewed, scoped assertion; it does not infer
      // full binary equivalence from a partial runtime snapshot.
      for (const key of ["mission", "mission_data_sha256", "level_data_sha256"] as const)
        if (receipt[key] !== m[key]) throw new Error("Capture qualification source mismatch");
      if (
        receipt.scope !== m.authority.scope ||
        receipt.binary_mission_sha256 !== m.authority.binary_mission_sha256 ||
        receipt.binary_level_sha256 !== m.authority.binary_level_sha256 ||
        JSON.stringify(receipt.bindings) !== JSON.stringify(m.bindings) ||
        JSON.stringify(receipt.limitations) !== JSON.stringify(m.authority.limitations)
      )
        throw new Error("Capture qualification scope mismatch");
      this.assertCurrent();
      const session = new RuntimeObservationSession(this.contract, m.bindings, this.sink);
      const assets = new Uint8Array(await this.files.get(m.assets.file)!.arrayBuffer());
      this.assertCurrent();
      this.session = session;
      this.assets = assets;
      this.manifest = structuredClone(m);
    } catch (error) {
      this.invalidate();
      throw error;
    } finally {
      this.busy = false;
    }
  }
  async next(): Promise<StateClockBatch> {
    this.assertCurrent();
    if (this.busy) throw new Error("Recorded capture delivery already in progress");
    const m = this.manifest,
      s = this.session,
      assets = this.assets;
    if (!m || !s || !assets) throw new Error("Recorded capture is not loaded");
    const snapshot = m.snapshots[this.cursor];
    if (!snapshot) throw new Error("No recorded snapshots remain");
    this.busy = true;
    try {
      const bytes = new Uint8Array(await this.files.get(snapshot.file)!.arrayBuffer());
      this.assertCurrent();
      const batch = await s.deliver(bytes, assets, {
        mission: m.mission,
        mission_data_sha256: m.mission_data_sha256,
        level_data_sha256: m.level_data_sha256,
        assets_sha256: m.assets.sha256,
        engine_sha256: snapshot.sha256,
      });
      this.cursor++;
      return batch;
    } catch (error) {
      this.invalidate();
      throw error;
    } finally {
      this.busy = false;
    }
  }
}
