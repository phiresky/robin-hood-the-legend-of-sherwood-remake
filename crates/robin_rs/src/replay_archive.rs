//! Mission recording storage. Chunks keep mission-wide ordinals and two
//! independent links: chronological predecessor and the checkpoint restored.
//! Export concatenates every chunk, including gameplay abandoned by a reload.

use anyhow::{Context, Result, ensure};
use robin_engine::replay::{ReplayData, ReplayHeader, ReplaySaveMarker};
use serde::{Deserialize, Serialize};
#[cfg(not(target_arch = "wasm32"))]
use std::io::Read;
use std::io::Write;
use std::path::{Path, PathBuf};

const MAX_BYTES: usize = 64 * 1024 * 1024;
const MANIFEST: &str = "mission.json";

/// Stored in the save, independent of the save's filename or slot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveReplayLink {
    pub mission_directory: String,
    pub chunk: String,
    pub marker: u32,
    pub state_hash: u64,
    pub timeline_frame: u32,
    pub payload_digest: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Chunk {
    file: String,
    previous: Option<String>,
    first_ordinal: u32,
    loaded_save: Option<SaveReplayLink>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChunkHeader {
    recording: ReplayHeader,
    chunk: Chunk,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u32,
    chunks: Vec<Chunk>,
}

/// Exclusive ownership prevents two processes from silently forking a mission's
/// chronological history. Readers may freeze an already committed prefix.
pub(crate) struct MissionArchive {
    directory: PathBuf,
    manifest: Manifest,
    #[cfg(not(target_arch = "wasm32"))]
    _lock: std::fs::File,
    #[cfg(target_arch = "wasm32")]
    _cache_lease: browser::DirectoryLease,
}

impl MissionArchive {
    pub(crate) fn write_ranked_input(
        &self,
        input: &crate::leaderboard_mission_end::MissionEndSubmissionInput,
    ) -> Result<()> {
        input
            .validate()
            .map_err(|error| anyhow::anyhow!("invalid archived ranked evidence: {error}"))?;
        let bytes = serde_json::to_vec(input)?;
        #[cfg(not(target_arch = "wasm32"))]
        crate::save_file::atomic_write(&self.directory.join("ranked.json"), &bytes)?;
        #[cfg(target_arch = "wasm32")]
        browser_write(&self.directory.join("ranked.json"), &bytes)?;
        Ok(())
    }

    pub(crate) fn read_ranked_input(
        &self,
    ) -> Result<crate::leaderboard_mission_end::MissionEndSubmissionInput> {
        let input: crate::leaderboard_mission_end::MissionEndSubmissionInput =
            serde_json::from_slice(&read_bounded(
                &self.directory.join("ranked.json"),
                MAX_BYTES,
            )?)?;
        input
            .validate()
            .map_err(|error| anyhow::anyhow!("invalid archived ranked evidence: {error}"))?;
        Ok(input)
    }

    pub(crate) fn create(directory: &Path) -> Result<Self> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            if let Some(parent) = directory.parent() {
                std::fs::create_dir_all(parent)?;
            }
            // Never truncate an earlier mission, including --record overrides.
            std::fs::create_dir(directory)
                .with_context(|| format!("create mission recording {}", directory.display()))?;
        }
        let directory = canonical_directory(directory)?;
        let mut archive = Self {
            #[cfg(not(target_arch = "wasm32"))]
            _lock: lock_directory(&directory)?,
            #[cfg(target_arch = "wasm32")]
            _cache_lease: browser::pin_directory(&directory)?,
            directory,
            manifest: Manifest {
                version: 1,
                chunks: Vec::new(),
            },
        };
        archive.append_chunk(0, None)?;
        Ok(archive)
    }

    pub(crate) fn open(directory: &Path) -> Result<Self> {
        let directory = canonical_directory(directory)?;
        #[cfg(not(target_arch = "wasm32"))]
        let lock = lock_directory(&directory)?;
        let manifest = read_manifest(&directory)?;
        Ok(Self {
            #[cfg(target_arch = "wasm32")]
            _cache_lease: browser::pin_directory(&directory)?,
            directory,
            manifest,
            #[cfg(not(target_arch = "wasm32"))]
            _lock: lock,
        })
    }

    pub(crate) fn directory(&self) -> &Path {
        &self.directory
    }

    pub(crate) fn current_chunk(&self) -> &str {
        &self
            .manifest
            .chunks
            .last()
            .expect("mission archive has a root chunk")
            .file
    }

    pub(crate) fn marker_link(
        &self,
        ordinal: u32,
        marker: ReplaySaveMarker,
        digest: [u8; 32],
    ) -> SaveReplayLink {
        SaveReplayLink {
            mission_directory: self.directory.to_string_lossy().into_owned(),
            chunk: self.current_chunk().to_owned(),
            marker: ordinal,
            state_hash: marker.state_hash,
            timeline_frame: marker.timeline_frame,
            payload_digest: digest,
        }
    }

    pub(crate) fn append_chunk(
        &mut self,
        first_ordinal: u32,
        loaded_save: Option<SaveReplayLink>,
    ) -> Result<()> {
        let index = self.manifest.chunks.len();
        let chunk = Chunk {
            file: chunk_name(index),
            previous: self.manifest.chunks.last().map(|chunk| chunk.file.clone()),
            first_ordinal,
            loaded_save,
        };
        // Reserve the file before publishing its name. A crash may leave an
        // orphan, but cannot overwrite recorded gameplay on the next load.
        create_chunk(&self.directory.join(&chunk.file))?;
        self.manifest.chunks.push(chunk);
        write_manifest(&self.directory, &self.manifest)?;
        Ok(())
    }

    pub(crate) fn writer(&self) -> Result<Box<dyn Write + Send>> {
        Ok(Box::new(ChunkHeaderWriter {
            writer: open_chunk_writer(&self.directory.join(self.current_chunk()))?,
            chunk: self.manifest.chunks.last().expect("root chunk").clone(),
            header: Some(Vec::new()),
        }))
    }

    pub(crate) fn sync_current(&self) -> Result<()> {
        #[cfg(target_arch = "wasm32")]
        browser::checkpoint()?;
        #[cfg(not(target_arch = "wasm32"))]
        std::fs::OpenOptions::new()
            .write(true)
            .open(self.directory.join(self.current_chunk()))?
            .sync_data()?;
        Ok(())
    }

    /// Return the validated export prefix, parsed history, and exact construction
    /// header together. The parser's derived header is not a continuation header.
    pub(crate) fn assembled_replay(&self) -> Result<(Vec<u8>, ReplayData, ReplayHeader)> {
        assemble_replay(&self.directory, &self.manifest)
    }

    pub(crate) fn validate_link(
        &self,
        link: &SaveReplayLink,
        data: &ReplayData,
        digest: [u8; 32],
    ) -> Result<()> {
        ensure!(
            canonical_directory(Path::new(&link.mission_directory))? == self.directory,
            "save references a different mission recording"
        );
        ensure!(
            link.payload_digest == digest,
            "save payload does not match its replay reference"
        );
        let index = self
            .manifest
            .chunks
            .iter()
            .position(|chunk| chunk.file == link.chunk)
            .context("save references a missing replay chunk")?;
        let start = self.manifest.chunks[index].first_ordinal;
        let end = self
            .manifest
            .chunks
            .get(index + 1)
            .map_or(data.frame_count(), |chunk| chunk.first_ordinal);
        ensure!(
            (start..end).contains(&link.marker),
            "save marker lies outside its referenced chunk"
        );
        let marker = data
            .save_marker_for_frame(link.marker)
            .context("save references a missing replay marker")?;
        ensure!(
            marker.state_hash == link.state_hash && marker.timeline_frame == link.timeline_frame,
            "save marker does not match its replay reference"
        );
        Ok(())
    }
}

