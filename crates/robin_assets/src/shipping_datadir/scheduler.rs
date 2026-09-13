//! Shipping scheduler boundary; payload wire shapes remain in the parent.
//!
//! One generic [`DecodeScheduler`] owns the dispatch loop and completion
//! handling for both sprite codecs; each codec supplies only its policy
//! (ordering, readiness, removal, preparation, decode) via [`DecodeCodec`].
#[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
use super::sprite_bank::VqChunkDecodeInputs;
use super::*;

/// Byte-weighted downstream paths among the chunks currently known to the
/// loader. Sprite IDs, rather than RHS names, distinguish restart groups.
/// Missing providers can still be in flight or not fetched yet. This only
/// chooses dispatch order; the bank's readiness checks remain authoritative.
#[cfg(any(test, all(target_arch = "wasm32", feature = "wasm-threads")))]
pub(super) fn vq_downstream_costs(chunks: &[SpriteVqChunk]) -> Vec<u64> {
    use std::collections::{HashMap, HashSet};
    let mut providers: HashMap<u32, Vec<usize>> = HashMap::new();
    for (index, chunk) in chunks.iter().enumerate() {
        for &id in &chunk.sprite_ids {
            providers.entry(id).or_default().push(index);
        }
    }
    let mut parents = vec![Vec::new(); chunks.len()];
    let mut children_left = vec![0usize; chunks.len()];
    for (child, chunk) in chunks.iter().enumerate() {
        let mut unique = HashSet::new();
        for id in chunk.base_ids.iter().chain(&chunk.base2_ids).flatten() {
            if let Some(indices) = providers.get(id) {
                for &parent in indices {
                    if parent != child && unique.insert(parent) {
                        parents[child].push(parent);
                        children_left[parent] += 1;
                    }
                }
            }
        }
    }
    let weights: Vec<u64> = chunks.iter().map(|chunk| chunk.blob.len() as u64).collect();
    let mut costs = weights.clone();
    let mut leaves: Vec<usize> = children_left
        .iter()
        .enumerate()
        .filter_map(|(index, &count)| (count == 0).then_some(index))
        .collect();
    while let Some(child) = leaves.pop() {
        for &parent in &parents[child] {
            costs[parent] = costs[parent].max(weights[parent].saturating_add(costs[child]));
            children_left[parent] -= 1;
            if children_left[parent] == 0 {
                leaves.push(parent);
            }
        }
    }
    // Duplicate providers can overapproximate the dependency graph. Keep a
    // finite priority for any cycle rather than rejecting otherwise valid
    // alternative providers; actual unresolved bases still fail at install.
    costs
}

