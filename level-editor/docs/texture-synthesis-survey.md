# Tileable texture synthesis and hole filling (survey, September 2026)

Scope: synthesize a larger seamless texture from a tiny exemplar (60×40 wall,
200×200 ground), fill holes inside a tile coherently, run offline in Node or
via fal.ai. Complements `ai-texture-completion.md` (AI inpainting for the
same holes) and the fill rules in `3d-reconstruction.md`. Items marked
"unverified" were not confirmed from a primary source.

## Classic non-parametric methods

| Method | How it works | Fit for small exemplars / structured (brick, timber) | Speed | Implementations found |
|---|---|---|---|---|
| Efros-Leung (1999) | Grow output pixel by pixel; each new pixel is sampled from exemplar pixels whose neighborhood matches the already-synthesized neighborhood. | Works from small samples, but the IPOL authors note the range "between verbatim copy of the exemplar and garbage growing is somewhat narrow". | Exhaustive neighborhood search per output pixel; IPOL needed PCA acceleration to reach "acceptable time". | IPOL implementation (AGPL): https://www.ipol.im/pub/art/2013/59/ ; JS teaching demo, not an npm package: https://github.com/una-dinosauria/efros-and-leung-js ; C# with Harrison resynthesis too: https://github.com/mxgmn/TextureSynthesis |
| Wei-Levoy / Harrison "multiresolution stochastic" | Same idea with fixed causal neighborhoods, image pyramid, and accelerated search; Harrison's resynthesizer adds constraints and "practically never produces completely unsatisfactory results" (mxgmn README). | Good for stochastic textures; pixel-based methods tend to lose large structure (unverified for this style). | Multi-core; seconds per image. | EmbarkStudios texture-synthesis (Rust, MIT/Apache): single/multi-example, guided, style transfer, inpainting with mask, tiling mode, seeds; compiles to wasm32 (PR #91); repo archived, "not actively developing". https://github.com/EmbarkStudios/texture-synthesis , wasm PoC https://github.com/bnjbvr/texture-synthesis-wasm . GIMP Resynthesizer (C plugin): heal selection, render texture with seamless option, fill pattern seamless. https://github.com/bootchk/resynthesizer/wiki/Quick-user's-guide-to-the-Resynthesizer-plugins-for-GIMP |
| Image quilting, Efros-Freeman (2001) | Place exemplar blocks in raster order; candidate blocks chosen by SSD over the overlap within a tolerance; seam via min-error boundary cut (dynamic programming). | Handles stochastic and regular patterns; "large distinctive features" (planks) can propagate everywhere. | 250×250 from 125×125 with 20 px blocks: ~25 s in Python on a 2011 laptop, so a TS port is fine for 60×40 to 200×200. | Python: https://github.com/rohitrango/Image-Quilting-for-Texture-Synthesis , https://github.com/vamsi3/image-quilting ; no npm package found. Description/timings: http://jmecom.github.io/projects/computational-photography/texture-synthesis/ |
| Graph-cut textures, Kwatra (2003) | Paste patches at chosen offsets; min-cut over the overlap picks the seam; works in any dimension and can re-cut old seams. | Best classic method for structured textures; seams tracked so quality can be refined. | Needs a max-flow solver. | MATLAB https://github.com/jruales/Graphcut-Textures , Java/ImageJ https://github.com/panovr/GraphCutDemo , https://github.com/mjallais/GraphCuts . No JS/npm BK max-flow found; C libs compile to wasm: https://github.com/gerddie/maxflow . Paper: https://dl.acm.org/doi/10.1145/882262.882264 |
| Texture optimization, Kwatra (2005) | Global energy over all overlapping patches, minimized EM-style. Foundation for PatchMatch synthesis. | Good coherence; heavier. | Iterative. | Paper only: https://dl.acm.org/doi/10.1145/1073204.1073263 |
| PatchMatch, Barnes (2009) | Randomized nearest-neighbor field with propagation and random search; basis of Content-Aware Fill. | Excellent for hole filling from surrounding texture. | Fast (interactive in native implementations). | Python package with native components: https://github.com/vacancy/PyPatchMatch ; no JS package found. https://gfx.cs.princeton.edu/pubs/Barnes_2009_PAR/patchmatch.pdf . Note: antimatter15/inpaint.js is Telea diffusion (blurry), not PatchMatch: https://github.com/antimatter15/inpaint.js ; OpenCV Telea/NS is in the opencv.js whitelist (photo: inpaint) https://raw.githubusercontent.com/opencv/opencv/4.x/platforms/js/opencv_js.config.py (npm @techstark/opencv-js exposure unverified). |
| Wang tiles, Cohen (2003) | Small set of edge-colored tiles, filled by quilting samples, laid stochastically for non-periodic coverage. | Good for large ground areas; tile content still needs quilting/graph cut. | Runtime trivial. | npm "wang" gives only the 16-tile/6-color index layout https://www.npmjs.com/package/wang ; JS quilting-based Wang demo https://users.csc.calpoly.edu/~zwood/teaching/csc572/final12/kowen/ ; paper https://dl.acm.org/doi/10.1145/882262.882265 |
| Histogram-preserving blending, Heitz-Neyret (HPG 2018, "procedural stochastic texturing") | Gaussianize the exemplar histogram, blend 3 random patches per triangle of a grid with a variance-preserving operator, invert the transform. | Explicitly for "random-phase ... stochastic and non-periodic" inputs (moss, sand, bark). Not for bricks/planks. | GPU real-time; CPU JS demo "orders of magnitude" slower but works. | JS CPU demos (source not linked, license unknown): synthesis https://unity-grenoble.github.io/website/demo/2020/10/16/demo-histogram-preserving-blend-synthesis.html , make-tileable https://unity-grenoble.github.io/website/demo/2020/10/16/demo-histogram-preserving-blend-make-tileable.html ; https://eheitzresearch.wordpress.com/722-2/ ; Unity blog https://unity.com/blog/engine-platform/procedural-stochastic-texturing-in-unity ; hex-tiling variant without precompute: https://jcgt.org/published/0011/03/05/ |

