//! Re-execute recorded inputs on this engine and atomically publish verified
//! hashes. Input eligibility is preserved; leaderboard verification remains
//! authoritative for the resulting run and its metrics.

use anyhow::{Context, Result, ensure};
use robin_engine::replay::{REPLAY_SCHEMA_VERSION, ReplayData, ReplayFile};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

mod campaign_v48;

const MAX_SOURCE_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpgradeOptions {
    /// Matching `robin` executable, used for both isolated simulation passes.
    pub executable: PathBuf,
    pub timeout_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpgradeReport {
    pub source_schema: u32,
    pub target_schema: u32,
    pub frames: u32,
    pub hash_checkpoints: usize,
    pub save_markers: usize,
    pub output: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
enum CaptureRecord {
    Frame {
        ordinal: u32,
        before: u64,
        after: u64,
    },
    Complete {
        frames: u32,
    },
}

/// Process-owned writer. Its completion record is emitted only after the
/// headless player has consumed the complete recording successfully.
#[derive(Serialize)]
pub(crate) struct HashCapture {
    #[serde(skip)]
    file: std::io::BufWriter<std::fs::File>,
    next: u32,
}

robin_util::deny_deserialize!(HashCapture, "hash capture owns a process-local file");

impl HashCapture {
    pub(crate) fn create(path: &Path) -> Result<Self> {
        Ok(Self {
            file: std::io::BufWriter::new(
                std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(path)?,
            ),
            next: 0,
        })
    }

    pub(crate) fn record(&mut self, ordinal: u32, before: u64, after: u64) -> Result<()> {
        ensure!(
            ordinal == self.next,
            "hash capture skipped or repeated frame {ordinal}"
        );
        serde_json::to_writer(
            &mut self.file,
            &CaptureRecord::Frame {
                ordinal,
                before,
                after,
            },
        )?;
        self.file.write_all(b"\n")?;
        self.next = self
            .next
            .checked_add(1)
            .context("hash capture ordinal overflow")?;
        Ok(())
    }

    pub(crate) fn finish(&mut self) -> Result<()> {
        serde_json::to_writer(
            &mut self.file,
            &CaptureRecord::Complete { frames: self.next },
        )?;
        self.file.write_all(b"\n")?;
        self.file.flush()?;
        self.file.get_ref().sync_all()?;
        Ok(())
    }
}

fn read_capture(path: &Path, frames: u32) -> Result<Vec<(u64, u64)>> {
    let reader = std::io::BufReader::new(std::fs::File::open(path)?);
    let mut hashes = Vec::new();
    let mut complete = false;
    for line in reader.lines() {
        ensure!(!complete, "hash capture has records after completion");
        match serde_json::from_str::<CaptureRecord>(&line?)? {
            CaptureRecord::Frame {
                ordinal,
                before,
                after,
            } => {
                ensure!(
                    ordinal as usize == hashes.len() && ordinal < frames,
                    "hash capture has an invalid ordinal {ordinal}"
                );
                hashes.push((before, after));
            }
            CaptureRecord::Complete { frames: captured } => {
                ensure!(
                    captured == frames && hashes.len() == frames as usize,
                    "hash capture ended before all {frames} frames"
                );
                complete = true;
            }
        }
    }
    ensure!(complete, "hash capture has no successful completion record");
    Ok(hashes)
}

fn bounded_read(path: &Path) -> Result<Vec<u8>> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(MAX_SOURCE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_SOURCE_BYTES,
        "replay exceeds local upgrade size limit"
    );
    Ok(bytes)
}

/// These versions retain the replay input layout. Migrate embedded campaign
/// state separately before decoding with the current engine types.
fn upgrade_header(header: &mut serde_json::Value) -> Result<u32> {
    let version = header
        .get("version")
        .and_then(|v| v.as_u64())
        .and_then(|v| u32::try_from(v).ok())
        .context("replay header has no valid schema")?;
    ensure!(
        version == REPLAY_SCHEMA_VERSION
            || ((43..=53).contains(&version) && (43..=53).contains(&REPLAY_SCHEMA_VERSION)),
        "replay schema {version} needs an input migration before upgrading to {REPLAY_SCHEMA_VERSION}"
    );
    if version < 49 {
        if let Some(campaign) = header.get_mut("campaign") {
            let bytes: Vec<u8> = serde_json::from_value(campaign.clone())
                .context("read embedded replay campaign bytes")?;
            *campaign = serde_json::to_value(campaign_v48::migrate(&bytes)?)?;
        }
    }
    if let Some(config) = header.get_mut("sim_config").and_then(|v| v.as_object_mut()) {
        config.remove("bypass_fog_sprites_crash");
    }
    header["version"] = REPLAY_SCHEMA_VERSION.into();
    Ok(version)
}

fn normalize_jsonl(bytes: &[u8], chunk: bool) -> Result<(Vec<u8>, u32)> {
    let newline = bytes
        .iter()
        .position(|b| *b == b'\n')
        .context("replay has no complete header")?;
    let mut header: serde_json::Value = serde_json::from_slice(&bytes[..newline])?;
    let version = upgrade_header(if chunk {
        header
            .get_mut("recording")
            .context("chunk has no recording header")?
    } else {
        &mut header
    })?;
    let mut output = serde_json::to_vec(&header)?;
    output.push(b'\n');
    output.extend_from_slice(&bytes[newline + 1..]);
    Ok((output, version))
}

fn prepare_source(source: &Path, staging: &Path) -> Result<(ReplayFile, u32)> {
    let source_bytes = if source.is_file() {
        Some(bounded_read(source)?)
    } else {
        None
    };
    let is_chunk = source_bytes
        .as_deref()
        .and_then(|bytes| bytes.split(|b| *b == b'\n').next())
        .and_then(|line| serde_json::from_slice::<serde_json::Value>(line).ok())
        .is_some_and(|header| header.get("recording").is_some());
    let (data, version) = if source.is_dir() || is_chunk {
        let root = if source.is_dir() {
            source
        } else {
            source.parent().context("chunk has no parent")?
        };
        let manifest_bytes = bounded_read(&root.join("mission.json"))?;
        let manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes)?;
        let chunks = manifest["chunks"]
            .as_array()
            .context("mission has no chunks")?;
        let target = staging.join("archive");
        std::fs::create_dir(&target)?;
        std::fs::write(target.join("mission.json"), manifest_bytes)?;
        let mut version = None;
        let mut total = 0;
        for (index, chunk) in chunks.iter().enumerate() {
            let name = chunk["file"].as_str().context("chunk has no filename")?;
            ensure!(
                name == format!("{index:08}.rhrec.jsonl"),
                "invalid chunk filename {name}"
            );
            let bytes = bounded_read(&root.join(name))?;
            total += bytes.len() as u64;
            ensure!(
                total <= MAX_SOURCE_BYTES,
                "mission archive exceeds upgrade size limit"
            );
            let (bytes, current) = normalize_jsonl(&bytes, true)?;
            ensure!(
                version.is_none_or(|v| v == current),
                "chunks use different schemas"
            );
            version = Some(current);
            std::fs::write(target.join(name), bytes)?;
        }
        (
            crate::replay_archive::load_directory(&target)?,
            version.context("empty mission archive")?,
        )
    } else {
        let bytes = source_bytes.context("replay input is not a file or mission directory")?;
        if bytes.starts_with(crate::replay_format::COMPACT_PREFIX) {
            let (_, data) = crate::replay_format::decode_compact(&bytes)?;
            let version = data.header().version;
            (data, version)
        } else {
            let (bytes, version) = normalize_jsonl(&bytes, false)?;
            (
                ReplayData::from_reader(std::io::Cursor::new(bytes)).map_err(anyhow::Error::msg)?,
                version,
            )
        }
    };
    crate::replay_format::validate_replay_data(&data)?;
    let mut file = ReplayFile::from(&data);
    // Embedded state needs its own schema migration; changing its version would
    // not reconstruct deleted or relocated simulation fields.
    for load in file.load_backs.values() {
        if let Some(snapshot) = &load.snapshot {
            let save: crate::save_file::GameSaveFile = serde_json::from_slice(&snapshot.payload)
                .context("embedded save requires migration before replay upgrade")?;
            save.validate_current_schema()?;
        }
    }
    file.hashes.clear();
    Ok((file, version))
}

