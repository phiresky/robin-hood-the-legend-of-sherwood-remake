# Image-to-3D alternatives (survey, September 2026)

Context: per-building reconstruction from the painted oblique-orthographic
maps (see `3d-reconstruction.md`). Baseline is `fal-ai/sam-3/3d-objects`.
"mask" = native mask input; "RGBA" = cutout with alpha, so our masks work
as-is.

| Model | Input | Output | Hosted / price / latency | Weights | Notes for our artwork |
|---|---|---|---|---|---|
| SAM 3D Objects (Meta, Nov 2025) | image + mask, multi-object | GLB, splat PLY, pose (R,t,s) | fal $0.02; 5–8 s single, 25–35 s multi | SAM License | Aerial-building study (arXiv 2512.22452) beats TRELLIS on roofs but "pose-agnostic texture" gives wrong yaw on symmetric buildings and layout drift. GitHub issue #71 on rotations unanswered. SAM 3D Align (fal $0.02) composes objects into one frame. |
| TRELLIS.2 (Microsoft, Dec 2025) | image (RGBA) | GLB + PBR | fal $0.25/$0.30/$0.35 at 512/1024/1536³, seconds; Replicate ~$1.09 | MIT, 4B, 24 GB VRAM | Best open quality, stylized-friendly; no pose output. TRELLIS v1 on fal $0.02. Flat-coloured art is a known weak spot. |
| Hunyuan3D 3.1 Pro (Tencent) | image (+multi-view) | GLB/OBJ, PBR | fal $0.375 (+$0.15 each: PBR, multi-view, face count); 2–6 min | closed | Highest ceiling in third-party tests. |
| Hunyuan3D 2.1 / Omni / Part | image; Omni adds 3D bbox / point-cloud control; Part = part generation | mesh + PBR | fal hunyuan3d/v2 $0.16 white, ~3× textured | community licence excludes EU/UK/KR for self-hosting | Via fal is fine. Omni bbox control could pin a footprint. |
| Tripo H3.1 (v3.1) | image (RGBA) | GLB/FBX, PBR, quad, parts | fal $0.20/$0.30/$0.40 by texture tier; official API 20–30 credits at $0.01 | closed | `orientation=align_image` rotates the model to match the input (needs texture=true) — the only hosted model with explicit image alignment. `generate_parts` may keep annexes. |
| Rodin Gen-2 (Hyper3D) | 1–N images | GLB/USDZ/FBX/OBJ | fal $0.40 | closed | Hero-asset quality, no mask or pose control. |
| Meshy 7 | image | glb/obj/fbx + PBR | API 20–35 credits (~$0.40–0.70) | closed | No mask or orientation control. |
| Hi3D (Hitem3D) | image | GLB/OBJ/…, PBR | fal, price not listed | closed | High-res watertight geometry. |
| Pixal3D | image | GLB | fal $0.30/$0.42; 6–7 min | closed | Mesh artifacts on stylized input. Skip. |
| Seed3D 2.0 (ByteDance) | image | mesh + PBR | Volcano Engine (China) only | closed | Not practically reachable. |
| Hi3DGen (MIT), Direct3D-S2 (MIT), Step1X-3D (Apache-2.0), PartCrafter (MIT) | image | mesh; PartCrafter generates several parts jointly | self-host | open | PartCrafter is the open answer to dropped annexes. SF3D/TripoSR/InstantMesh/CRM/Unique3D are superseded. |
| SceneGen (3DV 2026) | scene image + object masks | per-object GLB + relative positions | self-host, 16 GB | MIT | Indoor-trained. |
| MIDI-3D (VAST, CVPR 2025) | image + instance masks | textured multi-instance scene | self-host, ~30 GB | Apache-2.0 | Claims stylized generalization; synthetic indoor training. |
| 3D-Fixer (CVPR 2026) | single image | completed per-object splat/mesh + layout | self-host, 24 GB | Apache-2.0 | TRELLIS + MoGe-2 + SAM2 + Grounding DINO. |
| Gen3DSR / SceneConductor / SimuScene / RecGen | single image | object meshes + layout | research code, partly unreleased | mixed | All segment → per-object generator → layout fit. RecGen reports better pose than SAM 3D. |
| Marble (World Labs), HunyuanWorld / HY-World 2.0 | image → world | fused splat/mesh | Marble ~$1.26/world; fal image-to-world $0.30 | closed / EU-excluded | One fused perspective world, no per-building objects. Wrong tool. |
| MoGe-2 / Depth Anything 3 / MapAnything / VGGT | single or multi image | metric point map, depth, normals | Hugging Face | MIT / Apache (mostly) | Assume a pinhole camera; on an orthographic painting only relative depth/normals are usable (terrain, roof slopes). |

## Recommendation

Single building, ranked:

1. Tripo H3.1 (official API, ~$0.30 textured) with `orientation=align_image`,
   feeding the mask as an RGBA cutout. Only hosted model promising to rotate
   the output to match the input. Try `generate_parts` on buildings with
   annexes.
2. TRELLIS.2 on fal ($0.25–0.35) or self-hosted (MIT). Best open mesh
   quality and PBR; no pose output, so pair with our own yaw fit.
