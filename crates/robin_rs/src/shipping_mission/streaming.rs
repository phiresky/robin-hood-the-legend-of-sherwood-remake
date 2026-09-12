//! Browser worker-pool execution: fetch accounting, decode admission, and deferred publication.

use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use futures::StreamExt as _;
use robin_assets::shipping_datadir::{ShippingDatadir, ShippingMission, decode_mission_compressed};

use super::planning::{SpriteDeferral, streaming_worker_budget};
use super::{
    CompressedPayload, MISSION_FETCH_CONCURRENCY, MissionLoadPhase, MissionLoadProgress,
    canonical_relative_file_key, take_early_download,
};

/// Deferred sprite-chunk work handed to [`spawn_deferred_sprite_tail`] after
/// the mission activates: everything the background driver needs without
/// touching the installed (shared, immutable) mission payload.
pub(super) struct DeferredSpriteTail {
    /// Sparse row clone of the mission bank (rows share their grids via
    /// `Arc`, so this is cheap). Critical chunks are already materialized
    /// into these rows; the driver applies its own decodes here so
    /// dependent deferred chunks see their base grids.
    bank: robin_assets::shipping_datadir::ShippingSpriteBank,
    /// RHS metadata for deferred chunks that derive within-chunk
    /// self-references at decode time.
    rhs_files: std::collections::BTreeMap<String, robin_assets::shipping_datadir::RhsData>,
    chunks: Vec<robin_assets::shipping_datadir::SpriteVqChunk>,
}

/// Byte-progress shared between the concurrent part fetches and the install
/// loop's progress reporting. Part sizes are learned from each response's
/// `Content-Length` header (all requests go out at once, so every size is
/// known within the first round-trips) and corrected to the actual body
/// length on completion.
#[derive(Default)]
struct FetchByteProgress {
    /// Body bytes received so far, across all parts.
    received: std::sync::atomic::AtomicU64,
    /// Sum of the known (claimed or completed) part sizes.
    known_total: std::sync::atomic::AtomicU64,
    /// Number of parts whose size is known.
    known_files: std::sync::atomic::AtomicUsize,
    /// Largest known part size — the estimate for still-unknown parts.
    max_known: std::sync::atomic::AtomicU64,
}

impl FetchByteProgress {
    fn add_known(&self, bytes: u64) {
        use std::sync::atomic::Ordering;
        self.known_total.fetch_add(bytes, Ordering::Relaxed);
        self.known_files.fetch_add(1, Ordering::Relaxed);
        self.max_known.fetch_max(bytes, Ordering::Relaxed);
    }

    /// Estimated total bytes across `files_total` parts: known sizes plus
    /// the largest known size for each part whose headers are still in
    /// flight (a deliberate overestimate, so the fraction rises rather
    /// than retreats as real sizes arrive).
    fn estimated_total(&self, files_total: usize) -> u64 {
        use std::sync::atomic::Ordering;
        let known = self.known_total.load(Ordering::Relaxed);
        let unknown = files_total.saturating_sub(self.known_files.load(Ordering::Relaxed)) as u64;
        let per_file = self.max_known.load(Ordering::Relaxed).max(64 * 1024);
        known + unknown * per_file
    }
}

/// Weighted install-progress model: fetch progress by bytes received,
/// decode progress by critical VQ blob bytes materialized, combined into
/// one monotonic fraction reported as `completed`/`total` work units. The
/// only tuning constant is the relative per-byte cost of a blob decode
/// versus a network byte ([`Self::DECODE_BYTE_COST`], calibrated from the
/// measured install: decoding dominates the wall clock).
struct InstallWorkModel {
    files_total: usize,
    fetch: Arc<FetchByteProgress>,
    /// Critical (activation-blocking) VQ blob bytes discovered so far.
    decode_total: u64,
    /// Critical VQ blob bytes whose grids are materialized.
    decode_done: u64,
    /// Monotonic guard: totals grow while parts stream in, so the raw
    /// fraction can dip; never report backwards motion.
    emitted: f32,
    /// Last reported unit count — repeat emits (progress ticks) are
    /// suppressed so the loading log is not flooded with identical lines.
    last_units: usize,
}

impl InstallWorkModel {
    /// Measured on the browser install: context-model blob decode costs a
    /// few times more wall clock per byte than fetching a byte does.
    const DECODE_BYTE_COST: f64 = 6.0;
    /// Reported progress granularity (`completed` out of `total`).
    const UNITS: usize = 100;

    fn fraction(&self) -> f32 {
        use std::sync::atomic::Ordering;
        let received = self.fetch.received.load(Ordering::Relaxed) as f64;
        let fetch_total = self.fetch.estimated_total(self.files_total) as f64;
        let done = received + Self::DECODE_BYTE_COST * self.decode_done as f64;
        let total = (fetch_total + Self::DECODE_BYTE_COST * self.decode_total as f64).max(1.0);
        (done / total).min(1.0) as f32
    }

