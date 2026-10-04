"""Separate silhouette changes from shading drift in render-budget diagnostics."""
import numpy as np
from PIL import Image


def difference(first, second):
    a = np.asarray(Image.open(first).convert('RGBA')).astype(np.int16)
    b = np.asarray(Image.open(second).convert('RGBA')).astype(np.int16)
    if a.shape != b.shape:
        raise ValueError('Budget comparisons require identical image dimensions')
    delta = np.abs(a - b)
    opaque = (a[:, :, 3] > 127) | (b[:, :, 3] > 127)
    return dict(alpha_threshold_changed_pixels=int(np.count_nonzero((a[:, :, 3] > 127) != (b[:, :, 3] > 127))),
                alpha_max_delta=int(delta[:, :, 3].max()), rgba_max_delta=int(delta.max()),
                opaque_rgb_mean_delta=float(delta[:, :, :3][opaque].mean()) if opaque.any() else 0.,
                rgba_exact=bool(np.array_equal(a, b)))