fn chunk_name(index: usize) -> String {
    format!("{index:08}.rhrec.jsonl")
}

fn read_manifest(directory: &Path) -> Result<Manifest> {
    let manifest: Manifest =
        serde_json::from_slice(&read_bounded(&directory.join(MANIFEST), MAX_BYTES)?)?;
    ensure!(
        manifest.version == 1,
        "unsupported mission recording version {}",
        manifest.version
    );
    ensure!(
        !manifest.chunks.is_empty(),
        "mission recording has no root chunk"
    );
    for (index, chunk) in manifest.chunks.iter().enumerate() {
        ensure!(
            chunk.file == chunk_name(index),
            "invalid replay chunk filename or chronology"
        );
        ensure!(
            chunk.previous.as_deref()
                == index
                    .checked_sub(1)
                    .map(|previous| &*manifest.chunks[previous].file),
            "broken chronological replay link"
        );
        if index == 0 {
            ensure!(
                chunk.first_ordinal == 0 && chunk.loaded_save.is_none(),
                "invalid mission recording root"
            );
        }
    }
    Ok(manifest)
}

fn write_manifest(directory: &Path, manifest: &Manifest) -> Result<()> {
    let bytes = serde_json::to_vec(manifest)?;
    #[cfg(not(target_arch = "wasm32"))]
    crate::save_file::atomic_write(&directory.join(MANIFEST), &bytes)?;
    #[cfg(target_arch = "wasm32")]
    browser_write(&directory.join(MANIFEST), &bytes)?;
    Ok(())
}

