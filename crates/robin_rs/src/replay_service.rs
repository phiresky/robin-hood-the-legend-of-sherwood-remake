//! Process-lived replay ownership, independent of HTTP and leaderboard transports.
//!
//! The composition root shares one service. Recorder writers carry generation
//! authority; snapshots own immutable chunks; pending launches reject duplicates.
use robin_engine::replay as engine_replay;
use serde::{Deserialize, Serialize};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::OnceLock;
use std::sync::{Arc, Mutex};

pub struct PendingReplay {
    pub data: engine_replay::ReplayData,
    pub paused: bool,
}

/// Owns replay lifecycle resources, not the transport that happens to request them.
pub struct ReplayService {
    pending: Mutex<Option<PendingReplay>>,
    spool: ReplaySpool,
    #[cfg(not(target_arch = "wasm32"))]
    export_worker: OnceLock<Result<std::sync::mpsc::SyncSender<NativeReplayExportJob>, String>>,
    #[cfg(target_arch = "wasm32")]
    export_busy: Arc<std::sync::atomic::AtomicBool>,
}

impl Default for ReplayService {
    fn default() -> Self {
        Self {
            pending: Mutex::new(None),
            spool: ReplaySpool::new(MAX_ACTIVE_REPLAY_BYTES),
            #[cfg(not(target_arch = "wasm32"))]
            export_worker: OnceLock::new(),
            #[cfg(target_arch = "wasm32")]
            export_busy: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
}

impl Serialize for ReplayService {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str("live replay service")
    }
}
impl<'de> Deserialize<'de> for ReplayService {
    fn deserialize<D: serde::Deserializer<'de>>(_: D) -> Result<Self, D::Error> {
        Err(serde::de::Error::custom(
            "replay service must be constructed by its process owner",
        ))
    }
}

impl std::fmt::Debug for ReplayService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ReplayService { live authority }")
    }
}

impl ReplayService {
    pub fn recording(self: &Arc<Self>) -> ReplayRecordingControl {
        ReplayRecordingControl(self.clone())
    }
    pub fn exports(self: &Arc<Self>) -> ReplayExports {
        ReplayExports(self.clone())
    }
    pub fn launches(self: &Arc<Self>) -> ReplayLaunches {
        ReplayLaunches(self.clone())
    }
    /// Single-slot admission is first-accepted-wins until the launch is consumed.
    /// A rejected request never displaces an already acknowledged launch.
    pub fn admit_pending(&self, replay: PendingReplay) -> Result<(), String> {
        let mut pending = self.pending.lock().expect("pending replay poisoned");
        if pending.is_some() {
            return Err(
                "a replay launch is already pending; consume it before queuing another".into(),
            );
        }
        *pending = Some(replay);
        Ok(())
    }
    pub fn take_pending(&self) -> Option<PendingReplay> {
        self.pending.lock().expect("pending replay poisoned").take()
    }
    pub fn pending_mission(&self) -> Option<String> {
        self.pending
            .lock()
            .expect("pending replay poisoned")
            .as_ref()
            .map(|p| p.data.header().mission_id.clone())
    }
    /// Starts a new generation; all previous writer handles become stale.
    pub fn begin_recording(&self) -> ReplaySpoolWriter {
        self.spool.begin()
    }
    /// Invalidates active recording without touching already frozen snapshots.
    pub(crate) fn invalidate(&self, reason: impl Into<String>) {
        self.begin_recording().poison(reason);
    }
    pub fn snapshot_bytes(&self) -> Result<Vec<u8>, String> {
        self.snapshot().map(|snapshot| snapshot.to_vec())
    }
    pub(crate) fn snapshot(&self) -> Result<ReplaySnapshot, String> {
        self.spool.snapshot()
    }

