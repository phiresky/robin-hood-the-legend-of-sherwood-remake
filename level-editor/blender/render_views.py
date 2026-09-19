"""Render explicit camera bookmarks through Blender MCP, preserving scene state."""

import json
from pathlib import Path

import bpy


def render_views(scene_name, views, output_dir, modes=("textured", "solid"), width=1200):
    """views maps output labels to camera names; width preserves scene aspect ratio.

    Use separate output directories for before/after captures. Existing renders
    are rejected to preserve comparison evidence. Camera markers are temporarily
    detached so they cannot silently override an explicitly requested camera.
    """
    scene = bpy.data.scenes[scene_name]
    if not views or not modes or width < 1:
        raise ValueError("Supply cameras, render modes, and a positive width")
    if any(mode not in ("textured", "solid") for mode in modes):
        raise ValueError("Supported modes: textured, solid")
    cameras = {}
    out = Path(output_dir).resolve()
    for label, name in views.items():
        if not label or Path(label).name != label or label in (".", ".."):
            raise ValueError(f"Invalid output label: {label!r}")
        camera = scene.objects.get(name)
        if camera is None or camera.type != "CAMERA":
            raise ValueError(f"Scene camera not found: {name}")
        cameras[label] = camera
        for mode in modes:
            path = out / f"{label}-{mode}.png"
            if path.exists():
                raise FileExistsError(path)
    manifest = out / "renders.json"
    if manifest.exists():
        raise FileExistsError(manifest)
    render = scene.render
    state = {key: getattr(render, key) for key in (
        "engine", "resolution_x", "resolution_y", "resolution_percentage",
        "filepath", "use_border", "use_file_extension",
    )}
    camera_before = scene.camera
    format_before = render.image_settings.file_format
    markers = [(marker, marker.camera) for marker in scene.timeline_markers]
    window = bpy.context.window
    window_scene = window.scene if window else None
    records = []
    out.mkdir(parents=True, exist_ok=True)
    try:
        if window:
            window.scene = scene
        for marker, _ in markers:
            marker.camera = None
        render.resolution_y = max(1, round(width * state["resolution_y"] / state["resolution_x"]))
        render.resolution_x = width
        render.resolution_percentage = 100
        render.use_border = False
        render.use_file_extension = True
        render.image_settings.file_format = "PNG"
        for label, camera in cameras.items():
            scene.camera = camera
            for mode in modes:
                render.engine = "BLENDER_WORKBENCH" if mode == "solid" else state["engine"]
                if mode == "textured" and render.engine == "BLENDER_WORKBENCH":
                    raise ValueError("Select a textured render engine before capturing textured views")
                render.filepath = str(out / f"{label}-{mode}.png")
                bpy.ops.render.render(write_still=True, scene=scene.name)
                records.append({
                    "file": render.filepath, "camera": camera.name, "mode": mode,
                    "frame": scene.frame_current, "engine": render.engine,
                    "resolution": [render.resolution_x, render.resolution_y],
                    "camera_matrix": [list(row) for row in camera.matrix_world],
                    "camera_type": camera.data.type,
                    "ortho_scale": camera.data.ortho_scale,
                    "lens": camera.data.lens,
                })
        manifest.write_text(json.dumps({"scene": scene.name, "renders": records}, indent=2) + "\n")
    finally:
        for key, value in state.items():
            setattr(render, key, value)
        render.image_settings.file_format = format_before
        for marker, camera in markers:
            marker.camera = camera
        scene.camera = camera_before
        if window:
            window.scene = window_scene
    return records