fn assemble_replay(
    directory: &Path,
    manifest: &Manifest,
) -> Result<(Vec<u8>, ReplayData, ReplayHeader)> {
    let mut bytes = Vec::new();
    let mut root_header: Option<ReplayHeader> = None;
    for (index, chunk) in manifest.chunks.iter().enumerate() {
        let raw = read_bounded(&directory.join(&chunk.file), MAX_BYTES - bytes.len())?;
        // A freshly opened child can be empty before the recorder writes its
        // header. It is never silently omitted from a published replay.
        let newline = raw
            .iter()
            .position(|byte| *byte == b'\n')
            .context("replay chunk has no complete header")?;
        let stored: ChunkHeader = serde_json::from_slice(&raw[..newline])?;
        ensure!(
            &stored.chunk == chunk,
            "chunk header disagrees with mission chronology"
        );
        let header = serde_json::to_value(&stored.recording)?;
        if index == 0 {
            serde_json::to_writer(&mut bytes, &stored.recording)?;
            bytes.push(b'\n');
            root_header = Some(stored.recording);
        } else {
            ensure!(
                header
                    == serde_json::to_value(root_header.as_ref().context("missing root header")?)?,
                "replay chunks have different construction headers"
            );
        }
        let records = &raw[newline + 1..];
        ensure!(
            records.is_empty() || records.ends_with(b"\n"),
            "replay chunk ends in an incomplete record"
        );
        for line in records
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
        {
            let value: serde_json::Value = serde_json::from_slice(line)?;
            let ordinal = value
                .get("f")
                .and_then(serde_json::Value::as_u64)
                .context("chunk record has no ordinal")?;
            ensure!(
                ordinal >= u64::from(chunk.first_ordinal),
                "chunk contains records before its chronological boundary"
            );
            if let Some(next) = manifest.chunks.get(index + 1) {
                ensure!(
                    ordinal < u64::from(next.first_ordinal),
                    "chunk overlaps its successor"
                );
            }
        }
        bytes.extend_from_slice(records);
    }
    let data = ReplayData::from_reader(std::io::Cursor::new(&bytes))
        .map_err(|error| anyhow::anyhow!("invalid assembled replay: {error}"))?;
    for chunk in manifest.chunks.iter().skip(1) {
        if let Some(link) = &chunk.loaded_save {
            let load = data
                .load_back_for_frame(chunk.first_ordinal)
                .context("chunk is missing its load event")?;
            ensure!(
                load.snapshot.is_none() && load.to_frame == link.marker,
                "chunk restore disagrees with its saved marker link"
            );
            let marker = data
                .save_marker_for_frame(link.marker)
                .context("chunk restore references a missing marker")?;
            ensure!(
                marker.state_hash == link.state_hash
                    && marker.timeline_frame == link.timeline_frame,
                "chunk restore marker differs from save reference"
            );
        } else {
            ensure!(
                data.load_back_for_frame(chunk.first_ordinal).is_some(),
                "continuation chunk has no restore event"
            );
        }
    }
    Ok((
        bytes,
        data,
        root_header.context("mission recording has no root header")?,
    ))
}

