AVIF container fixtures for robin_assets tests, encoded with the pinned
toolchain (libavif 1.4.2 / libaom 3.15.0, scripts/install_pinned_avif_tools.sh):

- rgba8x4_lossless.avif: `avifenc -j 1 -y 444 -s 2 --lossless` of an 8x4 RGBA PNG
- rgba8x4_keyed_q60.avif: `avifenc -j 1 -y 444 -q 60 --qalpha 100 -s 2` of the same PNG
  (columns 0-1 alpha 0, pixel (7,3) alpha 128, everything else alpha 255)
- rgb2x3_q60.avif: `avifenc -j 1 -y 444 -q 60 -s 2` of a 2x3 solid red RGB PNG

Native tests cannot decode AVIF pixels (the web runtime uses the browser's
decoder), so tests parse these containers for dimensions/alpha and inject the
RGBA a browser decode would produce via `browser_images::insert_decoded`.
