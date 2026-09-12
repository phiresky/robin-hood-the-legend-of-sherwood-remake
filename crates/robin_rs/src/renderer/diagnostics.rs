//! Renderer-owned diagnostic counters; captures cannot drain another renderer.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fmt::Write as _;

#[derive(Default, serde::Serialize, serde::Deserialize)]
pub(super) struct UploadCounters {
    count: Cell<usize>,
    labels: RefCell<HashMap<String, usize>>,
}

impl UploadCounters {
    pub(super) fn inc(&self, label: &str) {
        self.count.set(self.count.get() + 1);
        *self
            .labels
            .borrow_mut()
            .entry(label.to_owned())
            .or_default() += 1;
    }

    pub(super) fn take_count(&self) -> usize {
        self.count.replace(0)
    }

    fn take_labels(&self) -> String {
        format_labels(self.labels.borrow_mut().drain().collect())
    }
}

fn format_labels(mut entries: Vec<(String, usize)>) -> String {
    if entries.is_empty() {
        return "-".to_owned();
    }
    entries.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let mut summary = String::new();
    for (index, (label, count)) in entries.into_iter().take(6).enumerate() {
        if index > 0 {
            summary.push(',');
        }
        write!(summary, "{label}:{count}").expect("writing to a String cannot fail");
    }
    summary
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
pub(super) struct FrameCounters {
    binds: Cell<usize>,
    draw_calls: Cell<usize>,
    fps: RefCell<FpsState>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct FpsState {
    frames: u32,
    draws_total: usize,
    uploads_total: usize,
    binds_total: usize,
    draw_calls_total: usize,
    present_total_us: u64,
    #[serde(skip, default = "web_time::Instant::now")]
    last: web_time::Instant,
}

impl Default for FpsState {
    fn default() -> Self {
        Self {
            frames: 0,
            draws_total: 0,
            uploads_total: 0,
            binds_total: 0,
            draw_calls_total: 0,
            present_total_us: 0,
            last: web_time::Instant::now(),
        }
    }
}

impl FrameCounters {
    pub(super) fn inc_bind(&self) {
        self.binds.set(self.binds.get() + 1);
    }
    pub(super) fn inc_draw_call(&self) {
        self.draw_calls.set(self.draw_calls.get() + 1);
    }
    pub(super) fn take_binds(&self) -> usize {
        self.binds.replace(0)
    }
    pub(super) fn take_draw_calls(&self) -> usize {
        self.draw_calls.replace(0)
    }

    pub(super) fn log_fps(&self, draws: usize, present_us: u64, resources: &super::GpuResources) {
        let mut g = self.fps.borrow_mut();
        g.frames += 1;
        g.draws_total += draws;
        g.uploads_total += resources.uploads.take_count();
        g.binds_total += self.take_binds();
        g.draw_calls_total += self.take_draw_calls();
        g.present_total_us = g.present_total_us.wrapping_add(present_us);
        if g.last.elapsed().as_secs() < 1 {
            return;
        }
        let atlas = resources.sprite_atlas.stats();
        let residency = resources.sprite_residency_stats();
        let frames = g.frames as usize;
        tracing::debug!(target:"fps",
            sprite_cache_entries=residency.entries,
            distinct_sprite_frames=residency.distinct_frames,
            resident_sprite_bytes=residency.resident_bytes,
            atlas_packing_efficiency=atlas.packing_efficiency(),
            "{} fps  quads/f={}  drawcalls/f={}  binds/f={}  uploads/f={}  present={:.2}ms  atlas={}L/{:.0}MiB/{:.0}%occ/{}spr  upload_labels={}",
            g.frames,g.draws_total/frames,g.draw_calls_total/frames,g.binds_total/frames,g.uploads_total/frames,
            (g.present_total_us/u64::from(g.frames)) as f32/1000.0,atlas.layers,
            atlas.bytes() as f32/(1024.0*1024.0),atlas.occupancy()*100.0,atlas.sprites,resources.uploads.take_labels());
        *g = FpsState::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn independent_renderers_do_not_drain_each_others_diagnostics() {
        let first = UploadCounters::default();
        let second = UploadCounters::default();
        first.inc("first");
        second.inc("second");
        second.inc("second");
        assert_eq!(first.take_count(), 1);
        assert_eq!(second.take_count(), 2);
        assert_eq!(first.take_labels(), "first:1");
        assert_eq!(second.take_labels(), "second:2");
        let first = FrameCounters::default();
        let second = FrameCounters::default();
        first.inc_bind();
        second.inc_draw_call();
        assert_eq!(first.take_draw_calls(), 0);
        assert_eq!(second.take_binds(), 0);
        assert_eq!(first.take_binds(), 1);
        assert_eq!(second.take_draw_calls(), 1);
    }

    #[test]
    fn label_summary_preserves_count_order_ties_limit_and_empty_marker() {
        assert_eq!(format_labels(Vec::new()), "-");
        let entries = [
            ("z", 2),
            ("f", 1),
            ("a", 2),
            ("e", 1),
            ("b", 3),
            ("d", 1),
            ("c", 1),
        ]
        .into_iter()
        .map(|(label, count)| (label.to_owned(), count))
        .collect();
        assert_eq!(format_labels(entries), "b:3,a:2,z:2,c:1,d:1,e:1");
    }
}
