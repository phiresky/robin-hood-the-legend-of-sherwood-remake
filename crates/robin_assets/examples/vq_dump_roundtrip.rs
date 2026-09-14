//! Round-trip dumped VQ grids (from a previous codec generation) through the
//! current sprite codec: encode, decode, compare every grid, report bytes and
//! decode cost. The dump comes from `examples/vq_grid_dump.rs` on the branch
//! that can still decode the old blobs; see that file for the format.
//!
//! cargo build -p robin_assets --release --example vq_dump_roundtrip
//! RAYON_NUM_THREADS=1 perf stat -e instructions:u \
//!     target/release/examples/vq_dump_roundtrip <file.vqdump> [decode rounds]
//!
//! Per-round decode instructions: (rounds=3 - rounds=1) / 2.

use anyhow::{Context, Result, bail, ensure};
use robin_assets::sprite_codec::{
    SelfRef, SpriteGrid, decode_grids_shipping, encode_grids_shipping,
};
use std::{path::PathBuf, time::Instant};

#[derive(clap::Parser, serde::Serialize, serde::Deserialize)]
struct Args {
    dump: PathBuf,
    #[arg(default_value_t = 1)]
    rounds: usize,
}

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8]> {
        ensure!(self.0.len() >= n, "truncated dump");
        let (head, tail) = self.0.split_at(n);
        self.0 = tail;
        Ok(head)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into()?))
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into()?))
    }
    fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into()?))
    }
    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into()?))
    }
    fn tiles(&mut self, n: usize) -> Result<Vec<u16>> {
        Ok(self
            .take(n * 2)?
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect())
    }
}

struct Chunk {
    alphabet: u16,
    dims: Vec<(u16, u16)>,
    grids: Vec<Vec<u16>>,
    base: Vec<Option<Vec<u16>>>,
    base2: Vec<Option<Vec<u16>>>,
    selfref: Vec<Option<SelfRef>>,
}

fn main() -> Result<()> {
    let args = <Args as clap::Parser>::parse();
    let bytes = std::fs::read(&args.dump).with_context(|| format!("read {:?}", args.dump))?;
    let mut r = Reader(&bytes);
    if r.take(8)? != b"VQDUMP01" {
        bail!("not a VQDUMP01 file");
    }
    let count = r.u32()? as usize;
    let fnv = r.u64()?;
    let shipped = r.u64()?;
    let mut chunks = Vec::with_capacity(count);
    for _ in 0..count {
        let alphabet = r.u16()?;
        let n = r.u32()? as usize;
        let mut dims = Vec::with_capacity(n);
        let mut flags = Vec::with_capacity(n);
        for _ in 0..n {
            dims.push((r.u16()?, r.u16()?));
            flags.push(r.u8()?);
        }
        let mut selfref = Vec::with_capacity(n);
        for &f in &flags {
            selfref.push(if f & 4 != 0 {
                Some(SelfRef {
                    grid: r.u32()?,
                    dtx: r.i32()?,
                    dy: r.i32()?,
                })
            } else {
                None
            });
        }
        let len = |d: (u16, u16)| usize::from(d.0) * usize::from(d.1);
        let grids = dims
            .iter()
            .map(|&d| r.tiles(len(d)))
            .collect::<Result<Vec<_>>>()?;
        let mut read_bases = |bit: u8| -> Result<Vec<Option<Vec<u16>>>> {
            dims.iter()
                .zip(&flags)
                .map(|(&d, &f)| (f & bit != 0).then(|| r.tiles(len(d))).transpose())
                .collect()
        };
        let base = read_bases(1)?;
        let base2 = read_bases(2)?;
        chunks.push(Chunk {
            alphabet,
            dims,
            grids,
            base,
            base2,
            selfref,
        });
    }
    ensure!(r.0.is_empty(), "trailing dump bytes");

    fn refs(list: &[Option<Vec<u16>>]) -> Vec<Option<&[u16]>> {
        list.iter().map(|b| b.as_deref()).collect()
    }
    let start = Instant::now();
    let mut blobs = Vec::with_capacity(chunks.len());
    for c in &chunks {
        let views: Vec<SpriteGrid> = c
            .dims
            .iter()
            .zip(&c.grids)
            .map(|(&(cols, rows), g)| SpriteGrid {
                cols,
                rows,
                indices: g,
            })
            .collect();
        blobs.push(encode_grids_shipping(
            c.alphabet,
            &views,
            Some(&refs(&c.base)),
            Some(&refs(&c.base2)),
            &c.selfref,
        )?);
    }
    let encode_ms = start.elapsed().as_secs_f64() * 1000.0;
    let total: usize = blobs.iter().map(Vec::len).sum();
    let tiles: usize = chunks.iter().flat_map(|c| &c.grids).map(Vec::len).sum();
    let family: usize = chunks
        .iter()
        .flat_map(|c| c.grids.iter().zip(&c.base))
        .filter(|(_, b)| b.is_some())
        .map(|(g, _)| g.len())
        .sum();

    let mut rounds = Vec::new();
    for _ in 0..args.rounds.max(1) {
        let start = Instant::now();
        for (c, blob) in chunks.iter().zip(&blobs) {
            let decoded = decode_grids_shipping(
                c.alphabet,
                &c.dims,
                Some(&refs(&c.base)),
                Some(&refs(&c.base2)),
                &c.selfref,
                blob,
            )?;
            ensure!(decoded == c.grids, "decoded grids differ from the dump");
        }
        rounds.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    rounds.sort_by(f64::total_cmp);
    println!(
        "{}: {} chunks, {tiles} tiles ({family} family), dump grids_fnv {fnv:016x}; \
         v16 {shipped} B -> current {total} B ({:+.2}%); encode {encode_ms:.0} ms; \
         decode x{} min {:.0} ms; all grids identical",
        args.dump.display(),
        chunks.len(),
        100.0 * (total as f64 / shipped as f64 - 1.0),
        rounds.len(),
        rounds[0]
    );
    Ok(())
}
