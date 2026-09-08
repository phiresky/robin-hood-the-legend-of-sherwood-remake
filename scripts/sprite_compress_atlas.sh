#!/usr/bin/env zsh
# Encode atlas outputs of sprite_compression_probe --atlas with modern codecs.
# Usage: compress_atlas.sh /tmp/sprite_streams/RobinTown [effort]
set -e
d=$1
e=${2:-7}
CJXL=${CJXL:-cjxl}
cd $d
read -r line < video.txt
W=$(echo $line | sed 's/.*width=\([0-9]*\).*/\1/')
H=$(echo $line | sed 's/.*height=\([0-9]*\).*/\1/')
F=$(echo $line | sed 's/.*frames=\([0-9]*\).*/\1/')
echo "== $d (video ${W}x${H}x${F}, jxl effort $e)"

# --- per-sheet JXL (rgb + rgba) and WebP ---
jxl_rgb=0; jxl_rgba=0; webp_rgb=0
for f in sheets/*.rgb.png; do
  $CJXL -d 0 -e $e --quiet $f /tmp/_s.jxl 2>/dev/null
  jxl_rgb=$((jxl_rgb + $(stat -c %s /tmp/_s.jxl)))
done
for f in sheets/*.rgba.png; do
  $CJXL -d 0 -e $e --quiet $f /tmp/_s.jxl 2>/dev/null
  jxl_rgba=$((jxl_rgba + $(stat -c %s /tmp/_s.jxl)))
done
for f in sheets/*.rgb.png; do
  magick $f -define webp:lossless=true -define webp:method=6 -define webp:exact=true /tmp/_s.webp
  webp_rgb=$((webp_rgb + $(stat -c %s /tmp/_s.webp)))
done
printf "%-24s %10d\n" sheets-jxl-rgb $jxl_rgb
printf "%-24s %10d\n" sheets-jxl-rgba $jxl_rgba
printf "%-24s %10d\n" sheets-webp-rgb $webp_rgb

# --- layout-only LZ ---
printf "%-24s %10d  (raw %d)\n" interleaved-565-zstd22 \
  $(zstd -22 --ultra --long=30 -q -c interleaved.rgb565 | wc -c) $(stat -c %s interleaved.rgb565)
printf "%-24s %10d\n" interleaved-565-xz $(xz -9e -T1 -c interleaved.rgb565 | wc -c)
printf "%-24s %10d\n" interleaved-565-bz2 $(bzip2 -9 -c interleaved.rgb565 | wc -c)

# --- video codecs on the 4x4 direction-grid stream ---
ffmpeg -hide_banner -loglevel error -y -f rawvideo -pix_fmt rgb24 -s ${W}x${H} -framerate 15 \
  -i video.rgb24 -vf format=gbrp -c:v ffv1 -level 3 -g 1 -context 1 /tmp/_v_ffv1.mkv
printf "%-24s %10d\n" video-ffv1 $(stat -c %s /tmp/_v_ffv1.mkv)

ffmpeg -hide_banner -loglevel error -y -f rawvideo -pix_fmt rgb24 -s ${W}x${H} -framerate 15 \
  -i video.rgb24 -vf format=gbrp -c:v libaom-av1 \
  -aom-params lossless=1:enable-palette=1:enable-intrabc=1 \
  -cpu-used 4 -row-mt 1 -threads 16 -g 240 /tmp/_v_av1.mkv
printf "%-24s %10d\n" video-av1-lossless $(stat -c %s /tmp/_v_av1.mkv)
rm -f /tmp/_s.jxl /tmp/_s.webp /tmp/_v_ffv1.mkv /tmp/_v_av1.mkv