    pub(crate) fn export(&self, complete: ExportCompletion) {
        let snapshot = match self.snapshot() {
            Ok(snapshot) => snapshot,
            Err(error) => {
                complete(Err(error));
                return;
            }
        };
        #[cfg(not(target_arch = "wasm32"))]
        match self.native_export_worker() {
            Ok(worker) => try_enqueue_native_replay_export(worker, snapshot, complete),
            Err(error) => complete(Err(error)),
        }
        #[cfg(target_arch = "wasm32")]
        self.export_browser(snapshot, complete);
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn native_export_worker(
        &self,
    ) -> Result<&std::sync::mpsc::SyncSender<NativeReplayExportJob>, String> {
        self.export_worker
            .get_or_init(|| {
                let (tx, rx) = std::sync::mpsc::sync_channel::<NativeReplayExportJob>(1);
                std::thread::Builder::new()
                    .name("robin-replay-export".into())
                    .spawn(move || {
                        while let Ok(job) = rx.recv() {
                            (job.complete)(job.snapshot.compact_sync());
                        }
                    })
                    .map_err(|error| format!("spawn replay export worker: {error}"))?;
                Ok(tx)
            })
            .as_ref()
            .map_err(Clone::clone)
    }

    #[cfg(target_arch = "wasm32")]
    fn export_browser(&self, snapshot: ReplaySnapshot, complete: ExportCompletion) {
        use std::sync::atomic::Ordering;
        if self.export_busy.swap(true, Ordering::AcqRel) {
            complete(Err(
                "replay export is already running; retry after it finishes".into(),
            ));
            return;
        }
        let busy = Arc::clone(&self.export_busy);
        wasm_bindgen_futures::spawn_local(async move {
            struct ReleaseBusy(Arc<std::sync::atomic::AtomicBool>);
            impl Drop for ReleaseBusy {
                fn drop(&mut self) {
                    self.0.store(false, Ordering::Release);
                }
            }
            let _release = ReleaseBusy(busy);
            // Yield to rendering before encoding; block-wise encoding remains
            // a separate task, not a reason to change the canonical format.
            gloo_timers::future::TimeoutFuture::new(0).await;
            complete(snapshot.compact_sync());
        });
    }
}

macro_rules! replay_capability {
    ($name:ident) => {
        #[derive(Clone, Debug)]
        pub struct $name(Arc<ReplayService>);
        impl Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(concat!("live ", stringify!($name)))
            }
        }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(_: D) -> Result<Self, D::Error> {
                Err(serde::de::Error::custom(
                    "replay capabilities must be injected by their application owner",
                ))
            }
        }
    };
}
replay_capability!(ReplayRecordingControl);
replay_capability!(ReplayExports);
replay_capability!(ReplayLaunches);

/// Recording authority does not include export or replay admission.
/// ```compile_fail
/// fn export(control: robin_rs::replay_service::ReplayRecordingControl) {
///     control.snapshot_bytes();
/// }
/// ```
/// ```compile_fail
/// fn escalate(control: robin_rs::replay_service::ReplayRecordingControl) {
///     let root = control.0;
/// }
/// ```
impl ReplayRecordingControl {
    pub fn begin_recording(&self) -> ReplaySpoolWriter {
        self.0.begin_recording()
    }
    pub(crate) fn invalidate(&self, reason: impl Into<String>) {
        self.0.invalidate(reason);
    }
}
impl ReplayExports {
    pub fn snapshot_bytes(&self) -> Result<Vec<u8>, String> {
        self.0.snapshot_bytes()
    }
    pub(crate) fn snapshot(&self) -> Result<ReplaySnapshot, String> {
        self.0.snapshot()
    }
    pub(crate) fn export(&self, complete: ExportCompletion) {
        self.0.export(complete);
    }
}
impl ReplayLaunches {
    pub fn admit_pending(&self, replay: PendingReplay) -> Result<(), String> {
        self.0.admit_pending(replay)
    }
    pub fn take_pending(&self) -> Option<PendingReplay> {
        self.0.take_pending()
    }
    pub fn pending_mission(&self) -> Option<String> {
        self.0.pending_mission()
    }
}

#[cfg(not(target_arch = "wasm32"))]
type ExportCompletion = Box<dyn FnOnce(Result<String, String>) + Send>;
#[cfg(target_arch = "wasm32")]
type ExportCompletion = Box<dyn FnOnce(Result<String, String>)>;

#[cfg(not(target_arch = "wasm32"))]
struct NativeReplayExportJob {
    snapshot: ReplaySnapshot,
    complete: ExportCompletion,
}

#[cfg(not(target_arch = "wasm32"))]
fn try_enqueue_native_replay_export(
    worker: &std::sync::mpsc::SyncSender<NativeReplayExportJob>,
    snapshot: ReplaySnapshot,
    complete: ExportCompletion,
) {
    match worker.try_send(NativeReplayExportJob { snapshot, complete }) {
        Ok(()) => {}
        Err(std::sync::mpsc::TrySendError::Full(job)) => (job.complete)(Err(
            "replay export worker is busy; retry after the current export finishes".into(),
        )),
        Err(std::sync::mpsc::TrySendError::Disconnected(job)) => {
            (job.complete)(Err("replay export worker stopped unexpectedly".into()))
        }
    }
}