/// Per-codec dispatch policy for [`DecodeScheduler`]. Implementations are
/// zero-sized markers; the scheduler never stores codec state.
#[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
pub trait DecodeCodec: 'static {
    type Chunk: Send + 'static;
    type Output: Send + 'static;
    /// Owned decode inputs built on the dispatching thread, moved to the worker.
    type Prepared: Send + 'static;
    /// Per-call dispatch arguments.
    type Args<'a>: Copy;
    /// Codec name used in error context and timing logs.
    const LABEL: &'static str;

    /// Reorder `pending` before the admission scan. `limit` is the bounded
    /// in-flight cap, `None` for unbounded dispatch.
    fn order(pending: &mut Vec<Self::Chunk>, args: Self::Args<'_>, limit: Option<usize>);
    fn ready(bank: &ShippingSpriteBank, chunk: &Self::Chunk, args: Self::Args<'_>) -> Result<bool>;
    /// Remove the ready chunk at `index` from `pending`.
    fn take(pending: &mut Vec<Self::Chunk>, index: usize, args: Self::Args<'_>) -> Self::Chunk;
    fn prepare(
        bank: &ShippingSpriteBank,
        chunk: &Self::Chunk,
        args: Self::Args<'_>,
    ) -> Result<Self::Prepared>;
    /// Runs on a pool worker.
    fn decode(
        chunk: &Self::Chunk,
        prepared: &Self::Prepared,
        limit: Option<usize>,
    ) -> Result<Self::Output>;
    fn rhs(chunk: &Self::Chunk) -> &str;
    fn first_sprite(chunk: &Self::Chunk) -> Option<&u32>;
}

/// Dispatcher state for worker-pool sprite chunk decode (wasm-threads builds).
///
/// Owns the set of in-flight decodes. The dispatching thread alternates
/// dispatching ready chunks onto the rayon pool with awaiting completions
/// ([`Self::next_decoded`]) and writing them into the bank, so dependents
/// unblock as soon as their providers land. The mission loader also drives
/// this incrementally while part files are still downloading.
///
/// Transient scheduling state, never serialized — deliberately no serde.
#[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
pub struct DecodeScheduler<C: DecodeCodec> {
    in_flight: super::decode_jobs::DecodeJobs<C::Chunk, C::Output>,
}

#[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
impl<C: DecodeCodec> Default for DecodeScheduler<C> {
    fn default() -> Self {
        Self {
            in_flight: Default::default(),
        }
    }
}

#[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
impl<C: DecodeCodec> DecodeScheduler<C> {
    /// Shared admission loop; the public per-codec wrappers pick `args`/`limit`.
    pub(super) fn dispatch_ready_with_limit(
        &mut self,
        bank: &ShippingSpriteBank,
        pending: &mut Vec<C::Chunk>,
        args: C::Args<'_>,
        limit: Option<usize>,
    ) -> Result<()> {
        let max_in_flight = limit.unwrap_or(usize::MAX);
        if self.in_flight.len() >= max_in_flight {
            return Ok(());
        }
        C::order(pending, args, limit);
        let mut index = 0;
        while index < pending.len() && self.in_flight.len() < max_in_flight {
            if !C::ready(bank, &pending[index], args)? {
                index += 1;
                continue;
            }
            let ready = tracing::enabled!(tracing::Level::DEBUG).then(js_sys::Date::now);
            let chunk = C::take(pending, index, args);
            let prepared = C::prepare(bank, &chunk, args).with_context(|| {
                format!("decode {} sprite chunk for {}", C::LABEL, C::rhs(&chunk))
            })?;
            self.in_flight.spawn(chunk, ready, move |chunk| {
                C::decode(chunk, &prepared, limit)
            });
        }
        Ok(())
    }

    /// Await the next completed decode. `Ok(None)` when no decode is in
    /// flight. Cancel-safe: dropping the returned future before completion
    /// loses nothing (the mission loader races this against part fetches).
    pub async fn next_decoded(&mut self) -> Result<Option<(C::Chunk, C::Output)>> {
        let Some((chunk, output, timing)) = self.in_flight.next(C::LABEL).await? else {
            return Ok(None);
        };
        let output = output
            .with_context(|| format!("decode {} sprite chunk for {}", C::LABEL, C::rhs(&chunk)))?;
        if let Some([ready_ms, enqueued_ms, worker_start_ms, worker_end_ms]) = timing {
            let received_ms = js_sys::Date::now();
            tracing::debug!(chunk = %C::rhs(&chunk), first_sprite = ?C::first_sprite(&chunk), ready_ms, enqueued_ms, worker_start_ms,
                worker_end_ms, received_ms, decode_ms = worker_end_ms - worker_start_ms,
                "{} sprite chunk decoded on worker", C::LABEL);
        }
        Ok(Some((chunk, output)))
    }

    /// Includes completed results until the caller consumes them.
    pub fn in_flight_count(&self) -> usize {
        self.in_flight.len()
    }

    /// True while at least one decode is running on the pool.
    pub fn has_in_flight(&self) -> bool {
        !self.in_flight.is_empty()
    }
}

/// VQ chunk decode: chunks depend on family base grids materialized by
/// other chunks, so readiness consults the bank.
#[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
pub enum VqCodec {}

/// Per-call VQ dispatch arguments.
#[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
#[derive(Clone, Copy)]
pub struct VqDispatchArgs<'a> {
    rhs_files: &'a BTreeMap<String, RhsData>,
    strict: bool,
}

#[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
impl DecodeCodec for VqCodec {
    type Chunk = SpriteVqChunk;
    type Output = Vec<(u32, Vec<u16>)>;
    type Prepared = VqChunkDecodeInputs;
    type Args<'a> = VqDispatchArgs<'a>;
    const LABEL: &'static str = "VQ";

    fn order(pending: &mut Vec<SpriteVqChunk>, _args: VqDispatchArgs<'_>, limit: Option<usize>) {
        // Longest-first dispatch: rayon's injected queue is FIFO, so this
        // starts the biggest blobs (family hubs — the heads of the longest
        // dependency chains) before the small variants pile onto the
        // workers. Chunk decode time tracks blob size closely.
        if limit.is_some() {
            let costs = vq_downstream_costs(pending);
            let mut ranked: Vec<_> = pending.drain(..).zip(costs).collect();
            ranked.sort_by_key(|(chunk, cost)| std::cmp::Reverse((*cost, chunk.blob.len())));
            pending.extend(ranked.into_iter().map(|(chunk, _)| chunk));
        } else {
            pending.sort_by_key(|chunk| std::cmp::Reverse(chunk.blob.len()));
        }
    }

    /// With `strict` readiness a chunk naming a base sprite that is missing
    /// from the bank is a hard error (the full mission payload is present,
    /// so the manifest is broken); lenient readiness treats it as "not yet"
    /// — the row is still being fetched.
    fn ready(
        bank: &ShippingSpriteBank,
        chunk: &SpriteVqChunk,
        args: VqDispatchArgs<'_>,
    ) -> Result<bool> {
        if args.strict {
            bank.vq_chunk_bases_ready(chunk)
        } else {
            bank.vq_chunk_ready_lenient(chunk, args.rhs_files)
        }
    }

    fn take(
        pending: &mut Vec<SpriteVqChunk>,
        index: usize,
        _args: VqDispatchArgs<'_>,
    ) -> SpriteVqChunk {
        // Order-preserving removal (`swap_remove` would drag the
        // smallest chunk into the just-vacated slot and dispatch it
        // second). The list is tens of entries; O(n) shifting is noise.
        pending.remove(index)
    }

    fn prepare(
        bank: &ShippingSpriteBank,
        chunk: &SpriteVqChunk,
        args: VqDispatchArgs<'_>,
    ) -> Result<VqChunkDecodeInputs> {
        bank.prepare_vq_chunk_inputs(chunk, args.rhs_files)
    }

    fn decode(
        chunk: &SpriteVqChunk,
        prepared: &VqChunkDecodeInputs,
        _limit: Option<usize>,
    ) -> Result<Vec<(u32, Vec<u16>)>> {
        ShippingSpriteBank::run_vq_chunk_decode(chunk, prepared)
    }

    fn rhs(chunk: &SpriteVqChunk) -> &str {
        &chunk.rhs
    }

    fn first_sprite(chunk: &SpriteVqChunk) -> Option<&u32> {
        chunk.sprite_ids.first()
    }
}

/// Worker-pool VQ chunk decode scheduler.
#[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
pub type VqDecodeScheduler = DecodeScheduler<VqCodec>;

#[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
impl DecodeScheduler<VqCodec> {
    /// Move every chunk of `pending` whose base grids are materialized onto
    /// the worker pool. See [`VqCodec::ready`] for `strict`.
    pub fn dispatch_ready(
        &mut self,
        bank: &ShippingSpriteBank,
        pending: &mut Vec<SpriteVqChunk>,
        rhs_files: &BTreeMap<String, RhsData>,
        strict: bool,
    ) -> Result<()> {
        self.dispatch_ready_with_limit(bank, pending, VqDispatchArgs { rhs_files, strict }, None)
    }

    /// Admit at most `max_in_flight` total jobs, keeping undispatched chunks
    /// available for reprioritization when another dependency part arrives.
    pub fn dispatch_ready_bounded(
        &mut self,
        bank: &ShippingSpriteBank,
        pending: &mut Vec<SpriteVqChunk>,
        rhs_files: &BTreeMap<String, RhsData>,
        strict: bool,
        max_in_flight: usize,
    ) -> Result<()> {
        self.dispatch_ready_with_limit(
            bank,
            pending,
            VqDispatchArgs { rhs_files, strict },
            Some(max_in_flight),
        )
    }

    /// Await the next completed decode and write its grids into the bank.
    /// `Ok(false)` when no decode is in flight.
    pub async fn apply_next(&mut self, bank: &mut ShippingSpriteBank) -> Result<bool> {
        let Some((chunk, grids)) = self.next_decoded().await? else {
            return Ok(false);
        };
        bank.apply_decoded_vq_chunk(&chunk, grids)?;
        Ok(true)
    }
}
