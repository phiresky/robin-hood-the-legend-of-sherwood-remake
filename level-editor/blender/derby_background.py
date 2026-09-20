"""Authored depth for Derby's painted distant valley.

This is a height contribution to the terrain mesh, not another overlaid plane.
Coordinates are pixels of the 1920 x 2752 reference artwork; heights are game
height units. The terrain builder moves points along their projection rays, so
the reference view and UVs remain unchanged. Depth is an artistic approximation:
the painting supplies hill boundaries but cannot establish absolute distance.
"""

import math


REFERENCE_SIZE = (1920, 2752)

# The castle footing begins substantially below this background-only band.
# Negative heights place the distant valley behind the elevated castle plateau.
DEPTH_PROFILE = ((0.0, -900.0), (180.0, -600.0), (420.0, -220.0),
                 (680.0, 0.0))


def _depth_at(y):
    """Monotone cubic profile without artificial horizontal terraces.

    Interior derivatives are shared across adjoining intervals. The final zero
    derivative meets the castle plateau; positive secants prevent depth folds.
    """
    widths = [b[0]-a[0] for a, b in zip(DEPTH_PROFILE, DEPTH_PROFILE[1:])]
    slopes = [(b[1]-a[1])/width for a, b, width in zip(DEPTH_PROFILE, DEPTH_PROFILE[1:], widths)]
    derivatives = [slopes[0]]
    for i in range(1, len(DEPTH_PROFILE)-1):
        w1, w2 = 2*widths[i]+widths[i-1], widths[i]+2*widths[i-1]
        derivatives.append((w1+w2)/(w1/slopes[i-1]+w2/slopes[i]))
    derivatives.append(0.0)
    for i, ((y0, z0), (y1, z1)) in enumerate(zip(DEPTH_PROFILE, DEPTH_PROFILE[1:])):
        if y <= y1:
            t = (y-y0)/(y1-y0)
            return ((2*t**3-3*t**2+1)*z0 + (t**3-2*t**2+t)*(y1-y0)*derivatives[i]
                    + (-2*t**3+3*t**2)*z1 + (t**3-t**2)*(y1-y0)*derivatives[i+1])
    return DEPTH_PROFILE[-1][1]

# Broad field/woodland ridges traced from the source. They deliberately omit
# texture-scale trees, houses and cloud banks; those need separate geometry.
RIDGES = (
    (350.0, 185.0, 340.0, 125.0, 95.0),
    (1390.0, 220.0, 400.0, 135.0, 85.0),
    (1640.0, 405.0, 230.0, 150.0, 55.0),
    (135.0, 435.0, 190.0, 150.0, 65.0),
)


def _smoothstep(value):
    value = min(1.0, max(0.0, value))
    return value * value * (3.0 - 2.0 * value)


def height_at(screen_x, screen_y):
    """Return background game-height contribution, exactly zero below row 680.

    Combine with foreground height by addition; their authored supports must not
    overlap. This function is deterministic and independent of Blender state.
    """
    x, y = float(screen_x), float(screen_y)
    if not math.isfinite(x) or not math.isfinite(y):
        raise ValueError("Background coordinates must be finite")
    if not 0.0 <= x <= REFERENCE_SIZE[0] or y >= DEPTH_PROFILE[-1][0]:
        return 0.0
    if y <= DEPTH_PROFILE[0][0]:
        return DEPTH_PROFILE[0][1]
    depth = _depth_at(y)
    # Fade relief at both boundaries so derivatives meet the adjacent surface.
    envelope = _smoothstep(y / 120.0) * _smoothstep((680.0 - y) / 180.0)
    relief = sum(height * math.exp(-2.0 * (((x - cx) / rx) ** 2
                                          + ((y - cy) / ry) ** 2))
                 for cx, cy, rx, ry, height in RIDGES)
    return min(0.0, depth + relief * envelope)


def metadata():
    return {
        "name": "Derby distant valley",
        "reference_size": list(REFERENCE_SIZE),
        "screen_support": [0, 0, REFERENCE_SIZE[0], 680],
        "height_units": "game",
        "depth_profile": [list(point) for point in DEPTH_PROFILE],
        "evidence": "Broad valley fields and woodland ridges in the upper artwork",
        "certainty": "Authored relative relief; absolute depth is not recoverable from one painting",
        "limitations": [
            "Distant trees and village buildings remain painted surface details",
            "Cloud and atmospheric perspective remain baked into the artwork",
            "This surface does not define navigation or collision elevations",
        ],
    }