/// Hard local limits for the active JSONL recorder. The public replay service
/// applies its own admission limits to the canonical compact artifact; these
/// limits protect the in-process native/browser recording path before export.
const MAX_ACTIVE_REPLAY_BYTES: usize = 64 * 1024 * 1024;
const MAX_ACTIVE_REPLAY_LINE_BYTES: usize = 16 * 1024 * 1024;
const REPLAY_SPOOL_CHUNK_BYTES: usize = 64 * 1024;

#[derive(Clone)]
struct ReplaySpool {
    inner: Arc<Mutex<ReplaySpoolState>>,
    max_bytes: usize,
    max_pending_bytes: usize,
}

struct ReplaySpoolState {
    generation: u64,
    chunks: Vec<Arc<[u8]>>,
    tail: Vec<u8>,
    committed_bytes: usize,
    failure: Option<String>,
}

impl ReplaySpool {
    fn new(max_bytes: usize) -> Self {
        assert!(max_bytes > 0, "replay spool limit must be positive");
        Self {
            inner: Arc::new(Mutex::new(ReplaySpoolState {
                generation: 0,
                chunks: Vec::new(),
                tail: Vec::with_capacity(REPLAY_SPOOL_CHUNK_BYTES),
                committed_bytes: 0,
                failure: None,
            })),
            max_bytes,
            max_pending_bytes: MAX_ACTIVE_REPLAY_LINE_BYTES.min(max_bytes),
        }
    }

    fn begin(&self) -> ReplaySpoolWriter {
        let mut state = self.inner.lock().expect("replay spool poisoned");
        state.generation = state
            .generation
            .checked_add(1)
            .expect("replay spool generation overflow");
        state.chunks.clear();
        state.tail.clear();
        state.committed_bytes = 0;
        state.failure = None;
        ReplaySpoolWriter {
            spool: self.clone(),
            generation: state.generation,
            pending: Vec::new(),
        }
    }

    fn snapshot(&self) -> Result<ReplaySnapshot, String> {
        let state = self.inner.lock().expect("replay spool poisoned");
        if let Some(error) = &state.failure {
            return Err(format!("active replay spool is unavailable: {error}"));
        }
        let mut chunks = state.chunks.clone();
        if !state.tail.is_empty() {
            // Complete chunks are Arc clones. Snapshotting on the game thread
            // copies at most the one incomplete 64-KiB tail.
            chunks.push(Arc::from(state.tail.clone()));
        }
        Ok(ReplaySnapshot {
            #[cfg(all(test, not(target_arch = "wasm32")))]
            generation: state.generation,
            byte_length: state.committed_bytes,
            chunks,
        })
    }
}

/// Recorder-owned staging writer. Bytes become visible to readers only after
/// the recorder flushes a complete header/record, so export never observes a
/// partial JSONL line.
pub struct ReplaySpoolWriter {
    spool: ReplaySpool,
    generation: u64,
    pending: Vec<u8>,
}

impl ReplaySpoolWriter {
    fn io_error(message: impl Into<String>) -> std::io::Error {
        std::io::Error::other(message.into())
    }

    fn preflight(&self, additional: usize) -> std::io::Result<()> {
        let mut state = self.spool.inner.lock().expect("replay spool poisoned");
        if state.generation != self.generation {
            return Err(Self::io_error(
                "replay spool writer belongs to an earlier mission",
            ));
        }
        if let Some(error) = &state.failure {
            return Err(Self::io_error(error.clone()));
        }
        let observed = state
            .committed_bytes
            .checked_add(self.pending.len())
            .and_then(|value| value.checked_add(additional))
            .ok_or_else(|| Self::io_error("replay spool byte count overflow"))?;
        let pending = self
            .pending
            .len()
            .checked_add(additional)
            .ok_or_else(|| Self::io_error("replay spool pending-line byte count overflow"))?;
        if observed > self.spool.max_bytes {
            let error = format!(
                "replay recording reached {observed} bytes, bounded spool limit is {} bytes",
                self.spool.max_bytes
            );
            state.failure = Some(error.clone());
            return Err(Self::io_error(error));
        }
        if pending > self.spool.max_pending_bytes {
            let error = format!(
                "replay JSONL record reached {pending} bytes, local line limit is {} bytes",
                self.spool.max_pending_bytes
            );
            state.failure = Some(error.clone());
            return Err(Self::io_error(error));
        }
        Ok(())
    }

