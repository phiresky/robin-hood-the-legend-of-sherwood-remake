"""Full-map capture acceptance on an already paused, isolated live game."""
import hashlib
import struct


def dimensions(png):
    if len(png) < 24 or png[:8] != b"\x89PNG\r\n\x1a\n" or png[12:16] != b"IHDR":
        raise RuntimeError("capture did not return a PNG with an IHDR")
    width, height = struct.unpack(">II", png[16:24])
    if not width or not height:
        raise RuntimeError("capture returned empty dimensions")
    return width, height


def exercise_capture(request, screenshot, evidence, summary):
    # Hide time-dependent HUD presentation; gameplay is paused by the driver.
    before_state = request("/engine-dump")
    before = screenshot("hide_ui=true")
    view_size = dimensions(before)
    (evidence / "capture-viewport-before.png").write_bytes(before)
    captures = []
    for index in range(2):
        full = screenshot("full_map=true&hide_ui=true")
        full_size = dimensions(full)
        if full_size[0] <= view_size[0] or full_size[1] <= view_size[1]:
            raise RuntimeError("Leicester full-map capture did not exceed viewport dimensions")
        (evidence / f"capture-map-{index}.png").write_bytes(full)
        after = screenshot("hide_ui=true")
        (evidence / f"capture-viewport-after-{index}.png").write_bytes(after)
        if after != before:
            raise RuntimeError("full-map capture changed the paused viewport image")
        if request("/engine-dump") != before_state:
            raise RuntimeError("full-map capture changed paused engine state")
        captures.append(full)
    if captures[0] != captures[1]:
        raise RuntimeError("repeated paused full-map captures differ")
    summary["capture"] = {
        "viewport_dimensions": view_size,
        "map_dimensions": dimensions(captures[0]),
        "viewport_sha256": hashlib.sha256(before).hexdigest(),
        "map_sha256": hashlib.sha256(captures[0]).hexdigest(),
    }
    summary["checks"]["full_map_preserves_live_view_and_state"] = True
    summary["checks"]["repeated_full_map_pixels_match"] = True
