#!/usr/bin/env zsh
# Compress stream-split variants emitted by sprite_compression_probe --streams.
# Usage: compress_streams.sh /tmp/sprite_streams/RobinTown
set -e
d=$1
cd $d
Z() { zstd -22 --ultra --long=30 -q -f -c "$@" | wc -c }
X() { xz -9e -T1 -c "$@" | wc -c }
B() { bzip2 -9 -c "$@" | wc -c }

cat_z() { # name, files...
  local name=$1; shift
  cat "$@" > /tmp/_combo.bin
  printf "%-22s %10d  (raw %d)\n" $name $(Z /tmp/_combo.bin) $(stat -c %s /tmp/_combo.bin)
}

echo "== $d"
printf "%-22s %10d  (raw %d)\n" baseline-zstd $(Z baseline.bin) $(stat -c %s baseline.bin)
printf "%-22s %10d\n" baseline-xz $(X baseline.bin)
printf "%-22s %10d\n" baseline-bz2 $(B baseline.bin)

HDR=(hdr_w.u16 hdr_h.u16 hdr_d.u16 hdr_len.u32)
cat_z split-basic     $HDR rle_first.u16 rle_size.u16 rle_px.u16 vq_idx.u16
cat_z split-runlen    $HDR rle_first.u16 rle_runlen.u16 rle_px.u16 vq_idx.u16
cat_z split-planes    $HDR rle_first.lo rle_first.hi rle_size.lo rle_size.hi rle_px.lo rle_px.hi vq_idx.lo vq_idx.hi
cat_z split-rgb       $HDR rle_first.u16 rle_runlen.u16 rle_px_r.u8 rle_px_g.u8 rle_px_b.u8 vq_idx.lo vq_idx.hi
cat_z split-drgb      $HDR rle_first.u16 rle_runlen.u16 rle_px_dr.u8 rle_px_dg.u8 rle_px_db.u8 vq_idx.lo vq_idx.hi
cat_z split-d16       $HDR rle_first.u16 rle_runlen.u16 rle_px_d.lo rle_px_d.hi vq_idx.lo vq_idx.hi
cat_z split-pal       $HDR rle_first.u16 rle_runlen.u16 rle_px_pal.lo rle_px_pal.hi vq_idx.lo vq_idx.hi
cat_z split-pal-vqd   $HDR rle_first.u16 rle_runlen.u16 rle_px_pal.lo rle_px_pal.hi vq_idx_d.lo vq_idx_d.hi
cat_z split-vq-rank   $HDR vq_idx_rank.lo vq_idx_rank.hi
cat_z split-vq-rankd  $HDR vq_idx_rank_d.lo vq_idx_rank_d.hi
cat_z split-vq-upd    $HDR vq_up_d.lo vq_up_d.hi

echo "-- per-stream zstd"
for f in hdr_w.u16 hdr_h.u16 hdr_d.u16 hdr_len.u32 rle_first.u16 rle_size.u16 rle_runlen.u16 \
         rle_px.u16 rle_px.lo rle_px.hi rle_px_r.u8 rle_px_g.u8 rle_px_b.u8 \
         rle_px_d.u16 rle_px_dr.u8 rle_px_dg.u8 rle_px_db.u8 \
         rle_px_pal.u16 rle_px_pal.lo rle_px_pal.hi \
         vq_idx.u16 vq_idx.lo vq_idx.hi vq_idx_d.lo vq_idx_d.hi \
         vq_idx_rank.u16 vq_idx_rank.lo vq_idx_rank.hi vq_idx_rank_d.lo vq_idx_rank_d.hi \
         vq_up_d.u16 vq_up_d.lo vq_up_d.hi \
         dict.u16 dict.lo dict.hi dict_r.u8 dict_g.u8 dict_b.u8; do
  [ -f $f ] || continue
  printf "  %-18s raw %9d  zstd %9d\n" $f $(stat -c %s $f) $(Z $f)
done

echo "-- xz/bz2 on best-candidate combos"
cat $HDR rle_first.u16 rle_runlen.u16 rle_px.u16 vq_idx.u16 > /tmp/_combo.bin
printf "%-22s %10d\n" split-runlen-xz $(X /tmp/_combo.bin)
printf "%-22s %10d\n" split-runlen-bz2 $(B /tmp/_combo.bin)
cat $HDR rle_first.u16 rle_runlen.u16 rle_px_pal.lo rle_px_pal.hi vq_idx.lo vq_idx.hi > /tmp/_combo.bin
printf "%-22s %10d\n" split-pal-xz $(X /tmp/_combo.bin)
printf "%-22s %10d\n" split-pal-bz2 $(B /tmp/_combo.bin)
cat $HDR vq_idx_rank.lo vq_idx_rank.hi > /tmp/_combo.bin
printf "%-22s %10d\n" split-vq-rank-xz $(X /tmp/_combo.bin)
printf "%-22s %10d\n" split-vq-rank-bz2 $(B /tmp/_combo.bin)
cat $HDR vq_up_d.lo vq_up_d.hi > /tmp/_combo.bin
printf "%-22s %10d\n" split-vq-upd-xz $(X /tmp/_combo.bin)
rm -f /tmp/_combo.bin