    /// Reject known backpressure before a tee writer changes its durable
    /// primary. Primary short writes are mirrored by their exact returned
    /// length and retried normally by `Write::write_all`.
    pub fn preflight_write(&self, bytes: usize) -> std::io::Result<()> {
        self.preflight(bytes)
    }

    /// Permanently invalidate this mission's spool after the durable primary
    /// reports an ambiguous write or flush failure.
    pub fn poison(&self, reason: impl Into<String>) {
        let mut state = self.spool.inner.lock().expect("replay spool poisoned");
        if state.generation == self.generation && state.failure.is_none() {
            state.failure = Some(reason.into());
        }
    }
}

impl std::io::Write for ReplaySpoolWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.preflight(buf.len())?;
        self.pending.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        let mut state = self.spool.inner.lock().expect("replay spool poisoned");
        if state.generation != self.generation {
            return Err(Self::io_error(
                "replay spool writer belongs to an earlier mission",
            ));
        }
        if let Some(error) = &state.failure {
            return Err(Self::io_error(error.clone()));
        }
        let pending_len = self.pending.len();
        let mut source = self.pending.as_slice();
        while !source.is_empty() {
            let available = REPLAY_SPOOL_CHUNK_BYTES - state.tail.len();
            let take = available.min(source.len());
            state.tail.extend_from_slice(&source[..take]);
            source = &source[take..];
            if state.tail.len() == REPLAY_SPOOL_CHUNK_BYTES {
                let full = std::mem::replace(
                    &mut state.tail,
                    Vec::with_capacity(REPLAY_SPOOL_CHUNK_BYTES),
                );
                state.chunks.push(Arc::from(full));
            }
        }
        state.committed_bytes = state
            .committed_bytes
            .checked_add(pending_len)
            .expect("replay spool committed byte count overflow after preflight");
        self.pending.clear();
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ReplaySnapshot {
    #[cfg(all(test, not(target_arch = "wasm32")))]
    generation: u64,
    byte_length: usize,
    chunks: Vec<Arc<[u8]>>,
}