## Learning-based synthesis and inpainting (hosted and offline)

- SD "tiling mode": patch every Conv2d to circular padding in UNet and VAE, giving self-tiling outputs; Tiled Diffusion (CVPR 2025) generalizes to mutually tileable sets. Text-driven, not exemplar-driven. https://arxiv.org/html/2412.15185v1 , https://github.com/CompVis/stable-diffusion/issues/250 . Replicate "Tileable SDXL" ~$0.011/run: https://replicate.com/pwntus/material-diffusion-sdxl
- Content-aware tile generation (Sartor & Peers, TOG 2024): reformulates tile generation as inpainting with exterior boundary conditions using off-the-shelf diffusion inpainting (SD2 inpainting at 256 px or SDXL at 512 px); code released, accepts an exemplar image, CUDA/MPS. Produces self-tiling, Wang, and dual Wang tiles. https://github.com/samsartor/content_aware_tiles , https://arxiv.org/abs/2409.14184
- TexTile (CVPR 2024): differentiable tileability score usable as a loss/QA metric. https://arxiv.org/abs/2403.12961
- SeamlessGAN (TVCG 2022) and Infinite Texture (2024) train a network per exemplar (SeamlessGAN "one order of magnitude less time than previous methods", but still per-texture training). Not practical for thousands of tiles. https://carlosrodriguezpardo.es/projects/SeamlessGAN/ , https://arxiv.org/abs/2405.08210
- Text2Tex and MaterialGAN: not investigated in depth; text-driven mesh texturing and SVBRDF capture respectively, not exemplar-based 2D synthesis.
- fal.ai PATINA: photo of a material to flattened, seamless PBR maps; $0.10 base + $0.02/MP + $0.01/MP per map. Photo-oriented; unverified on painted art. https://fal.ai/models/fal-ai/patina/material/extract
- Hosted mask+image inpainting (all take image_url + mask_url):
  - FLUX.1 [pro] Fill on fal: $0.05/MP, "billed by rounding up to the nearest megapixel", prompt required. Marketing claims style/lighting consistency; no evidence found for pixel-art or painted textures. https://fal.ai/models/fal-ai/flux-pro/v1/fill
  - FLUX.1 [dev] inpainting with LoRAs on fal: $0.035/MP, same rounding; LoRA hook for a style. https://fal.ai/models/fal-ai/flux-lora/inpainting ; flux-general inpainting $0.075/MP https://fal.ai/models/fal-ai/flux-general/inpainting
  - Bria Eraser on fal: $0.04 per generation, no prompt, object removal only. https://fal.ai/models/fal-ai/bria/eraser
  - fal-ai/inpaint (SD 1.5 / SDXL diffusers inpainting): billed per compute second, no flat price shown. https://fal.ai/models/fal-ai/inpaint
  - Replicate: LaMa ~$0.02/run on L40S https://replicate.com/twn39/lama ; FLUX Fill dev $0.025/image https://pricepertoken.com/image/model/black-forest-labs-flux-fill-dev ; Fill pro price not found.