fn write_replay(path: &Path, file: &ReplayFile) -> Result<()> {
    let mut output = std::io::BufWriter::new(std::fs::File::create(path)?);
    serde_json::to_writer(&mut output, &file.header)?;
    output.write_all(b"\n")?;
    for (ordinal, input) in &file.frames {
        let mut record = serde_json::json!({"f": ordinal, "i": input});
        if let Some(hash) = file.hashes.get(ordinal) {
            record["h"] = (*hash).into();
        }
        if let Some(marker) = file.save_markers.get(ordinal) {
            record["sv"] = serde_json::to_value(marker)?;
        }
        if let Some(load) = file.load_backs.get(ordinal) {
            record["lb"] = serde_json::to_value(load)?;
        }
        serde_json::to_writer(&mut output, &record)?;
        output.write_all(b"\n")?;
    }
    output.flush()?;
    output.get_ref().sync_all()?;
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn run_pass(
    options: &UpgradeOptions,
    replay: &Path,
    work: &Path,
    pass: &str,
    frames: u32,
) -> Result<Vec<(u64, u64)>> {
    use std::process::{Command, Stdio};
    let capture = work.join(format!("{pass}.hashes.jsonl"));
    let log = work.join(format!("{pass}.log"));
    let stdout = std::fs::File::create(&log)?;
    let mut command = Command::new(&options.executable);
    command
        .args(["--headless", "--no-sound", "--fast-forward", "--replay"])
        .arg(replay)
        .arg("--replay-hash-output")
        .arg(&capture)
        .env("ROBINHOOD_SAVE_DIR", work.join(format!("{pass}-saves")))
        .env_remove("ROBIN_WAIT_FOR_COMMAND")
        .stdin(Stdio::null())
        .stderr(stdout.try_clone()?)
        .stdout(stdout);
    #[cfg(feature = "script-rpc")]
    command.args(["--http-server", "0"]);
    let mut child = command.spawn().context("launch replay upgrade worker")?;
    let started = std::time::Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed().as_secs() >= options.timeout_seconds {
            child.kill()?;
            child.wait()?;
            anyhow::bail!(
                "{pass} exceeded {} seconds; output was not published",
                options.timeout_seconds
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    if !status.success() {
        let log = std::fs::read_to_string(log)?;
        let tail: Vec<_> = log.lines().rev().take(12).collect();
        anyhow::bail!(
            "{pass} failed ({status}):\n{}",
            tail.into_iter().rev().collect::<Vec<_>>().join("\n")
        );
    }
    read_capture(&capture, frames)
}

/// Upgrade a local replay into a new standalone JSONL artifact. Never replaces
/// the source or an existing destination. Both passes run the normal headless
/// mission loop, including host controls, save/load boundaries and finalization.
/// Every frame is compared, including the terminal frame. Published hashes
/// use the normal recorder checkpoint interval required by ranked playback.
/// Existing taints are retained exactly, with no migration-only taint added.
#[cfg(not(target_arch = "wasm32"))]
pub fn upgrade_replay(
    source: &Path,
    destination: &Path,
    options: &UpgradeOptions,
) -> Result<UpgradeReport> {
    ensure!(
        options.timeout_seconds > 0,
        "upgrade timeout must be positive"
    );
    ensure!(
        !destination.try_exists()?,
        "upgrade destination already exists: {}",
        destination.display()
    );
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let work = tempfile::Builder::new()
        .prefix(".replay-upgrade-")
        .tempdir_in(parent)?;
    let work_path = work.path().canonicalize()?;
    let (mut file, source_schema) = prepare_source(source, &work_path)?;
    let frames = file.header.total_frames;
    ensure!(frames > 0, "cannot upgrade an empty replay");
    let input = work_path.join("input.rhrec.jsonl");
    write_replay(&input, &file)?;
    let hashes = run_pass(options, &input, &work_path, "capture", frames)?;
    apply_captured_hashes(&mut file, &hashes);
    let upgraded = work_path.join("upgraded.rhrec.jsonl");
    write_replay(&upgraded, &file)?;
    let decoded = ReplayData::from_reader(std::io::BufReader::new(std::fs::File::open(&upgraded)?))
        .map_err(anyhow::Error::msg)?;
    crate::replay_format::validate_replay_data(&decoded)?;
    decoded
        .validate_ranked_hash_coverage()
        .map_err(anyhow::Error::msg)?;
    let verified = run_pass(options, &upgraded, &work_path, "verify", frames)?;
    verify_hashes(&hashes, &verified)?;
    let mut publication = tempfile::NamedTempFile::new_in(parent)?;
    std::io::copy(&mut std::fs::File::open(upgraded)?, &mut publication)?;
    publication.as_file().sync_all()?;
    publication
        .persist_noclobber(destination)
        .map_err(|error| error.error)?;
    #[cfg(unix)]
    std::fs::File::open(parent)?.sync_all()?;
    Ok(UpgradeReport {
        source_schema,
        target_schema: REPLAY_SCHEMA_VERSION,
        frames,
        hash_checkpoints: file.hashes.len(),
        save_markers: file.save_markers.len(),
        output: destination.to_owned(),
    })
}

fn apply_captured_hashes(file: &mut ReplayFile, hashes: &[(u64, u64)]) {
    file.hashes.clear();
    for (ordinal, &(before, after)) in hashes.iter().enumerate() {
        let ordinal = ordinal as u32;
        if ordinal.is_multiple_of(robin_engine::multiplayer::STATE_HASH_INTERVAL) {
            file.hashes.insert(ordinal, after);
        }
        if let Some(marker) = file.save_markers.get_mut(&ordinal) {
            marker.state_hash = before;
        }
    }
}

fn verify_hashes(captured: &[(u64, u64)], verified: &[(u64, u64)]) -> Result<()> {
    ensure!(
        captured.len() == verified.len(),
        "verification frame count changed"
    );
    for (ordinal, (first, second)) in captured.iter().zip(verified).enumerate() {
        ensure!(
            first == second,
            "replay is nondeterministic at frame {ordinal}: captured {first:?}, verified {second:?}; output was not published"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_capture_requires_dense_frames_and_successful_completion() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("hashes");
        let mut capture = HashCapture::create(&path).unwrap();
        assert!(capture.record(1, 1, 2).is_err());
        capture.record(0, 7, 8).unwrap();
        capture.file.flush().unwrap();
        assert!(read_capture(&path, 1).is_err());
        capture.finish().unwrap();
        assert_eq!(read_capture(&path, 1).unwrap(), vec![(7, 8)]);
        assert!(read_capture(&path, 2).is_err());
        assert!(HashCapture::create(&path).is_err());
    }

    #[test]
    fn schema_upgrade_preserves_input_and_rankability_evidence() {
        let input = br#"{"version":43,"rankability":{"status":"recorded","taints":[{"kind":"console_command","first_frame":4}]}}
{"f":0,"i":{"unchanged":true}}
"#;
        let (bytes, version) = normalize_jsonl(input, false).unwrap();
        assert_eq!(version, 43);
        let (header, records) = bytes.split_at(bytes.iter().position(|b| *b == b'\n').unwrap());
        let header: serde_json::Value = serde_json::from_slice(header).unwrap();
        assert_eq!(header["version"], REPLAY_SCHEMA_VERSION);
        assert_eq!(
            header["rankability"]["taints"][0]["kind"],
            "console_command"
        );
        assert_eq!(
            records,
            &input[input.iter().position(|b| *b == b'\n').unwrap()..]
        );
        for version in [42, REPLAY_SCHEMA_VERSION + 1] {
            assert!(upgrade_header(&mut serde_json::json!({"version":version})).is_err());
        }
        for version in 43..=53 {
            let mut header = serde_json::json!({
                "version": version,
                "sim_config": {"bypass_fog_sprites_crash": true, "fog_of_war": true}
            });
            assert_eq!(upgrade_header(&mut header).unwrap(), version);
            assert_eq!(header["version"], REPLAY_SCHEMA_VERSION);
            assert!(
                header["sim_config"]
                    .get("bypass_fog_sprites_crash")
                    .is_none()
            );
            assert_eq!(header["sim_config"]["fog_of_war"], true);
        }
    }

    #[test]
    fn upgraded_checkpoints_satisfy_ranked_coverage_without_per_frame_metadata() {
        let data = crate::leaderboard::test_fixtures::single_frame_replay(bitcode::encode(
            &robin_engine::campaign::Campaign::default(),
        ));
        let mut file = ReplayFile::from(&data);
        let frame = file.frames[&0].clone();
        file.header.total_frames = 4444;
        file.frames = (0..4444).map(|ordinal| (ordinal, frame.clone())).collect();
        let hashes = (0..4444u64)
            .map(|ordinal| (ordinal, ordinal + 1))
            .collect::<Vec<_>>();
        apply_captured_hashes(&mut file, &hashes);
        assert_eq!(file.hashes.len(), 178);
        assert_eq!(file.hashes[&25], 26);
        let data = ReplayData::try_from(file).unwrap();
        data.validate_ranked_hash_coverage().unwrap();
        let mut changed = hashes.clone();
        changed[4443].1 += 1;
        assert!(
            verify_hashes(&hashes, &changed).is_err(),
            "non-checkpoint terminal frames must still be verified"
        );
    }

    #[test]
    fn verification_checks_both_sides_of_loads_and_the_terminal_frame() {
        let hashes = [(1, 1), (2, 9), (10, 10)];
        verify_hashes(&hashes, &hashes).unwrap();
        assert!(verify_hashes(&hashes, &hashes[..2]).is_err());
        for (ordinal, sample) in [(1, (3, 9)), (1, (2, 8)), (2, (11, 11))] {
            let mut changed = hashes;
            changed[ordinal] = sample;
            let error = verify_hashes(&hashes, &changed).unwrap_err();
            assert!(error.to_string().contains(&format!("frame {ordinal}")));
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn existing_output_is_never_replaced_or_used_as_a_work_file() {
        let root = tempfile::tempdir().unwrap();
        let output = root.path().join("replay.jsonl");
        std::fs::write(&output, b"existing recording").unwrap();
        let error = upgrade_replay(
            &output,
            &output,
            &UpgradeOptions {
                executable: root.path().join("unused"),
                timeout_seconds: 1,
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("already exists"));
        assert_eq!(std::fs::read(&output).unwrap(), b"existing recording");
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
    }
}
