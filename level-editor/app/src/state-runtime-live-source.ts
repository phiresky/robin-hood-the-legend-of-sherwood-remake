import type { StateDeliveryContract } from "../../shared/src/state-delivery.ts";
import type { StateClockBatch } from "../../shared/src/state-event-clocks.ts";
import { RuntimeObservationSession } from "../../shared/src/state-runtime-observation-session.ts";
import type { RecordedCaptureManifest } from "./state-runtime-capture-source.ts";
export interface LiveObservationManifest extends Omit<
  RecordedCaptureManifest,
  "kind" | "snapshots"
> {
  kind: "live-state-observations";
  session_id: string;
  first_engine_frame: number;
}
type Fetcher = (path: string, signal: AbortSignal) => Promise<Response>;
const digest = async (bytes: Uint8Array) =>
  Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", new Uint8Array(bytes))), (n) =>
    n.toString(16).padStart(2, "0"),
  ).join("");
/** Consumes an ordered producer feed; never invents missing simulation frames. */
export class LiveStateCaptureSource {
  private readonly abort = new AbortController();
  private session: RuntimeObservationSession | null = null;
  private manifest: LiveObservationManifest | null = null;
  private assets: Uint8Array | null = null;
  private sequence = 0;
  private busy = false;
  private ended = false;
  private readonly contract: StateDeliveryContract;
  private readonly sink: (batch: StateClockBatch) => void;
  private readonly fetcher: Fetcher;
  constructor(
    contract: StateDeliveryContract,
    sink: (batch: StateClockBatch) => void,
    fetcher: Fetcher = (path, signal) =>
      fetch("/runtime-observations/" + path, { signal, cache: "no-store" }),
  ) {
    this.contract = structuredClone(contract);
    this.sink = sink;
    this.fetcher = fetcher;
  }
  invalidate() {
    this.abort.abort();
    this.session?.invalidate();
  }
  get closed() {
    return this.ended;
  }
  get authority() {
    return this.manifest ? structuredClone(this.manifest.authority) : null;
  }
  private current() {
    if (this.abort.signal.aborted) throw Error("Live observation session requires reload");
  }
  private async get(path: string) {
    this.current();
    const r = await this.fetcher(path, this.abort.signal);
    this.current();
    if (!r.ok) throw Error(`Observation bridge ${path}: HTTP${r.status}`);
    return r;
  }
  async connect() {
    this.current();
    if (this.busy || this.manifest) throw Error("Observation connection already started");
    this.busy = true;
    try {
      const m = (await (await this.get("session")).json()) as LiveObservationManifest;
      const hash = (x: unknown) => typeof x === "string" && /^[a-f0-9]{64}$/.test(x);
      if (
        m.version !== 1 ||
        m.kind !== "live-state-observations" ||
        !/^[a-f0-9]{32}$/.test(m.session_id) ||
        !Number.isSafeInteger(m.first_engine_frame) ||
        m.first_engine_frame < 0 ||
        m.authority?.scope !== "bound-state-metadata" ||
        !hash(m.assets?.sha256) ||
        !hash(m.authority.receipt?.sha256) ||
        !hash(m.authority.binary_mission_sha256) ||
        !hash(m.authority.binary_level_sha256) ||
        !Array.isArray(m.authority.limitations) ||
        !m.authority.limitations.length ||
        m.authority.limitations.some((x) => typeof x !== "string" || !x.trim())
      )
        throw Error("Invalid live source authority");
      for (const k of ["mission", "mission_data_sha256", "level_data_sha256"] as const)
        if (m[k] !== this.contract.native[k]) throw Error("Live source identity differs");
      const assets = new Uint8Array(await (await this.get("assets")).arrayBuffer()),
        receiptBytes = new Uint8Array(await (await this.get("qualification")).arrayBuffer());
      if (
        (await digest(assets)) !== m.assets.sha256 ||
        (await digest(receiptBytes)) !== m.authority.receipt.sha256
      )
        throw Error("Live authority payload hash differs");
      const r = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(receiptBytes));
      for (const k of ["mission", "mission_data_sha256", "level_data_sha256"] as const)
        if (r[k] !== m[k]) throw Error("Live qualification source differs");
      if (
        r.scope !== m.authority.scope ||
        r.binary_mission_sha256 !== m.authority.binary_mission_sha256 ||
        r.binary_level_sha256 !== m.authority.binary_level_sha256 ||
        JSON.stringify(r.bindings) !== JSON.stringify(m.bindings) ||
        JSON.stringify(r.limitations) !== JSON.stringify(m.authority.limitations)
      )
        throw Error("Live qualification scope differs");
      this.current();
      this.session = new RuntimeObservationSession(this.contract, m.bindings, this.sink);
      this.assets = assets;
      this.manifest = structuredClone(m);
    } catch (e) {
      this.invalidate();
      throw e;
    } finally {
      this.busy = false;
    }
  }
  async next(): Promise<StateClockBatch | null> {
    this.current();
    if (this.busy) throw Error("Observation read already in progress");
    if (!this.manifest || !this.session || !this.assets)
      throw Error("Observation connection is not loaded");
    if (this.ended) return null;
    this.busy = true;
    try {
      const m = this.manifest,
        r = await this.get(`packet/${this.sequence}`);
      if (r.status === 204) return null;
      const p = await r.json();
      this.current();
      if (p.session_id !== m.session_id || p.sequence !== this.sequence)
        throw Error("Live session restarted or packet sequence differs");
      if (p.closed === true) {
        this.ended = true;
        return null;
      }
      if (
        p.engine_frame !== m.first_engine_frame + this.sequence ||
        typeof p.engine_text !== "string"
      )
        throw Error("Live engine frame gap or restart");
      const bytes = new TextEncoder().encode(p.engine_text),
        parsed = JSON.parse(p.engine_text);
      if (parsed.control?.frame_counter !== p.engine_frame)
        throw Error("Live packet frame differs from engine");
      const batch = await this.session.deliver(bytes, this.assets, {
        mission: m.mission,
        mission_data_sha256: m.mission_data_sha256,
        level_data_sha256: m.level_data_sha256,
        assets_sha256: m.assets.sha256,
        engine_sha256: p.engine_sha256,
      });
      this.current();
      this.sequence++;
      return batch;
    } catch (e) {
      this.invalidate();
      throw e;
    } finally {
      this.busy = false;
    }
  }
}
