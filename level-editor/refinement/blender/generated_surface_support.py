"""Reject generated background that intrudes into an approved silhouette."""
import numpy as np
from scipy.ndimage import binary_propagation


def support(image, approved_surface, maximum_rgb):
    """Keep dark interior details; reject only dark pixels connected to exterior.

    This is an explicit per-packet policy for sheets with a black background.
    It never changes image pixels, source ownership, or physical opacity.
    """
    if image.shape[:2] != approved_surface.shape or not 0 <= maximum_rgb <= .05:
        raise ValueError('Invalid generated background support contract')
    dark = np.max(image[:, :, :3], axis=2) <= maximum_rgb
    exterior = dark & ~approved_surface
    return ~binary_propagation(exterior, mask=dark)


def filtered_color(colors, weights, supported):
    weights = np.asarray(weights) * np.asarray(supported)
    total = weights.sum()
    return None if total <= 1e-8 else (np.asarray(colors) * weights[:, None]).sum(axis=0) / total


def edit_support(edit_mask, ownership_mask):
    """Accept only a binary subset of the reviewed unknown ownership mask."""
    if edit_mask.shape != ownership_mask.shape or edit_mask.ndim != 3 or edit_mask.shape[2] != 4:
        raise ValueError('Generated support dimensions differ from reviewed ownership')
    if not np.isin(edit_mask[:, :, 3], [0., 1.]).all():
        raise ValueError('Generated support alpha must be binary')
    supported = edit_mask[:, :, 3] == 0
    if not supported.any() or np.any(supported & (ownership_mask[:, :, 3] >= .5)):
        raise ValueError('Generated support must be a nonempty subset of reviewed unknown pixels')
    return supported