3. Solve yaw ourselves regardless of model (done: gravity snap + yaw search
   against the mask and crop, `reconstruct.ts`). The aerial-building study
   found SAM 3D's translation/scale reliable and only rotation bad.
4. Hunyuan3D 3.1 Pro only for hero buildings. Keep SAM 3D at $0.02 as the
   baseline.

Whole scene: nothing hosted turns a painted isometric map into many placed
buildings. Keep segment → reconstruct → compose. Worth one experiment:
MIDI-3D or SceneGen on a 5–10 building crop with our masks (both
indoor-trained). 3D-Fixer if occluded backs must be completed. Marble and
HunyuanWorld are the wrong tool. MoGe-2 / DA3 only for a relative terrain
height field.

Licensing: Hunyuan3D 2.1 / Omni and HY-World 2.0 community licences exclude
the EU, UK and South Korea for self-hosting; use via fal is unaffected.

Sources: fal.ai model pages for sam-3/3d-objects, sam-3/3d-align, trellis-2,
trellis, hunyuan-3d v3.1 pro, tripo3d h3.1, hyper3d/rodin, hitem3d, pixal3d,
hunyuan_world; github.com/facebookresearch/sam-3d-objects (+ issue #71);
arXiv 2512.22452; github.com/microsoft/TRELLIS.2; Tencent-Hunyuan repos
(2.1, Omni, Part, HY-World 2.0); docs.tripo3d.ai; docs.meshy.ai;
Stable-X/Stable3DGen; DreamTechAI/Direct3D-S2; stepfun-ai/Step1X-3D;
wgsxm/PartCrafter; Mengmouxu/SceneGen; VAST-AI-Research/MIDI-3D;
HorizonRobotics/3D-Fixer; AndreeaDogaru/Gen3DSR; docs.worldlabs.ai;
microsoft/MoGe; bytedance-seed/depth-anything-3; facebookresearch/map-anything.

## Trial on six York buildings (2026-09-02)

`pnpm reconstruct --asset <id> --backend trellis2|tripo|hunyuan` then
`pnpm compare-backends --assets ...` → `work/york-scene/backends.png`.
Inputs: the same masks as an RGBA cutout with 8% margin; fit by the yaw
search (no pose from these models). IoU = silhouette vs. mask under the map
camera, app = colour agreement of the unlit render with the crop.

| asset | SAM 3D (IoU / app) | TRELLIS.2 | Tripo H3.1 | Hunyuan 3.1 Pro |
|---|---|---|---|---|
| tower house | 0.83 / 0.87 | 0.81 / 0.69 | 0.96 / 0.79 | 0.78 / 0.77 |
| b038 half-timbered | 0.89 / 0.80 | 0.47 / 0.81 | 0.98 / 0.77 | 0.88 / 0.78 |
| b008 round tower | 0.85 / 0.90 | 0.83 / 0.80 | 0.93 / 0.82 | 0.97 / 0.83 |
| b027 house + stair | 0.89 / 0.85 | 0.37 / 0.77 | 0.97 / 0.84 | 0.81 / 0.72 |
| b013 gallery house | 0.84 / 0.80 | 0.37 / 0.73 | 0.93 / 0.80 | 0.80 / 0.76 |
| b005 tower house | 0.85 / 0.87 | 0.52 / 0.86 | 0.89 / 0.89 | 0.82 / 0.83 |
| tris | 11–67k | 92–99k | 1.40–1.44M (no face_limit) | 100k |
| time | ~35 s | 34–107 s | 130–190 s | 103–194 s |
| price | $0.02 | $0.30 | $0.30 | $0.375 |

- Tripo H3.1 with `orientation=align_image`: best and most consistent
  silhouettes (0.89–0.98), complete well-proportioned geometry, always the
  same yaw offset (≈270°) so the alignment works; textures slightly softer
  and greyer than SAM 3D. Needs `face_limit` (1.4M tris per house is 15×
  SAM 3D).
- SAM 3D: closest colours (highest app on 4/6), lightest meshes, cheapest by
  15×; geometry simplified and silhouettes a little off.
- Hunyuan 3.1 Pro: solid on towers (best on b008), boxy or squashed on the
  houses with annexes (b027, b013); darkest textures; slowest.
- TRELLIS.2: unusable on this artwork — 4/6 came back as thin slabs or
  fragments; fine only on the two towers.

Recommendation: keep SAM 3D as the default (cost, colours) and use Tripo
H3.1 with `face_limit` ~100k as the quality backend for hero buildings or
where the SAM 3D silhouette fit is weak.

### Does context help? (b027, b038; `--cutout context|soft`)

| input | TRELLIS.2 b027 / b038 | Tripo b027 / b038 |
|---|---|---|
| mask cutout, 8% margin | 0.37 / 0.47 | 0.97 / 0.98 |
| opaque crop, +10% uncrop | 0.43 / 0.72 | 0.84 / 0.88 |
| soft: building opaque, surroundings feathered over 20% | 0.58 / 0.73 | 0.92 / 0.81 |

With an opaque crop both models treat the whole picture as the object and
return the house on a wedge of terrain; the feathered cutout yields the
house on a ground pancake. TRELLIS.2 becomes less fragmented with context
but stays broken (b027 remains debris); Tripo is best with the plain mask
cutout. Conclusion: the alternatives want the isolated object; context only
helps SAM 3D, which takes the mask separately.