/// Freeze an earlier attempt's chronology at its immutable terminal chunk.
/// Later loads append new files and cannot change this attempt's replay.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn load_through_chunk(path: &Path) -> Result<ReplayData> {
    let directory = path
        .parent()
        .context("replay chunk requires its mission directory")?;
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .context("replay chunk filename is not UTF-8")?;
    let mut manifest = read_manifest(directory)?;
    let index = manifest
        .chunks
        .iter()
        .position(|chunk| chunk.file == filename)
        .context("file is not a chunk in this mission recording")?;
    manifest.chunks.truncate(index + 1);
    let (_, data, _) = assemble_replay(directory, &manifest)?;
    Ok(data)
}

/// Assemble all chronological chunks into the same self-contained replay used
/// by compact exports and verification. No dependency on original save files.
pub fn load_directory(directory: &Path) -> Result<ReplayData> {
    let manifest = read_manifest(directory)?;
    let (_, data, _) = assemble_replay(directory, &manifest)?;
    Ok(data)
}

#[cfg(not(target_arch = "wasm32"))]
fn canonical_directory(path: &Path) -> Result<PathBuf> {
    let path = path.canonicalize()?;
    ensure!(
        path.to_str().is_some(),
        "mission recording directory must be UTF-8"
    );
    Ok(path)
}

#[cfg(not(target_arch = "wasm32"))]
fn lock_directory(directory: &Path) -> Result<std::fs::File> {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(directory.join("recording.lock"))?;
    fs2::FileExt::try_lock_exclusive(&file)
        .context("mission recording is already open in another session")?;
    Ok(file)
}

#[cfg(not(target_arch = "wasm32"))]
fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= limit,
        "mission recording exceeds {MAX_BYTES} bytes"
    );
    Ok(bytes)
}

#[cfg(not(target_arch = "wasm32"))]
fn create_chunk(path: &Path) -> Result<()> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?
        .sync_all()?;
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn open_chunk_writer(path: &Path) -> Result<Box<dyn Write + Send>> {
    Ok(Box::new(DurableChunk(
        std::fs::OpenOptions::new().append(true).open(path)?,
    )))
}

#[cfg(not(target_arch = "wasm32"))]
struct DurableChunk(std::fs::File);

#[cfg(not(target_arch = "wasm32"))]
impl Write for DurableChunk {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.write(bytes)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

/// Local chunks carry their own links as well as the manifest's index. The
/// transport mirror still receives the ordinary, single-artifact header.
struct ChunkHeaderWriter {
    writer: Box<dyn Write + Send>,
    chunk: Chunk,
    header: Option<Vec<u8>>,
}

impl Write for ChunkHeaderWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if let Some(header) = self.header.as_mut() {
            if let Some(newline) = bytes.iter().position(|byte| *byte == b'\n') {
                header.extend_from_slice(&bytes[..newline]);
                let recording = serde_json::from_slice(header).map_err(std::io::Error::other)?;
                serde_json::to_writer(
                    &mut self.writer,
                    &ChunkHeader {
                        recording,
                        chunk: self.chunk.clone(),
                    },
                )
                .map_err(std::io::Error::other)?;
                self.writer.write_all(b"\n")?;
                self.header = None;
                self.writer.write_all(&bytes[newline + 1..])?;
            } else {
                header.extend_from_slice(bytes);
            }
            Ok(bytes.len())
        } else {
            self.writer.write(bytes)
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.writer.flush()
    }
}

#[cfg(target_arch = "wasm32")]
fn canonical_directory(path: &Path) -> Result<PathBuf> {
    Ok(path.to_owned())
}

#[cfg(target_arch = "wasm32")]
mod browser;
#[cfg(target_arch = "wasm32")]
pub(crate) use browser::next_directory as browser_recording_directory;
#[cfg(target_arch = "wasm32")]
use browser::{create_chunk, open_chunk_writer, read_bounded, write as browser_write};
#[cfg(target_arch = "wasm32")]
pub use browser::{
    flush_pending as flush_browser_storage, initialize as initialize_browser_storage,
};
#[cfg(target_arch = "wasm32")]
pub(crate) use browser::{
    prepare_directory as prepare_browser_directory, retire_mission as retire_browser_mission,
};