impl ReplaySnapshot {
    fn to_vec(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.byte_length);
        for chunk in &self.chunks {
            bytes.extend_from_slice(chunk);
        }
        assert_eq!(
            bytes.len(),
            self.byte_length,
            "replay spool snapshot length disagrees with its chunks"
        );
        bytes
    }

    pub(crate) fn compact_sync(&self) -> Result<String, String> {
        let data = self.parse_sync()?;
        robin_replay_format::encode_compact(&data, robin_replay_format::ENGINE_VERSION_HASH)
            .map_err(|error| format!("encode compact replay: {error}"))
    }

    pub(crate) fn parse_sync(&self) -> Result<engine_replay::ReplayData, String> {
        if self.byte_length == 0 {
            return Err("no active replay recording".to_owned());
        }
        engine_replay::ReplayData::from_reader(std::io::Cursor::new(self.to_vec()))
            .map_err(|error| format!("parse mirrored replay buffer: {error}"))
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn injected_capabilities_share_only_their_application_lifecycle() {
        let first = Arc::new(ReplayService::default());
        let second = Arc::new(ReplayService::default());
        let recording = first.recording();
        let exports = first.exports();
        let launches = first.launches();
        let mut writer = recording.begin_recording();
        writer.write_all(b"isolated\n").unwrap();
        writer.flush().unwrap();
        launches.admit_pending(pending("first", true)).unwrap();
        assert_eq!(exports.snapshot_bytes().unwrap(), b"isolated\n");
        assert!(second.exports().snapshot_bytes().unwrap().is_empty());
        assert!(second.launches().take_pending().is_none());
        drop(first);
        assert_eq!(
            launches.take_pending().unwrap().data.header().mission_id,
            "first"
        );
        recording.invalidate("retired");
        assert!(exports.snapshot_bytes().unwrap_err().contains("retired"));
        assert!(writer.flush().is_err());
    }

    #[test]
    fn capability_diagnostics_cannot_reconstitute_authority() {
        let service = Arc::new(ReplayService::default());
        assert!(
            serde_json::from_value::<ReplayRecordingControl>(
                serde_json::to_value(service.recording()).unwrap()
            )
            .is_err()
        );
        assert!(
            serde_json::from_value::<ReplayExports>(
                serde_json::to_value(service.exports()).unwrap()
            )
            .is_err()
        );
        assert!(
            serde_json::from_value::<ReplayLaunches>(
                serde_json::to_value(service.launches()).unwrap()
            )
            .is_err()
        );
    }

    fn pending(mission: &str, paused: bool) -> PendingReplay {
        let spool = ReplaySpool::new(2 * 1024 * 1024);
        let _recorder = engine_replay::ReplayRecorder::with_writer(
            Box::new(spool.begin()),
            mission.to_owned(),
            robin_engine::mission_assets::MissionAssetDescriptor::built_in(mission, "map", "map")
                .unwrap(),
            17,
            robin_engine::engine::SimConfig::default(),
            &robin_engine::campaign::Campaign::default(),
        )
        .unwrap();
        PendingReplay {
            data: spool.snapshot().unwrap().parse_sync().unwrap(),
            paused,
        }
    }

    #[test]
    fn duplicate_pending_launch_preserves_the_acknowledged_request() {
        let service = ReplayService::default();
        service.admit_pending(pending("first", true)).unwrap();
        assert!(
            service
                .admit_pending(pending("second", false))
                .unwrap_err()
                .contains("already pending")
        );
        assert_eq!(service.pending_mission().as_deref(), Some("first"));
        let launch = service.take_pending().unwrap();
        assert_eq!(launch.data.header().mission_id, "first");
        assert!(launch.paused);
        assert!(service.take_pending().is_none());
        service.admit_pending(pending("second", false)).unwrap();
        assert_eq!(service.pending_mission().as_deref(), Some("second"));
    }

    #[test]
    fn independent_services_and_frozen_snapshots_do_not_share_lifecycle() {
        let first = ReplayService::default();
        let second = ReplayService::default();
        let mut stale_writer = first.begin_recording();
        stale_writer.write_all(b"first\n").unwrap();
        stale_writer.flush().unwrap();
        let frozen = first.snapshot().unwrap();
        first.invalidate("foreign save");
        assert!(first.snapshot().unwrap_err().contains("foreign save"));
        assert_eq!(frozen.to_vec(), b"first\n");
        assert!(stale_writer.write_all(b"stale\n").is_err());
        assert!(second.snapshot_bytes().unwrap().is_empty());
        let mut current = first.begin_recording();
        current.write_all(b"replacement\n").unwrap();
        current.flush().unwrap();
        assert_eq!(frozen.to_vec(), b"first\n");
        assert_eq!(first.snapshot_bytes().unwrap(), b"replacement\n");
    }

    #[test]
    fn diagnostics_cannot_recreate_live_service_authority() {
        let diagnostic = serde_json::to_value(ReplayService::default()).unwrap();
        assert!(serde_json::from_value::<ReplayService>(diagnostic).is_err());
    }
    #[test]
    fn replay_spool_publishes_only_complete_flush_boundaries() {
        let spool = ReplaySpool::new(1024);
        let mut writer = spool.begin();
        writer.write_all(b"header\n").unwrap();
        let before_flush = spool.snapshot().unwrap();
        assert_eq!(before_flush.generation, 1);
        assert_eq!(before_flush.byte_length, 0);
        assert!(before_flush.to_vec().is_empty());

        writer.flush().unwrap();
        writer.write_all(b"partial record").unwrap();
        assert_eq!(spool.snapshot().unwrap().to_vec(), b"header\n");

        writer.write_all(b" end\n").unwrap();
        writer.flush().unwrap();
        assert_eq!(
            spool.snapshot().unwrap().to_vec(),
            b"header\npartial record end\n"
        );
    }

    #[test]
    fn replay_spool_overflow_poison_is_atomic_and_generational() {
        let spool = ReplaySpool::new(8);
        let mut old = spool.begin();
        old.write_all(b"12345678").unwrap();
        old.flush().unwrap();
        assert_eq!(spool.snapshot().unwrap().to_vec(), b"12345678");

        let error = old.write_all(b"9").unwrap_err().to_string();
        assert!(error.contains("bounded spool limit"), "{error}");
        let state = spool.inner.lock().unwrap();
        assert_eq!(state.committed_bytes, 8);
        assert_eq!(state.tail.as_slice(), b"12345678");
        drop(state);
        let snapshot_error = spool.snapshot().unwrap_err();
        assert!(
            snapshot_error.contains("reached 9 bytes"),
            "{snapshot_error}"
        );

        let mut current = spool.begin();
        assert!(
            old.flush()
                .unwrap_err()
                .to_string()
                .contains("earlier mission")
        );
        current.write_all(b"new\n").unwrap();
        current.flush().unwrap();
        assert_eq!(spool.snapshot().unwrap().to_vec(), b"new\n");
    }

    #[test]
    fn replay_spool_enforces_the_independent_record_ceiling() {
        assert_eq!(MAX_ACTIVE_REPLAY_BYTES, 64 * 1024 * 1024);
        assert_eq!(MAX_ACTIVE_REPLAY_LINE_BYTES, 16 * 1024 * 1024);
        assert_eq!(REPLAY_SPOOL_CHUNK_BYTES, 64 * 1024);

        let mut spool = ReplaySpool::new(64);
        spool.max_pending_bytes = 8;
        let mut writer = spool.begin();
        writer.write_all(b"12345678").unwrap();
        let error = writer.write_all(b"9").unwrap_err().to_string();
        assert!(error.contains("local line limit is 8 bytes"), "{error}");
        assert!(spool.snapshot().unwrap_err().contains("reached 9 bytes"));
    }

    #[test]
    fn active_replay_spool_exports_the_single_canonical_compact_format() {
        let spool = ReplaySpool::new(2 * 1024 * 1024);
        let writer = spool.begin();
        let mut recorder = engine_replay::ReplayRecorder::with_writer(
            Box::new(writer),
            "active-snapshot".to_owned(),
            robin_engine::mission_assets::MissionAssetDescriptor::built_in(
                "active-snapshot",
                "active-map",
                "active-map",
            )
            .expect("valid built-in active replay test descriptor"),
            17,
            robin_engine::engine::SimConfig::default(),
            &robin_engine::campaign::Campaign::default(),
        )
        .unwrap();
        assert!(recorder.write_frame(
            0,
            0,
            1,
            robin_engine::engine::SimulationFrameInput {
                run_hourglass: true,
                ..robin_engine::engine::SimulationFrameInput::default()
            },
            Vec::new(),
            None,
        ));

        let jsonl = spool.snapshot().unwrap().to_vec();
        let replay = engine_replay::ReplayData::from_reader(std::io::Cursor::new(jsonl)).unwrap();
        let compact =
            robin_replay_format::encode_compact(&replay, robin_replay_format::ENGINE_VERSION_HASH)
                .unwrap();
        let (_, decoded) = robin_replay_format::decode_compact(&compact).unwrap();
        assert_eq!(decoded.frame_count(), 1);
        assert!(decoded.frame(0).unwrap().input.run_hourglass);
    }

    #[test]
    fn replay_spool_long_run_uses_fixed_chunks_and_bounded_staging() {
        let limit = 8 * 1024 * 1024;
        let spool = ReplaySpool::new(limit);
        let mut writer = spool.begin();
        let record = [b'x'; 511];
        let mut expected_len = 0;
        for _ in 0..10_000 {
            writer.write_all(&record).unwrap();
            writer.write_all(b"\n").unwrap();
            writer.flush().unwrap();
            expected_len += record.len() + 1;
            assert!(writer.pending.capacity() <= MAX_ACTIVE_REPLAY_LINE_BYTES);
        }
        let snapshot = spool.snapshot().unwrap();
        assert_eq!(snapshot.byte_length, expected_len);
        let state = spool.inner.lock().unwrap();
        assert!(
            state
                .chunks
                .iter()
                .all(|chunk| chunk.len() == REPLAY_SPOOL_CHUNK_BYTES)
        );
        assert!(state.tail.len() < REPLAY_SPOOL_CHUNK_BYTES);
        assert!(state.chunks.len() <= limit.div_ceil(REPLAY_SPOOL_CHUNK_BYTES));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn saturated_native_replay_export_queue_returns_explicit_backpressure() {
        let (worker, held) = std::sync::mpsc::sync_channel(1);
        let snapshot = ReplaySnapshot {
            generation: 7,
            byte_length: 2,
            chunks: vec![Arc::from(&b"x\n"[..])],
        };
        let (held_response, _held_rx) = std::sync::mpsc::sync_channel(1);
        worker
            .send(NativeReplayExportJob {
                snapshot: snapshot.clone(),
                complete: Box::new(move |result| {
                    let _ = held_response.send(result);
                }),
            })
            .unwrap();
        let (response, response_rx) = std::sync::mpsc::sync_channel(1);
        try_enqueue_native_replay_export(
            &worker,
            snapshot,
            Box::new(move |result| {
                let _ = response.send(result);
            }),
        );
        let error = match response_rx.recv().unwrap() {
            Ok(_) => panic!("saturated export queue unexpectedly accepted work"),
            Err(error) => error,
        };
        assert!(error.contains("worker is busy"), "{error}");
        drop(held);
    }
}
