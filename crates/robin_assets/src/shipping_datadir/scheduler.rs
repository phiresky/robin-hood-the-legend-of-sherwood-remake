//! Shipping scheduler boundary; payload wire shapes remain in the parent.
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

/// Dispatcher state for worker-pool VQ chunk decode (wasm-threads builds).
///
/// Owns the set of in-flight decodes. The dispatching thread alternates
/// [`Self::dispatch_ready`] (move dependency-satisfied chunks onto the rayon
/// pool) with [`Self::apply_next`] (await one completion and write it into
/// the bank), so a family hub's variants unblock immediately when the hub
/// lands. The mission loader also drives this incrementally while part files
/// are still downloading.
///
/// Transient scheduling state, never serialized — deliberately no serde.
#[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
#[derive(Default)]
pub struct VqDecodeScheduler {
    in_flight: super::decode_jobs::DecodeJobs<SpriteVqChunk, Vec<(u32, Vec<u16>)>>,
}

#[cfg(all(target_arch = "wasm32", feature = "wasm-threads"))]
impl VqDecodeScheduler {
    /// Move every chunk of `pending` whose base grids are materialized onto
    /// the worker pool. With `strict` readiness a chunk naming a base sprite
    /// that is missing from the bank is a hard error (the full mission
    /// payload is present, so the manifest is broken); lenient readiness
    /// treats it as "not yet" — the row is still being fetched.
    pub fn dispatch_ready(
        &mut self,
        bank: &ShippingSpriteBank,
        pending: &mut Vec<SpriteVqChunk>,
        rhs_files: &BTreeMap<String, RhsData>,
        strict: bool,
    ) -> Result<()> {
        self.dispatch_ready_with_limit(bank, pending, rhs_files, strict, None)
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
        self.dispatch_ready_with_limit(bank, pending, rhs_files, strict, Some(max_in_flight))
    }

    fn dispatch_ready_with_limit(
        &mut self,
        bank: &ShippingSpriteBank,
        pending: &mut Vec<SpriteVqChunk>,
        rhs_files: &BTreeMap<String, RhsData>,
        strict: bool,
        limit: Option<usize>,
    ) -> Result<()> {
        let max_in_flight = limit.unwrap_or(usize::MAX);
        if self.in_flight.len() >= max_in_flight {
            return Ok(());
        }
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
        let mut index = 0;
        while index < pending.len() && self.in_flight.len() < max_in_flight {
            let ready = if strict {
                bank.vq_chunk_bases_ready(&pending[index])?
            } else {
                bank.vq_chunk_ready_lenient(&pending[index], rhs_files)?
            };
            if !ready {
                index += 1;
                continue;
            }
            // Order-preserving removal (`swap_remove` would drag the
            // smallest chunk into the just-vacated slot and dispatch it
            // second). The list is tens of entries; O(n) shifting is noise.
            let ready = tracing::enabled!(tracing::Level::DEBUG).then(js_sys::Date::now);
            let chunk = pending.remove(index);
            let inputs = bank
                .prepare_vq_chunk_inputs(&chunk, rhs_files)
                .with_context(|| format!("decode VQ sprite chunk for {}", chunk.rhs))?;
            self.in_flight.spawn(chunk, ready, move |chunk| {
                ShippingSpriteBank::run_vq_chunk_decode(chunk, &inputs)
            });
        }
        Ok(())
    }

    /// Await the next completed decode. `Ok(None)` when no decode is in
    /// flight. Cancel-safe: dropping the returned future before completion
    /// loses nothing (the mission loader races this against part fetches).
    pub async fn next_decoded(&mut self) -> Result<Option<(SpriteVqChunk, Vec<(u32, Vec<u16>)>)>> {
        let Some((chunk, grids, timing)) = self.in_flight.next("VQ").await? else {
            return Ok(None);
        };
        let grids = grids.with_context(|| format!("decode VQ sprite chunk for {}", chunk.rhs))?;
        if let Some([ready_ms, enqueued_ms, worker_start_ms, worker_end_ms]) = timing {
            let received_ms = js_sys::Date::now();
            tracing::debug!(chunk = %chunk.rhs, first_sprite = ?chunk.sprite_ids.first(), ready_ms, enqueued_ms, worker_start_ms,
                worker_end_ms, received_ms, decode_ms = worker_end_ms - worker_start_ms,
                "VQ sprite chunk decoded on worker");
        }
        Ok(Some((chunk, grids)))
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

    /// Includes completed results until the caller consumes them.
    pub fn in_flight_count(&self) -> usize {
        self.in_flight.len()
    }

    /// True while at least one decode is running on the pool.
    pub fn has_in_flight(&self) -> bool {
        !self.in_flight.is_empty()
    }
}
