use std::path::PathBuf;
use std::time::{Duration, Instant};

#[cfg(feature = "audio")]
use kira::sound::{static_sound::StaticSoundData, streaming::StreamingSoundData};

fn time_it<T>(f: impl FnOnce() -> T) -> (T, Duration) {
    let start = Instant::now();
    let value = f();
    (value, start.elapsed())
}

#[cfg(feature = "audio")]
fn main() {
    let paths: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    if paths.is_empty() {
        eprintln!("usage: cargo run --example audio_decode_bench -- <audio-file>...");
        std::process::exit(2);
    }

    for path in paths {
        println!("== {} ==", path.display());

        let (metadata, metadata_time) = time_it(|| std::fs::metadata(&path));
        match metadata {
            Ok(metadata) => {
                println!(
                    "metadata: {:>8.3} ms  size={} bytes",
                    metadata_time.as_secs_f64() * 1000.0,
                    metadata.len()
                );
            }
            Err(err) => {
                println!(
                    "metadata: {:>8.3} ms  ERROR {err}",
                    metadata_time.as_secs_f64() * 1000.0
                );
                continue;
            }
        }

        let (read, read_time) = time_it(|| std::fs::read(&path));
        match read {
            Ok(bytes) => {
                println!(
                    "read all: {:>8.3} ms  bytes={}",
                    read_time.as_secs_f64() * 1000.0,
                    bytes.len()
                );
            }
            Err(err) => {
                println!(
                    "read all: {:>8.3} ms  ERROR {err}",
                    read_time.as_secs_f64() * 1000.0
                );
            }
        }

        let (streaming, streaming_time) = time_it(|| StreamingSoundData::from_file(&path));
        match streaming {
            Ok(data) => {
                println!(
                    "streaming open: {:>8.3} ms  duration={:.3}s frames={}",
                    streaming_time.as_secs_f64() * 1000.0,
                    data.duration().as_secs_f64(),
                    data.num_frames()
                );
            }
            Err(err) => {
                println!(
                    "streaming open: {:>8.3} ms  ERROR {err}",
                    streaming_time.as_secs_f64() * 1000.0
                );
            }
        }

        let (static_data, static_time) = time_it(|| StaticSoundData::from_file(&path));
        match static_data {
            Ok(data) => {
                println!(
                    "static decode: {:>8.3} ms  duration={:.3}s frames={}",
                    static_time.as_secs_f64() * 1000.0,
                    data.duration().as_secs_f64(),
                    data.num_frames()
                );
            }
            Err(err) => {
                println!(
                    "static decode: {:>8.3} ms  ERROR {err}",
                    static_time.as_secs_f64() * 1000.0
                );
            }
        }
    }
}

#[cfg(not(feature = "audio"))]
fn main() {
    eprintln!("audio_decode_bench requires the `audio` feature");
    std::process::exit(2);
}