- Offline neural inpainting in Node: LaMa has ONNX exports (https://huggingface.co/opencv/inpainting_lama , https://colab.research.google.com/github/Carve-Photos/lama/blob/main/export_LaMa_to_onnx.ipynb) that already run in onnxruntime-web (https://github.com/lxfater/inpaint-web), so onnxruntime-node (CPU or CUDA) works: https://www.npmjs.com/package/onnxruntime-node . MI-GAN (ICCV 2023) is ~10× cheaper than LaMa with ready ONNX files via IOPaint: https://github.com/Picsart-AI-Research/MI-GAN , https://www.iopaint.com/models/erase/migan . LaMa/MI-GAN are trained on photos; behavior on pixel-art texels is unverified.

## Making an arbitrary crop tileable

Seam-removal options, weakest to strongest:

1. Mirroring: seamless by construction but produces symmetric "butterfly" repeats (the artifact the current fill shows).
2. Edge crossfade (GIMP Tile Seamless): blends the image with its half-offset copy; GIMP docs say "result may need correction" (ghosting, contrast loss). https://docs.gimp.org/en/gimp-filter-tile-seamless.html , https://impr.hdyar.com/guides/seamlessTexture.html
3. Histogram-preserving blend of the borders: same crossfade idea without ghosting/contrast loss, stochastic textures only (JS demo above).
4. Offset by half, then inpaint a band along the cross-shaped seam: the Photoshop "offset + heal" workflow, automatable with any masked inpainter (Resynthesizer heal, PatchMatch, LaMa, FLUX Fill). https://www.gimpusers.com/tutorials/create-repeatable-seamless-textures
5. Synthesize on a toroidal domain: quilting/graph cut/texture-synthesis "tiling mode" with wrap-around overlaps yields tileable output by construction, no post-fix.

## Recommendation (pragmatic order)

1. Implement constrained image quilting with min-cut in TypeScript. A few hundred lines, no dependencies, milliseconds at these sizes, and it handles both stochastic ground and structured walls (brick courses, timber framing) better than pixel-growing methods. Use a toroidal output for tileability (option 5 above) and treat known pixels as fixed constraints for hole filling, so the same code covers synthesis and hole filling. Upgrade seams to graph cut later if DP cuts show artifacts.
2. Ship the EmbarkStudios texture-synthesis crate as a CLI sidecar or wasm-pack module for a second opinion: it already does inpainting from a mask, tiling mode, and multi-exemplar remixing. Caveat: archived, pixel-based, so structured facades may blur; test on a 60×40 wall first. Since the repo is already a Rust workspace, a Rust example binary is the cheapest integration.
3. AI inpainting on fal is worth a pilot, not a default. Cost estimate for ~3000 tiles: FLUX Fill pro tile-by-tile is $0.05 minimum per call because of megapixel rounding, so ~$150 (Bria Eraser ~$120, FLUX dev LoRA ~$105). Packing tiles into 1024×1024 atlases (about 1 MP each) cuts it to roughly 8 to 120 MP of real pixels, i.e. $0.40 to $6 depending on tile size, but cross-cell context bleed and style drift on pixel-art are unverified. Run a 20-tile pilot on the 200×200 ground patches first, where repetition is most visible; keep classic methods for small wall tiles. Free alternative: LaMa or MI-GAN through onnxruntime-node, likely adequate for hole filling but may soften texels.

Uncertainties: no JS/npm implementation of PatchMatch, quilting, graph cut, or BK max-flow was found; opencv.js inpaint availability in the @techstark npm build is unconfirmed; no source found on how FLUX Fill or LaMa behave on 1-texel-per-pixel painted art.

## Software in any language (survey, 2026-09-03)

Facts from web sources; speeds unverified unless cited.

### Exemplar-based synthesis (enlarge from a tiny exemplar)

- **EmbarkStudios texture-synthesis** (Rust lib + CLI, MIT/Apache-2.0): multiresolution stochastic pixel synthesis (Wei-Levoy/Ashikhmin k-coherence with backtracking), multi-example + guide maps; `--out-size`, `--in-size`, `--seed`, `--inpaint <mask>` / `--inpaint-channel a`, `--tiling`, `--threads`, `transfer-style`, `repeat`. Repo archived 2023-10, CLI 0.8.3 (2022-02), no active fork. `cargo install --locked texture-synthesis-cli` (unlocked build fails on current Rust). README: "not great with regular textures (seams can become obvious)". https://github.com/EmbarkStudios/texture-synthesis
- **G'MIC** (`apt install gmic`, CeCILL, active, 3.6.5 2025-12): `syntexturize_matchpatch W,H[,scales,patch,blend,precision]` (multi-scale PatchMatch resynthesis), `syntexturize` (random-phase noise, micro-textures only), `inpaint_matchpatch [mask],scales,patch,iters,blend,outer`, `inpaint [mask],patch,...` (older patch method), `inpaint_pde`, `frame_seamless size[,patch,blend]` (make tileable). Same filters inside Krita/GIMP via G'MIC-Qt. https://gmic.eu/reference/syntexturize_matchpatch.html
- **Resynthesizer** (Harrison 2001, bootchk, GPL-3): GIMP "heal selection"; `libresynthesizer.a` re-entrant; standalone C port miinso/resynthesizer with a PPM CLI. Reputedly the most robust classic method; GPL means subprocess only. https://github.com/bootchk/resynthesizer
- **WaveFunctionCollapse overlapping model** (npm `wavefunctioncollapse` MIT, zero deps, in-process; Rust `wfc_image`): output contains only N×N patterns present in the exemplar, so pixel-art brick courses and timber framing survive; periodic output; no hole filling; contradictions on rich-colour exemplars (quantise first). https://www.npmjs.com/package/wavefunctioncollapse
- **mxgmn/TextureSynthesis** (C#, MIT): Efros-Leung, k-coherent and Harrison; demo code, not a CLI. https://github.com/mxgmn/TextureSynthesis
- **Image quilting**: no maintained package on PyPI/npm/crates.io; IPOL implementation (AGPL) https://www.ipol.im/pub/art/2017/171/ ; Julia ImageQuilting.jl (MIT, geostatistics-oriented).
- **Wang tiles**: nothing usable off the shelf.
- **Neural**: `pip install texturize` (Gatys-style, AGPL, last release 2023, CPU slow, photo-oriented); Self-Tuning Texture Optimization (Kaspar 2015, MATLAB) targets structured + stochastic but is research code.

### Hole filling only

- **IOPaint** (`pip install iopaint`, Apache-2.0, archived 2025-08): `iopaint run --model=lama --device=cpu --image=DIR --mask=DIR --output=DIR`; LaMa/MI-GAN/FcF/MAT. ONNX exports of LaMa (Carve/LaMa-ONNX, 512² fixed) and MI-GAN run in onnxruntime-node (unverified).
- **PyPatchMatch** (`pip install pypatchmatch`, MIT, compiles native components on first import): `patch_match.inpaint(img, mask, patch_size)`; fast, no tiling/enlargement.
- **IPOL non-local patch inpainting** (Newson, GPL-3, source tarball).
- **OpenCV** `cv2.inpaint` Telea/NS (diffusion, smears), `cv2.xphoto.inpaint` SHIFTMAP/FSR (opencv-contrib); scikit-image `inpaint_biharmonic` and the Rust `inpaint` crate are diffusion only.

### Make-seamless only

`pip install img2texture` (overlap blend), Fred's ImageMagick `tiler` (non-commercial), G'MIC `frame_seamless`.

### Trial on York tiles (2026-09-03)

Inputs: `--fill none` tiles (own pixels, alpha-0 holes) of the cathedral wall (33 % visible), the nave wall (90 %), a roof (45 %), two ground crops; exemplar crops 190² stone and 150² ground. Sheet: `work/york-scene/compare-gmic-vs-texture-synthesis.png`.

| tile | G'MIC `inpaint` patch 11 | G'MIC `inpaint_matchpatch` | texture-synthesis `--inpaint` |
|---|---|---|---|
| cathedral wall | windows scattered, snow blobs | smoother, windows smeared | continuous stone, windows in rows, one snow ledge |
| nave wall | battlement copied into the hole | — | clean continuation |
| roof | grass/rock patches in the roof | — | coherent shingle texture |
| ground under a wall | streaked repeats | blurred, coherent | coherent, rocks and snow |
| river bank | duplicated posts | — | fewer duplicates |
| time (16 cores) | 1.5–5.5 s | 14–17 s | 0.5–1.3 s |

`syntexturize` (phase noise) turns everything into noise; `syntexturize_matchpatch` and `generate --tiling` both give usable stone; ground exemplars repeat their one rock in both. Conclusion: texture-synthesis is the tool to integrate (`--fill synth` in `volumes.ts`), G'MIC is the fallback that needs no cargo build, WFC is the candidate for structured timber/brick walls.
