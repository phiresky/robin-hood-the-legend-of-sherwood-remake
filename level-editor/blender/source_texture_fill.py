"""Example-based synthesis using only explicitly owned source texels.

Donors stay within one logical asset and projection layer. The installed
texture-synthesis CLI expands observed patches; thin donors use mirrored fill.
This is a reversible display approximation, not additional reference evidence.
"""
import hashlib
import os
from pathlib import Path
import subprocess
from concurrent.futures import ThreadPoolExecutor
import numpy as np


def donor_patch(rgba, owned):
    """Largest wholly observed square, up to 64 pixels; never includes padding."""
    integral = np.pad(owned.astype(np.int32), ((1, 0), (1, 0))).cumsum(0).cumsum(1)
    for size in (64, 48, 32, 24, 16, 12, 8, 4, 2, 1):
        if min(owned.shape) < size:
            continue
        counts = integral[size:, size:] - integral[:-size, size:] - integral[size:, :-size] + integral[:-size, :-size]
        candidates = np.argwhere(counts == size * size)
        if len(candidates):
            y, x = candidates[len(candidates) // 2]
            return rgba[y:y+size, x:x+size, :3].copy()
    return None


def choose_donor(donors, normal_z, object_name):
    if not donors:
        return None
    def score(donor):
        patch, slope, mesh_name = donor
        return (abs(slope - normal_z) > .3,
                mesh_name != object_name, -patch.shape[0], abs(slope-normal_z))
    return min(donors, key=score)


def synthesize_tiles(donors, cache_dir, binary=None, jobs=8):
    """Expand selected observed patches with the existing texture-synthesis CLI.

The content-addressed cache includes generator version and all generation flags.
CLI failures are errors; absence of the binary is an explicitly reported fallback.
"""
    from PIL import Image
    binary = Path(binary or Path.home() / '.cargo/bin/texture-synthesis')
    if not binary.is_file():
        return {}, {"method": "mirrored donor fallback", "reason": "texture-synthesis binary missing", "binary": str(binary)}
    version = subprocess.run([str(binary), '--version'], check=True, capture_output=True, text=True).stdout.strip()
    cache = Path(cache_dir)
    cache.mkdir(parents=True, exist_ok=True)
    unique = {}
    thin = set()
    for donor in donors:
        if donor is None:
            continue
        patch = donor[0]
        if min(patch.shape[:2]) < 16:
            thin.add(id(patch))
            continue
        raw = np.rint(np.clip(patch, 0, 1)*255).astype(np.uint8)
        key = hashlib.sha256(raw.tobytes() + repr((raw.shape, version, 128, 0, 1, 'tiling')).encode()).hexdigest()
        unique[key] = (raw, patch)
    def generate(item):
        key, (raw, patch) = item
        source, output = cache / (key+'-source.png'), cache / (key+'.png')
        if not output.exists():
            Image.fromarray(raw).save(source)
            command = [str(binary), '--tiling', '--out-size', '128x128', '--threads', '1',
                       '--seed', '0', '--no-progress', '--out', str(output), 'generate', str(source)]
            subprocess.run(command, check=True, capture_output=True, text=True)
        with Image.open(output) as image:
            if image.size != (128, 128):
                raise ValueError('Invalid cached synthesis tile dimensions')
            result = np.asarray(image.convert('RGB'), dtype=np.float32)/255
        return key, result
    with ThreadPoolExecutor(max_workers=max(1, min(jobs, os.cpu_count() or 1))) as pool:
        generated = dict(pool.map(generate, unique.items()))
    lookup = {}
    for donor in donors:
        if donor is not None:
            patch = donor[0]
            if id(patch) in thin:
                continue
            raw = np.rint(np.clip(patch, 0, 1)*255).astype(np.uint8)
            key = hashlib.sha256(raw.tobytes() + repr((raw.shape, version, 128, 0, 1, 'tiling')).encode()).hexdigest()
            lookup[id(patch)] = generated[key]
    return lookup, {"method": "example-based texture synthesis", "generator": version,
                    "tiles": len(unique), "cache": str(cache), "tile_size": 128,
                    "thin_donor_mirror_fallbacks": len(thin),
                    "threads_per_tile": 1, "seed": 0, "tiling": True}


def fill_island(rgba, donors, normal_z, object_name, offset, synthesized_tiles=None):
    """Fill alpha-zero pixels, retaining every owned RGB value exactly.

Prefer similar surface inclination in the same mesh, then the same asset.
The donor repeats at the bake's texel density with mirrored boundaries.
No donor is a real absence of evidence and leaves the neutral fill intact.
"""
    if not donors:
        return 0
    patch, _, _ = choose_donor(donors, normal_z, object_name)
    generated = synthesized_tiles.get(id(patch)) if synthesized_tiles else None
    if generated is not None:
        patch = generated
    yy, xx = np.indices(rgba.shape[:2])
    size = patch.shape[0]
    if generated is None:
        xx = (xx + offset[0]) % (2 * size)
        yy = (yy + offset[1]) % (2 * size)
        xx = np.minimum(xx, 2 * size - 1 - xx)
        yy = np.minimum(yy, 2 * size - 1 - yy)
    else:
        xx = (xx + offset[0]) % size
        yy = (yy + offset[1]) % size
    unknown = rgba[:, :, 3] == 0
    rgba[:, :, :3][unknown] = patch[yy[unknown], xx[unknown]]
    return int(unknown.sum())


def prune_donors(donors):
    """Bound candidate lookup while retaining each mesh's surface inclinations."""
    best = {}
    for donor in donors:
        patch, slope, name = donor
        key = name, round(slope*10)
        if key not in best or patch.shape[0] > best[key][0].shape[0]:
            best[key] = donor
    return list(best.values())
