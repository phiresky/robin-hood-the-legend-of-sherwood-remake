import { StateRuntimeAdapter, type StateRuntimeBinding } from "./state-runtime-adapter.ts";
import { decodeRuntimeCapture, type RuntimeCaptureManifest } from "./state-runtime-observation.ts";
import type { StateDeliveryContract } from "./state-delivery.ts";
import type { StateClockBatch } from "./state-event-clocks.ts";

/** One loaded mission's observed clocks; the sink is the existing presentation entry point. */
export class RuntimeObservationSession {
  private readonly contract: StateDeliveryContract;
  private readonly bindings: StateRuntimeBinding[];
  private readonly adapter: StateRuntimeAdapter;
  private busy = false;
  private invalid = false;
  private readonly sink: (batch: StateClockBatch) => void;
  constructor(
    contract: StateDeliveryContract,
    bindings: StateRuntimeBinding[],
    sink: (batch: StateClockBatch) => void,
  ) {
    this.sink = sink;
    this.contract = structuredClone(contract);
    this.bindings = structuredClone(bindings);
    this.adapter = new StateRuntimeAdapter(this.contract, this.bindings);
  }
  invalidate() {
    this.invalid = true;
  }
  async deliver(
    engine: Uint8Array,
    assets: Uint8Array,
    manifest: RuntimeCaptureManifest,
  ): Promise<StateClockBatch> {
    if (this.invalid) throw new Error("Observation session requires reload");
    if (this.busy) throw new Error("Observation delivery already in progress");
    this.busy = true;
    try {
      const observation = await decodeRuntimeCapture(
        engine,
        assets,
        manifest,
        this.contract.native,
        this.bindings,
      );
      if (this.invalid) throw new Error("Observation session requires reload");
      const batch = this.adapter.sample(observation);
      try {
        this.sink(structuredClone(batch));
      } catch (error) {
        // The adapter has committed its sequence. Do not continue with a divergent sink.
        this.invalid = true;
        throw error;
      }
      return batch;
    } finally {
      this.busy = false;
    }
  }
}
