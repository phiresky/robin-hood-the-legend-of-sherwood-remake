//! Compare full engine clones with the native bitcode codec and zstd level 0.
//!
//! Build: cargo build --release -p robin_engine --example snapshot_storage_bench
//! Run: target/release/examples/snapshot_storage_bench SAVE.json [SAVE.json ...]
//!
//! Reports thread CPU time; excludes file loading, validation hashes, and live
//! level-resource reattachment. Compression contexts are reused. Retained byte
//! buffers are boxed slices, so their allocation size equals their length.
//! SNAPSHOT_BENCH_STACK_BYTES overrides the default 32 MiB worker stack.

use cpu_time::ThreadTime;
use robin_engine::engine::Engine;
use robin_util::state_hash::StateHash;
use serde::{Deserialize, Serialize};
use std::{hash::Hasher, hint::black_box};

const SAMPLES: usize = 9;
const OPERATIONS: usize = 32;
const RETAINED: usize = 8;

#[derive(Default, Serialize, Deserialize)]
struct Samples {
    create_us: Vec<f64>,
    drop_us: Vec<f64>,
}

fn sample<T>(mut create: impl FnMut() -> T, samples: &mut Samples) {
    let mut retained = Vec::with_capacity(RETAINED);
    let mut create_seconds = 0.0;
    let mut drop_seconds = 0.0;
    for _ in 0..OPERATIONS / RETAINED {
        let started = ThreadTime::now();
        for _ in 0..RETAINED {
            retained.push(black_box(create()));
        }
        create_seconds += started.elapsed().as_secs_f64();
        black_box(&retained);
        let started = ThreadTime::now();
        retained.clear();
        drop_seconds += started.elapsed().as_secs_f64();
    }
    samples
        .create_us
        .push(create_seconds * 1e6 / OPERATIONS as f64);
    samples.drop_us.push(drop_seconds * 1e6 / OPERATIONS as f64);
}

fn summarize(values: &[f64]) -> serde_json::Value {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    serde_json::json!({
        "median": sorted[sorted.len()/2],
        "min": sorted[0],
        "max": sorted[sorted.len()-1],
    })
}

impl Samples {
    fn summary(&self) -> serde_json::Value {
        serde_json::json!({
            "create_us": summarize(&self.create_us),
            "drop_us": summarize(&self.drop_us),
            "raw": self,
        })
    }
}

fn hash(engine: &Engine) -> u64 {
    let mut hasher = std::hash::DefaultHasher::new();
    engine.state_hash(&mut hasher);
    hasher.finish()
}

fn main() {
    let stack_bytes = std::env::var("SNAPSHOT_BENCH_STACK_BYTES")
        .map(|value| value.parse().expect("stack size in bytes"))
        .unwrap_or(32 * 1024 * 1024);
    std::thread::Builder::new()
        .stack_size(stack_bytes)
        .spawn(run)
        .expect("spawn snapshot benchmark")
        .join()
        .expect("snapshot benchmark panicked");
}

fn run() {
    assert!(
        !cfg!(debug_assertions),
        "run this benchmark in release mode"
    );
    let paths = std::env::args().skip(1).collect::<Vec<_>>();
    assert!(!paths.is_empty(), "provide a saved game JSON path");
    for path in paths {
        let mut value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).expect("read save")).expect("parse save");
        let engine: Engine = serde_json::from_value(value["engine"].take()).expect("decode engine");
        drop(value);
        eprintln!("{path}: encoding snapshot");
        let bytes = engine.encode_native_snapshot().into_boxed_slice();
        let mut compressor = zstd::bulk::Compressor::new(0).unwrap();
        let mut decompressor = zstd::bulk::Decompressor::new().unwrap();
        let compressed = compressor.compress(&bytes).unwrap().into_boxed_slice();
        let decompressed = decompressor.decompress(&compressed, bytes.len()).unwrap();
        assert_eq!(&*bytes, decompressed.as_slice());
        eprintln!("{path}: decoding snapshot");
        let restored = Engine::decode_native_snapshot(&decompressed).unwrap();
        assert_eq!(hash(&engine), hash(&restored));
        // Verify every surviving named field as well as the deterministic hash.
        assert_eq!(
            serde_json::to_value(&engine).unwrap(),
            serde_json::to_value(&restored).unwrap()
        );
        drop(restored);
        drop(decompressed);

        let mut clones = Samples::default();
        let mut encode = Samples::default();
        let mut decode = Samples::default();
        let mut compressed_encode = Samples::default();
        let mut compressed_decode = Samples::default();
        let mut zstd_encode = Samples::default();
        let mut zstd_decode = Samples::default();
        for round in 0..=SAMPLES {
            for offset in 0..7 {
                let mode = (round + offset) % 7;
                let mut warmup = Samples::default();
                match mode {
                    0 => sample(
                        || black_box(&engine).clone(),
                        if round == 0 { &mut warmup } else { &mut clones },
                    ),
                    1 => sample(
                        || {
                            black_box(&engine)
                                .encode_native_snapshot()
                                .into_boxed_slice()
                        },
                        if round == 0 { &mut warmup } else { &mut encode },
                    ),
                    2 => sample(
                        || Engine::decode_native_snapshot(black_box(&bytes)).unwrap(),
                        if round == 0 { &mut warmup } else { &mut decode },
                    ),
                    3 => sample(
                        || {
                            let bytes = black_box(&engine).encode_native_snapshot();
                            compressor.compress(&bytes).unwrap().into_boxed_slice()
                        },
                        if round == 0 {
                            &mut warmup
                        } else {
                            &mut compressed_encode
                        },
                    ),
                    4 => sample(
                        || {
                            let decoded = decompressor
                                .decompress(black_box(&compressed), bytes.len())
                                .unwrap();
                            Engine::decode_native_snapshot(&decoded).unwrap()
                        },
                        if round == 0 {
                            &mut warmup
                        } else {
                            &mut compressed_decode
                        },
                    ),
                    5 => sample(
                        || {
                            compressor
                                .compress(black_box(&bytes))
                                .unwrap()
                                .into_boxed_slice()
                        },
                        if round == 0 {
                            &mut warmup
                        } else {
                            &mut zstd_encode
                        },
                    ),
                    _ => sample(
                        || {
                            decompressor
                                .decompress(black_box(&compressed), bytes.len())
                                .unwrap()
                        },
                        if round == 0 {
                            &mut warmup
                        } else {
                            &mut zstd_decode
                        },
                    ),
                }
            }
        }
        println!(
            "{}",
            serde_json::json!({
                "save": path,
                "clock": "thread_cpu_time",
                "samples": SAMPLES,
                "operations_per_sample": OPERATIONS,
                "retained_batch": RETAINED,
                "entities": engine.entities_iter().count(),
                "bitcode_bytes": bytes.len(),
                "zstd_level": 0,
                "zstd_default_level": zstd::DEFAULT_COMPRESSION_LEVEL,
                "compressed_bytes": compressed.len(),
                "clone": clones.summary(),
                "bitcode_encode": encode.summary(),
                "bitcode_decode": decode.summary(),
                "bitcode_zstd_encode": compressed_encode.summary(),
                "bitcode_zstd_decode": compressed_decode.summary(),
                "zstd_encode": zstd_encode.summary(),
                "zstd_decode": zstd_decode.summary(),
            })
        );
    }
}