    fn emit<F>(&mut self, progress: &mut F, label: &str)
    where
        F: FnMut(MissionLoadProgress<'_>),
    {
        self.emitted = self.emitted.max(self.fraction());
        let completed = (self.emitted * Self::UNITS as f32).round() as usize;
        if completed == self.last_units {
            return;
        }
        self.last_units = completed;
        progress(MissionLoadProgress {
            phase: MissionLoadPhase::Data,
            completed,
            total: Self::UNITS,
            file: Some(label),
        });
    }
}

fn dispatch_streaming_chunks(
    bank: &robin_assets::shipping_datadir::ShippingSpriteBank,
    pending: &mut Vec<robin_assets::shipping_datadir::SpriteVqChunk>,
    scheduler: &mut robin_assets::shipping_datadir::VqDecodeScheduler,
    pending_rle: &mut Vec<robin_assets::shipping_datadir::SpriteRleJxlChunk>,
    rle_scheduler: &mut robin_assets::shipping_datadir::RleJxlDecodeScheduler,
    rhs_files: &std::collections::BTreeMap<String, robin_assets::shipping_datadir::RhsData>,
    bounded: bool,
    balanced: bool,
    fetching: bool,
    reserved_workers: usize,
) -> Result<()> {
    if !bounded {
        rle_scheduler.dispatch_ready(bank, pending_rle)?;
        return scheduler.dispatch_ready(bank, pending, rhs_files, !fetching);
    }
    let budget = streaming_worker_budget(
        robin_assets::wasm_threads::pool_threads(),
        fetching,
        reserved_workers,
    );
    // Keep one independent RLE job progressing even while VQ has a backlog.
    // The existing counts include completed-but-unapplied jobs, so this
    // reservation cannot overfill the shared admission budget.
    if balanced {
        rle_scheduler.dispatch_ready_prioritized(
            bank,
            pending_rle,
            budget.saturating_sub(scheduler.in_flight_count()).min(1),
        )?;
    }
    scheduler.dispatch_ready_bounded(
        bank,
        pending,
        rhs_files,
        !fetching,
        budget.saturating_sub(rle_scheduler.in_flight_count()),
    )?;
    let rle_limit = budget.saturating_sub(scheduler.in_flight_count());
    if balanced {
        rle_scheduler.dispatch_ready_prioritized(bank, pending_rle, rle_limit)
    } else {
        rle_scheduler.dispatch_ready_bounded(bank, pending_rle, rle_limit)
    }
}

/// Streaming mission load for the browser worker-pool build.
///
/// Every part request is issued simultaneously — the browser's network stack
/// multiplexes the actual transfers — and each response is processed the
/// moment it arrives: the zstd+bitcode part decode runs on a rayon worker
/// (inline on the serial fallback), the decoded part merges immediately, and
/// dependency-ready *critical* VQ sprite chunks are admitted within the
/// worker budget. The main thread never blocks: it awaits whichever event
/// completes next (a part arrival, a finished chunk decode, or a progress
/// tick) via `futures::select!`.
///
/// With a worker pool, chunks belonging to reinforcement-only gang
/// characters (see [`SpriteDeferral`]) are *not* decoded here: they return
/// as the third tuple element for [`spawn_deferred_sprite_tail`] to stream
/// after the mission activates. Without a pool everything stays blocking, as
/// before.
pub(super) async fn fetch_merge_materialize_streaming<F>(
    datadir: &ShippingDatadir,
    mission: &str,
    campaign: &robin_engine::campaign::Campaign,
    profiles: &robin_engine::profiles::ProfileManager,
    has_decoded_saved_world: bool,
    files: &[String],
    progress: &mut F,
    downloads_finished: &mut impl FnMut(),
) -> Result<(
    ShippingMission,
    usize,
    Option<DeferredSpriteTail>,
    Option<crate::level_loading_host::EarlyTerrainDecode>,
)>
where
    F: FnMut(MissionLoadProgress<'_>),
{
    use futures::FutureExt as _;
    use robin_assets::shipping_datadir::{
        RleJxlDecodeScheduler, ShippingSpriteBank, SpriteRleJxlChunk, SpriteVqChunk,
        VqDecodeScheduler,
    };
    use robin_assets::wasm_threads;

    let stream_start = web_time::Instant::now();
    tracing::info!(mission, "startup timing: mission streaming begin");
    let total = files.len();
    let pooled = wasm_threads::pool_threads() > 0;
    // Diagnostic switch for paired browser measurements, without rebuilding.
    let window =
        web_sys::window().ok_or_else(|| anyhow!("mission streaming requires a browser window"))?;
    let query = web_sys::UrlSearchParams::new_with_str(
        &window
            .location()
            .search()
            .map_err(|error| anyhow!("read startup query: {error:?}"))?,
    )
    .map_err(|error| anyhow!("parse startup query: {error:?}"))?;
    let (bounded, balanced) = match query.get("streaming-scheduler").as_deref() {
        None | Some("balanced") => (true, true),
        Some("bounded") => (true, false),
        Some("unbounded") => (false, false),
        Some(value) => return Err(anyhow!("unknown streaming-scheduler policy {value:?}")),
    };
    tracing::info!(
        bounded,
        balanced,
        workers = wasm_threads::pool_threads(),
        "mission worker scheduling policy"
    );
    let mut download_files = files.to_vec();
    let download_concurrency = match query.get("mission-downloads").as_deref() {
        // Keep the browser's connections busy while short decompression jobs
        // await worker/main-thread progress. Worker admission remains bounded
        // independently; limiting both queues delayed transfers in Chrome.
        None | Some("ordered") => total.max(1),
        Some("prioritized") => MISSION_FETCH_CONCURRENCY,
        Some("unbounded") => {
            // Restore the original alphabetical all-at-once policy exactly.
            download_files.sort();
            total.max(1)
        }
        Some(value) => return Err(anyhow!("unknown mission-downloads policy {value:?}")),
    };
    tracing::info!(download_concurrency, "mission download scheduling policy");
    let fetch_progress = Arc::new(FetchByteProgress::default());
    let mut work = InstallWorkModel {
        files_total: total,
        fetch: Arc::clone(&fetch_progress),
        decode_total: 0,
        decode_done: 0,
        emitted: 0.0,
        last_units: usize::MAX,
    };
    // A decoded saved world can contain any entity — reinforcements that
    // already spawned included — so "present at mission start" cannot be
    // derived from the authored level: keep every chunk activation-blocking.
    let mut deferral = if pooled && !has_decoded_saved_world {
        Some(SpriteDeferral::new(datadir, mission, campaign, profiles)?)
    } else {
        None
    };
    let mut fetched = futures::stream::iter(download_files.into_iter().map(|file| {
        let fetch_progress = Arc::clone(&fetch_progress);
        async move {
            let compressed = fetch_counted(datadir, &file, &fetch_progress)
                .await
                .with_context(|| format!("fetch shipping file {file}"))?;
            let bytes = compressed.len();
            let ready = tracing::enabled!(tracing::Level::DEBUG).then(js_sys::Date::now);
            // Pure compute; overlap it with the remaining downloads when the
            // pool exists. Carry worker timestamps back to the main thread;
            // the browser workers do not necessarily have a log subscriber.
            let decode = move || {
                let worker_start = ready.map(|_| js_sys::Date::now());
                let payload = decode_mission_compressed(&compressed);
                let worker_end = ready.map(|_| js_sys::Date::now());
                (payload, worker_start.zip(worker_end))
            };
            let enqueued = ready.map(|_| js_sys::Date::now());
            let (payload, timing) = if wasm_threads::pool_threads() > 0 {
                wasm_threads::run_on_pool(decode).await?
            } else {
                decode()
            };
            if let Some(((ready_ms, enqueued_ms), (worker_start_ms, worker_end_ms))) =
                ready.zip(enqueued).zip(timing)
            {
                tracing::debug!(
                    file,
                    ready_ms,
                    enqueued_ms,
                    worker_start_ms,
                    worker_end_ms,
                    received_ms = js_sys::Date::now(),
                    "shipping part decoded on worker"
                );
            }
            let payload = payload.with_context(|| format!("decode shipping file {file}"))?;
            Ok::<_, anyhow::Error>((file, bytes, payload))
        }
    }))
    .buffer_unordered(download_concurrency)
    .fuse();

    enum Event {
        Part(Option<Result<(String, usize, ShippingMission)>>),
        Decoded(Result<Option<(SpriteVqChunk, Vec<(u32, Vec<u16>)>)>>),
        RleDecoded(
            Result<
                Option<(
                    SpriteRleJxlChunk,
                    Vec<(u32, robin_assets::frame_holder::SpriteRaster)>,
                )>,
            >,
        ),
        Tick,
    }

    let mut merged = ShippingMission::default();
    let mut early_terrain: Option<crate::level_loading_host::EarlyTerrainDecode> = None;
    let mut fetched_bytes = 0usize;
    let mut pending_chunks: Vec<SpriteVqChunk> = Vec::new();
    let mut scheduler = VqDecodeScheduler::default();
    let mut label = String::from("downloading mission parts");
    // RLE-JXL chunks (web recipe) have no cross-chunk dependencies, so each
    // is dispatched to the pool the moment its part merges; results are
    // applied in the drain phase below. On the serial fallback they stay in
    // the bank and `install_mission` decodes them.
    let mut pending_rle: Vec<SpriteRleJxlChunk> = Vec::new();
    let mut rle_scheduler = RleJxlDecodeScheduler::default();
    loop {
        // Boxed: `select!` polls through `&mut`, which needs `Unpin`, and an
        // `async fn` future is not. One small allocation per event is noise
        // next to a network fetch or chunk decode.
        let mut tick = Box::pin(crate::window::sleep_ms(150)).fuse();
        let event = {
            let mut next_decoded = Box::pin(async {
                if pooled && scheduler.has_in_flight() {
                    scheduler.next_decoded().await
                } else {
                    futures::future::pending().await
                }
            })
            .fuse();
            let mut next_rle = Box::pin(async {
                if bounded && pooled && rle_scheduler.has_in_flight() {
                    rle_scheduler.next_decoded().await
                } else {
                    futures::future::pending().await
                }
            })
            .fuse();
            futures::select! {
                part = fetched.next() => Event::Part(part),
                decoded = next_decoded => Event::Decoded(decoded),
                decoded = next_rle => Event::RleDecoded(decoded),
                _ = tick => Event::Tick,
            }
        };
        match event {
            Event::Part(None) => break,
            Event::Part(Some(part)) => {
                let (file, bytes, payload) = part?;
                fetched_bytes += bytes;
                tracing::debug!(mission, file, bytes, "shipping mission dependency fetched");
                let apply_start = tracing::enabled!(tracing::Level::DEBUG).then(js_sys::Date::now);
                merged
                    .merge_part(payload)
                    .with_context(|| format!("merge shipping file {file}"))?;
                if let Some(apply_start_ms) = apply_start {
                    tracing::debug!(
                        file,
                        apply_start_ms,
                        apply_end_ms = js_sys::Date::now(),
                        "shipping part merged"
                    );
                }
                label = file;
                if pooled && early_terrain.is_none() {
                    early_terrain = crate::level_loading_host::EarlyTerrainDecode::try_start(
                        mission, datadir, &merged,
                    );
                }
                if let Some(bank) = merged.payload.sprite_bank.as_mut() {
                    let mut incoming = std::mem::take(&mut bank.vq_chunks);
                    if tracing::enabled!(tracing::Level::DEBUG) {
                        let discovered_ms = js_sys::Date::now();
                        for chunk in &incoming {
                            tracing::debug!(chunk = %chunk.rhs, first_sprite = ?chunk.sprite_ids.first(),
                                bytes = chunk.blob.len(), discovered_ms, "VQ sprite chunk discovered");
                        }
                        for chunk in &bank.rle_jxl_chunks {
                            tracing::debug!(chunk = %chunk.rhs, first_sprite = ?chunk.sprite_ids.first(),
                                bytes = chunk.jxl_blobs.iter().map(Vec::len).sum::<usize>(),
                                discovered_ms, "RLE-JXL sprite chunk discovered");
                        }
                    }
                    if let Some(deferral) = deferral.as_mut() {
                        if !deferral.level_filtered
                            && let Some(level) = merged.payload.levels.get(mission)
                        {
                            deferral.exclude_level_requirements(
                                level,
                                profiles,
                                &mut pending_chunks,
                                &mut work.decode_total,
                            );
                        }
                        deferral.absorb(&mut incoming, &mut pending_chunks, &mut work.decode_total);
                    } else {
                        for chunk in &incoming {
                            work.decode_total += chunk.blob.len() as u64;
                        }
                        pending_chunks.append(&mut incoming);
                    }
                    let rhs_files = &merged.payload.rhs_files;
                    if pooled {
                        pending_rle.append(&mut bank.rle_jxl_chunks);
                        dispatch_streaming_chunks(
                            bank,
                            &mut pending_chunks,
                            &mut scheduler,
                            &mut pending_rle,
                            &mut rle_scheduler,
                            rhs_files,
                            bounded,
                            balanced,
                            true,
                            usize::from(
                                early_terrain.as_ref().is_some_and(|job| !job.is_finished()),
                            ),
                        )?;
                    } else {
                        // Serial fallback: no pool, but decode still overlaps
                        // the network by draining ready chunks between
                        // arrivals, yielding so the loading screen stays
                        // responsive between chunks.
                        let before: u64 = pending_chunks.iter().map(|c| c.blob.len() as u64).sum();
                        while bank
                            .materialize_next_ready_vq_chunk(&mut pending_chunks, rhs_files)?
                        {
                            crate::window::yield_to_runtime().await;
                        }
                        let after: u64 = pending_chunks.iter().map(|c| c.blob.len() as u64).sum();
                        work.decode_done += before - after;
                    }
                }
                work.emit(progress, &label);
                // Present the observer's progress frame.
                crate::window::yield_to_runtime().await;
            }
            Event::Decoded(item) => {
                let Some((chunk, grids)) = item? else {
                    continue;
                };
                let bank = merged
                    .payload
                    .sprite_bank
                    .as_mut()
                    .ok_or_else(|| anyhow!("decoded VQ chunk without a sprite bank"))?;
                bank.apply_decoded_vq_chunk(&chunk, grids)?;
                work.decode_done += chunk.blob.len() as u64;
                let rhs_files = &merged.payload.rhs_files;
                dispatch_streaming_chunks(
                    bank,
                    &mut pending_chunks,
                    &mut scheduler,
                    &mut pending_rle,
                    &mut rle_scheduler,
                    rhs_files,
                    bounded,
                    balanced,
                    true,
                    usize::from(early_terrain.as_ref().is_some_and(|job| !job.is_finished())),
                )?;
                work.emit(progress, &chunk.rhs);
            }
            Event::RleDecoded(item) => {
                let Some((chunk, rasters)) = item? else {
                    continue;
                };
                let bank = merged
                    .payload
                    .sprite_bank
                    .as_mut()
                    .ok_or_else(|| anyhow!("decoded RLE chunk without a sprite bank"))?;
                bank.apply_decoded_rle_jxl_chunk(&chunk, rasters)?;
                dispatch_streaming_chunks(
                    bank,
                    &mut pending_chunks,
                    &mut scheduler,
                    &mut pending_rle,
                    &mut rle_scheduler,
                    &merged.payload.rhs_files,
                    bounded,
                    balanced,
                    true,
                    usize::from(early_terrain.as_ref().is_some_and(|job| !job.is_finished())),
                )?;
            }
            Event::Tick => {
                // Real byte progress accrued inside the concurrent body
                // reads; surface it even while no part has completed.
                work.emit(progress, &label);
                crate::window::yield_to_runtime().await;
                if bounded
                    && pooled
                    && let Some(bank) = merged.payload.sprite_bank.as_ref()
                {
                    dispatch_streaming_chunks(
                        bank,
                        &mut pending_chunks,
                        &mut scheduler,
                        &mut pending_rle,
                        &mut rle_scheduler,
                        &merged.payload.rhs_files,
                        bounded,
                        balanced,
                        true,
                        usize::from(early_terrain.as_ref().is_some_and(|job| !job.is_finished())),
                    )?;
                }
            }
        }
    }
    tracing::info!(
        mission,
        elapsed_ms = stream_start.elapsed().as_secs_f64() * 1000.0,
        "startup timing: all parts merged"
    );
    downloads_finished();
    let vq_drain_start = web_time::Instant::now();
    if let Some(bank) = merged.payload.sprite_bank.as_mut() {
        // The level part has merged by now on any well-formed payload
        // (installation fails later otherwise); make sure its start-entity
        // requirements were applied before the final partition is fixed.
        if let Some(deferral) = deferral.as_mut()
            && !deferral.level_filtered
            && let Some(level) = merged.payload.levels.get(mission)
        {
            deferral.exclude_level_requirements(
                level,
                profiles,
                &mut pending_chunks,
                &mut work.decode_total,
            );
        }
        let rhs_files = &merged.payload.rhs_files;
        if bounded && pooled {
            loop {
                dispatch_streaming_chunks(
                    bank,
                    &mut pending_chunks,
                    &mut scheduler,
                    &mut pending_rle,
                    &mut rle_scheduler,
                    rhs_files,
                    true,
                    balanced,
                    false,
                    usize::from(early_terrain.as_ref().is_some_and(|job| !job.is_finished())),
                )?;
                if !scheduler.has_in_flight() && !rle_scheduler.has_in_flight() {
                    if (!pending_chunks.is_empty() || !pending_rle.is_empty())
                        && early_terrain.as_ref().is_some_and(|job| !job.is_finished())
                    {
                        crate::window::sleep_ms(10).await;
                        continue;
                    }
                    break;
                }
                let event = {
                    let mut terrain_tick = Box::pin(async {
                        if early_terrain.as_ref().is_some_and(|job| !job.is_finished()) {
                            crate::window::sleep_ms(10).await;
                        } else {
                            futures::future::pending().await
                        }
                    })
                    .fuse();
                    let mut next_vq = Box::pin(async {
                        if scheduler.has_in_flight() {
                            scheduler.next_decoded().await
                        } else {
                            futures::future::pending().await
                        }
                    })
                    .fuse();
                    let mut next_rle = Box::pin(async {
                        if rle_scheduler.has_in_flight() {
                            rle_scheduler.next_decoded().await
                        } else {
                            futures::future::pending().await
                        }
                    })
                    .fuse();
                    futures::select! {
                        item = next_vq => Event::Decoded(item),
                        item = next_rle => Event::RleDecoded(item),
                        _ = terrain_tick => Event::Tick,
                    }
                };
                match event {
                    Event::Decoded(item) => {
                        if let Some((chunk, grids)) = item? {
                            bank.apply_decoded_vq_chunk(&chunk, grids)?;
                            work.decode_done += chunk.blob.len() as u64;
                            work.emit(progress, &chunk.rhs);
                        }
                    }
                    Event::RleDecoded(item) => {
                        if let Some((chunk, rasters)) = item? {
                            bank.apply_decoded_rle_jxl_chunk(&chunk, rasters)?;
                        }
                    }
                    Event::Tick => {}
                    _ => {
                        unreachable!("drain only polls sprite worker results and terrain readiness")
                    }
                }
            }
        } else {
            // Drain outstanding worker decodes; each applied chunk can unlock
            // dependents that were still pending.
            while let Some((chunk, grids)) = scheduler.next_decoded().await? {
                bank.apply_decoded_vq_chunk(&chunk, grids)?;
                work.decode_done += chunk.blob.len() as u64;
                scheduler.dispatch_ready(bank, &mut pending_chunks, rhs_files, false)?;
                work.emit(progress, &chunk.rhs);
            }
        }
        // Strict pass for the critical remainder: with the whole payload
        // merged, "not fetched yet" is no longer an excuse, so unresolved
        // dependencies now surface as real manifest errors.
        if pooled {
            while !bounded {
                scheduler.dispatch_ready(bank, &mut pending_chunks, rhs_files, true)?;
                let Some((chunk, grids)) = scheduler.next_decoded().await? else {
                    break;
                };
                bank.apply_decoded_vq_chunk(&chunk, grids)?;
                work.decode_done += chunk.blob.len() as u64;
                work.emit(progress, &chunk.rhs);
            }
            if !pending_chunks.is_empty() {
                let stuck: Vec<&str> = pending_chunks
                    .iter()
                    .map(|chunk| chunk.rhs.as_str())
                    .collect();
                return Err(anyhow!(
                    "critical VQ sprite chunks cannot be decoded — base sprites never \
                     materialized: {}",
                    stuck.join(", ")
                ));
            }
        } else {
            bank.vq_chunks.append(&mut pending_chunks);
            bank.materialize_vq_chunks_parallel(rhs_files)
                .await
                .with_context(|| format!("materialize VQ sprite chunks for mission {mission}"))?;
        }
        tracing::info!(
            mission,
            elapsed_ms = vq_drain_start.elapsed().as_secs_f64() * 1000.0,
            includes_rle = bounded,
            "startup timing: VQ tail wait and apply"
        );
        let rle_drain_start = web_time::Instant::now();
        // Apply the worker-pool RLE-JXL decodes that ran alongside the
        // fetches; anything still pending falls to the strict serial pass.
        while let Some((chunk, packed)) = rle_scheduler.next_decoded().await? {
            bank.apply_decoded_rle_jxl_chunk(&chunk, packed)?;
            rle_scheduler.dispatch_ready(bank, &mut pending_rle)?;
        }
        bank.rle_jxl_chunks.append(&mut pending_rle);
        bank.materialize_rle_jxl_chunks()
            .with_context(|| format!("materialize RLE-JXL sprite chunks for mission {mission}"))?;
        tracing::info!(
            mission,
            elapsed_ms = rle_drain_start.elapsed().as_secs_f64() * 1000.0,
            "startup timing: RLE tail wait and apply"
        );
    }
    progress(MissionLoadProgress {
        phase: MissionLoadPhase::Data,
        completed: InstallWorkModel::UNITS,
        total: InstallWorkModel::UNITS,
        file: None,
    });
    // Package the deferred tail (post-activation background streaming).
    let tail = match (deferral, merged.payload.sprite_bank.as_ref()) {
        (Some(deferral), Some(bank)) if !deferral.parked.is_empty() => {
            let chunks = deferral.parked;
            let rhs_files = chunks
                .iter()
                .filter(|chunk| chunk.self_refs)
                .filter_map(|chunk| merged.payload.rhs_files.get_key_value(&chunk.rhs))
                .map(|(path, rhs)| (path.clone(), rhs.clone()))
                .collect();
            Some(DeferredSpriteTail {
                bank: ShippingSpriteBank {
                    signature: bank.signature,
                    dictionaries: Vec::new(),
                    sprite_count: bank.sprite_count,
                    sprites: bank.sprites.clone(),
                    vq_chunks: Vec::new(),
                    // The tail only carries deferred VQ work; RLE-JXL
                    // chunks are always materialized before activation.
                    rle_jxl_chunks: Vec::new(),
                },
                rhs_files,
                chunks,
            })
        }
        _ => None,
    };
    Ok((merged, fetched_bytes, tail, early_terrain))
}

/// Stream the deferred (reinforcement-only) sprite chunks on the worker
/// pool while the mission is already running. Decoded grids are published
/// through [`robin_assets::late_sprites`], where every live `FrameHolder`
/// row already points; a draw call that races a still-pending grid skips
/// safely and the sprite pops in a frame later.
///
/// Post-activation failures must never take the mission down: decode
/// errors, stuck dependencies, and a superseding mission install all
/// degrade to a warn/debug log plus permanently-skipped sprites.
pub(super) fn spawn_deferred_sprite_tail(
    mission: String,
    streaming: &robin_assets::late_sprites::SpriteStreaming,
    tail: DeferredSpriteTail,
) {
    use robin_assets::shipping_datadir::VqDecodeScheduler;

    let DeferredSpriteTail {
        bank,
        rhs_files,
        chunks,
    } = tail;
    let total_chunks = chunks.len();
    let total_blob: u64 = chunks.iter().map(|chunk| chunk.blob.len() as u64).sum();
    let publisher = streaming.publisher(total_chunks, total_blob);
    tracing::info!(
        mission,
        chunks = total_chunks,
        blob_bytes = total_blob,
        "mission activated with deferred sprite chunks; streaming in background"
    );
    wasm_bindgen_futures::spawn_local(async move {
        let started = js_sys::Date::now();
        let mut bank = bank;
        let mut pending = chunks;
        let mut scheduler = VqDecodeScheduler::default();
        let mut decoded_chunks = 0usize;
        let mut decoded_sprites = 0usize;
        loop {
            if publisher.is_retired() {
                tracing::debug!(mission, "mission sprite streaming retired");
                return;
            }
            // Strict readiness: the full mission payload is merged, so a
            // missing base row is a manifest error, not "not yet".
            if let Err(error) = scheduler.dispatch_ready(&bank, &mut pending, &rhs_files, true) {
                tracing::warn!(
                    mission,
                    "background sprite streaming aborted (affected sprites stay skipped): \
                     {error:#}"
                );
                publisher.fail_tail();
                return;
            }
            match scheduler.next_decoded().await {
                Err(error) => {
                    // One chunk lost; its sprites keep safe-skipping. Other
                    // in-flight decodes are still worth draining.
                    tracing::warn!(mission, "background sprite chunk decode failed: {error:#}");
                    publisher.fail_tail();
                }
                Ok(None) => break,
                Ok(Some((chunk, grids))) => {
                    let grids: Vec<(u32, Arc<Vec<u16>>)> = grids
                        .into_iter()
                        .map(|(sprite_id, grid)| (sprite_id, Arc::new(grid)))
                        .collect();
                    if !publisher.publish_chunk(chunk.blob.len() as u64, &grids) {
                        tracing::debug!(
                            mission,
                            "another mission install superseded the sprite streaming tail"
                        );
                        return;
                    }
                    decoded_chunks += 1;
                    decoded_sprites += grids.len();
                    // Mirror the grids into the private row clone so
                    // dependent deferred chunks see their base grids.
                    for (sprite_id, grid) in grids {
                        if let Ok(position) =
                            bank.sprites.binary_search_by_key(&sprite_id, |(id, _)| *id)
                        {
                            let sprite = &mut bank.sprites[position].1;
                            if sprite.packed_data.is_empty() {
                                sprite.packed_data = grid;
                            }
                        }
                    }
                    tracing::debug!(
                        mission,
                        chunk = %chunk.rhs,
                        "background sprite chunk streamed in"
                    );
                }
            }
            // Keep the main thread breathing between applies.
            crate::window::yield_to_runtime().await;
        }
        if !pending.is_empty() {
            let stuck: Vec<&str> = pending.iter().map(|chunk| chunk.rhs.as_str()).collect();
            tracing::warn!(
                mission,
                "background sprite streaming stuck — base sprites never materialized \
                 (affected sprites stay skipped): {}",
                stuck.join(", ")
            );
            publisher.fail_tail();
            return;
        }
        if decoded_chunks != total_chunks {
            publisher.fail_tail();
            tracing::warn!(
                mission,
                decoded_chunks,
                total_chunks,
                "sprite streaming finished with failed chunks; affected sprites stay skipped"
            );
            return;
        }
        tracing::info!(
            mission,
            chunks = decoded_chunks,
            sprites = decoded_sprites,
            skipped_draws = ?publisher.skipped_draws(),
            elapsed_ms = js_sys::Date::now() - started,
            "background sprite streaming complete"
        );
    });
}

/// Fetch with live byte accounting for the install-progress model: the part's
/// size registers from `Content-Length` when response headers arrive, and body
/// bytes count into `progress.received` as each network chunk lands. Reading
/// through a `ReadableStream` rather than one opaque `arrayBuffer()` await
/// makes the loading bar reflect received bytes, not completed files.
async fn fetch_counted(
    datadir: &ShippingDatadir,
    relative: &str,
    progress: &FetchByteProgress,
) -> Result<CompressedPayload> {
    use std::sync::atomic::Ordering;
    use wasm_bindgen::JsCast as _;
    use wasm_bindgen_futures::JsFuture;

    let key = canonical_relative_file_key(relative)?;
    if let Some(bytes) = datadir.preloaded_file(&key) {
        let len = bytes.len() as u64;
        progress.add_known(len);
        progress.received.fetch_add(len, Ordering::Relaxed);
        return Ok(CompressedPayload::Shared(bytes));
    }
    if let Some(pending) = take_early_download(datadir, &key) {
        let bytes = pending.await.map_err(anyhow::Error::msg)?;
        let len = bytes.len() as u64;
        progress.add_known(len);
        progress.received.fetch_add(len, Ordering::Relaxed);
        return Ok(CompressedPayload::Shared(bytes));
    }

    let base = datadir
        .remote_base_url()
        .ok_or_else(|| anyhow!("browser shipping manifest has no remote base URL"))?;
    let url = format!("{base}/{}", relative.trim_start_matches('/'));
    let window = web_sys::window().ok_or_else(|| anyhow!("browser window is unavailable"))?;
    let response = JsFuture::from(window.fetch_with_str(&url))
        .await
        .map_err(|error| anyhow!("fetch {url}: {error:?}"))?
        .dyn_into::<web_sys::Response>()
        .map_err(|_| anyhow!("fetch {url}: result is not a Response"))?;
    if !response.ok() {
        return Err(anyhow!("fetch {url}: HTTP {}", response.status()));
    }
    let claimed: Option<u64> = response
        .headers()
        .get("content-length")
        .ok()
        .flatten()
        .and_then(|value| value.parse().ok());
    if let Some(claimed) = claimed {
        progress.add_known(claimed);
    }
    let mut bytes: Vec<u8> = Vec::with_capacity(claimed.unwrap_or(0) as usize);
    match response.body() {
        Some(stream) => {
            let reader: web_sys::ReadableStreamDefaultReader = stream
                .get_reader()
                .dyn_into()
                .map_err(|_| anyhow!("fetch {url}: body reader is not a default reader"))?;
            loop {
                let result = JsFuture::from(reader.read())
                    .await
                    .map_err(|error| anyhow!("fetch {url}: read body: {error:?}"))?;
                let done = js_sys::Reflect::get(&result, &"done".into())
                    .map_err(|error| anyhow!("fetch {url}: read result: {error:?}"))?
                    .as_bool()
                    .unwrap_or(false);
                if done {
                    break;
                }
                let value = js_sys::Reflect::get(&result, &"value".into())
                    .map_err(|error| anyhow!("fetch {url}: read result: {error:?}"))?;
                let chunk = js_sys::Uint8Array::new(&value);
                let offset = bytes.len();
                bytes.resize(offset + chunk.length() as usize, 0);
                chunk.copy_to(&mut bytes[offset..]);
                progress
                    .received
                    .fetch_add(u64::from(chunk.length()), Ordering::Relaxed);
            }
        }
        None => {
            // No streamable body (unusual); fall back to one opaque read.
            let buffer = response
                .array_buffer()
                .map_err(|error| anyhow!("fetch {url}: arrayBuffer: {error:?}"))?;
            let buffer = JsFuture::from(buffer)
                .await
                .map_err(|error| anyhow!("fetch {url}: read body: {error:?}"))?;
            bytes = js_sys::Uint8Array::new(&buffer).to_vec();
            progress
                .received
                .fetch_add(bytes.len() as u64, Ordering::Relaxed);
        }
    }
    // Reconcile the totals with the actual decoded body length: a missing
    // header registers late; a compressed transfer's header claims the
    // encoded size while the reader yields decoded bytes.
    match claimed {
        None => progress.add_known(bytes.len() as u64),
        Some(claimed) => {
            let actual = bytes.len() as u64;
            if actual >= claimed {
                progress
                    .known_total
                    .fetch_add(actual - claimed, Ordering::Relaxed);
            } else {
                progress
                    .known_total
                    .fetch_sub(claimed - actual, Ordering::Relaxed);
            }
        }
    }
    Ok(CompressedPayload::Owned(bytes))
}
