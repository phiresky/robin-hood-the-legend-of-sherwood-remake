# BC7f worker module

Generated from unmodified Basis Universal revision
`99f52d63aa6799cbdaecfe977111dc5ec3b31d47` and the image/mipmap adapter in
`level-editor/benchmarks/bc7f/bc7f.cpp`. This exposes the analytical BC7f block
encoder directly; it does not encode an intermediate Basis/UASTC representation.

The checked-in build uses Emscripten 4.0.22, LLVM 22, Binaryen 132, `-O3`, LTO and
WASM SIMD. Debug information is requested to preserve import names across the
local Emscripten/Binaryen versions, then stripped from the final WASM. Generated
JavaScript is formatted with the workspace formatter and excluded from lint.
The scalar benchmark build remains available for comparison.

From the repository root, after checking out the pinned upstream revision under
`level-editor/work/bc7f-benchmark/basis_universal`:

```sh
BC7F_APP=1 BC7F_SIMD=1 bash level-editor/benchmarks/bc7f/build.sh
pnpm --dir level-editor exec oxfmt app/src/vendor/bc7f/bc7f.js
```

`EMXX` can select an alternative `em++` executable. The benchmark README describes
how to fetch the source, build both variants and reproduce the measurements.
No game images are included in this module. AVIF files remain the delivery format.

Basis Universal is Apache-2.0; see LICENSE and NOTICE. The generated Emscripten
runtime is covered by EMSCRIPTEN-LICENSE. The adapter is project source code;
upstream encoder source has not been edited.
