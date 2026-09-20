"""Prepare and validate independent, source-only Blender refinement workspaces.

Run in a background Blender process, never the shared interactive scene. A worker
owns one logical asset; the surrounding scene remains available for occlusion.
"""
import argparse
import hashlib
import json
import shutil
import shlex
import sys
import uuid
from pathlib import Path


def _json(path, value):
    Path(path).write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")


def _sha(path):
    h = hashlib.sha256()
    with Path(path).open("rb") as file:
        for block in iter(lambda: file.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def _files(directory):
    return {str(p.relative_to(directory)): _sha(p)
            for p in sorted(Path(directory).rglob("*")) if p.is_file()}


def _copy_manifest_images(value, source_dir, destination_dir):
    """Copy every referenced patch/frame image, including nested mission assets."""
    if isinstance(value, dict):
        return {key: _copy_manifest_images(item, source_dir, destination_dir)
                for key, item in value.items()}
    if isinstance(value, list):
        return [_copy_manifest_images(item, source_dir, destination_dir) for item in value]
    if isinstance(value, str) and Path(value).suffix.lower() in (".png", ".webp", ".jpg", ".jpeg"):
        relative = Path(value)
        source = (source_dir / relative).resolve(strict=True)
        if relative.is_absolute() or ".." in relative.parts:
            relative = Path("external") / (_sha(source)[:12] + "-" + source.name)
        destination = destination_dir / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        if destination.exists() and _sha(destination) != _sha(source):
            raise ValueError(f"Conflicting reference image name: {relative}")
        shutil.copy2(source, destination)
        return relative.as_posix()
    return value


def _absolute_manifest_images(value, directory):
    if isinstance(value, dict):
        return {key: _absolute_manifest_images(item, directory) for key, item in value.items()}
    if isinstance(value, list):
        return [_absolute_manifest_images(item, directory) for item in value]
    if isinstance(value, str) and Path(value).suffix.lower() in (".png", ".webp", ".jpg", ".jpeg"):
        return str((directory / value).resolve(strict=True))
    return value


def _geometry(obj):
    value = {"type": obj.type, "matrix": [list(row) for row in obj.matrix_world],
             "parent": obj.parent.name if obj.parent else None,
             "hide_render": obj.hide_render, "hide_viewport": obj.hide_viewport,
             "source_node": obj.get("source_node"), "asset_group": obj.get("asset_group")}
    if obj.type == "MESH":
        value["vertices"] = [list(v.co) for v in obj.data.vertices]
        value["faces"] = [list(p.vertices) for p in obj.data.polygons]
        value["edges"] = [list(e.vertices) for e in obj.data.edges]
        # Active modifiers alter effective geometry without changing base vertices.
        value["modifiers"] = [(m.name, m.type, m.show_viewport, m.show_render)
                              for m in obj.modifiers]
        if any(m.show_viewport or m.show_render for m in obj.modifiers):
            import bpy
            evaluated = obj.evaluated_get(bpy.context.evaluated_depsgraph_get())
            mesh = evaluated.to_mesh()
            try:
                value["evaluated_vertices"] = [list(v.co) for v in mesh.vertices]
                value["evaluated_faces"] = [list(p.vertices) for p in mesh.polygons]
            finally:
                evaluated.to_mesh_clear()
    return hashlib.sha256(json.dumps(value, sort_keys=True).encode()).hexdigest()


def _objects(config):
    import bpy
    return list(bpy.data.collections[config["collection_name"]].all_objects)


def _ownership(config):
    import bpy
    objects = _objects(config)
    target = [o for o in objects if o.type == "MESH" and o.get("asset_group") == config["asset_id"]]
    if not target:
        raise ValueError("Asset has no meshes in the reviewed working collection")
    if any(not o.get("source_node") for o in target):
        raise ValueError("Every asset component must retain a stable source_node")
    return target, {o.name: _geometry(o) for o in bpy.data.scenes[config["scene_name"]].objects
                    if o not in target}


def _review_layers(config):
    if not config.get("projection_manifest"):
        return None
    from interior_layers import projection_receivers, projection_occluders
    path = Path(config["projection_manifest"])
    manifest = json.loads(path.read_text())
    interior = projection_receivers(manifest)
    available = {o.get("source_node") for o in _objects(config)
                 if o.type == "MESH" and not o.hide_render}
    exterior = available - {node for nodes in interior.values() for node in nodes}
    occluders = projection_occluders(manifest, available)
    partitions = [("exterior", "exterior", sorted(exterior), sorted(exterior))]
    partitions.extend(("interior", "interior-" + patch, sorted(nodes), occluders[patch]) for patch, nodes in interior.items())
    exterior_source = _mission_review_source(config) or str((path.parent / manifest['sources']['exterior']).resolve())
    return [{"source_path": exterior_source if source == 'exterior' else str((path.parent / manifest["sources"][source]).resolve()),
             "receiver_nodes": nodes, "occluder_nodes": blockers,
             **({"projection_label": label} if config.get('source_mask_manifest') else {})}
            for source, label, nodes, blockers in partitions]


def _mission_review_source(config):
    """Use an explicitly modeled mission endpoint, never unrelated base pixels."""
    if not config.get('projection_manifest'):
        return None
    path = Path(config['projection_manifest'])
    manifest = json.loads(path.read_text())
    sources = set()
    for obj in _objects(config):
        if obj.type != 'MESH' or obj.hide_render or not obj.get('mission_patch_profile'):
            continue
        mission, profile = obj.get('mission_patch_mission'), obj['mission_patch_profile']
        state = obj.get('drawbridge_state', 'initial')
        if state not in ('initial', 'applied'):
            raise ValueError('Review transition poses with an explicit matching source frame; endpoint artwork is not valid')
        records = [p for p in manifest.get('mission_patches', [])
                   if p['mission'] == mission and p['name'] == profile]
        if len(records) != 1 or state not in records[0].get('projection_sources', {}):
            raise ValueError(f'Missing declared mission projection source for {mission}/{profile}/{state}')
        sources.add(str((path.parent / records[0]['projection_sources'][state]).resolve(strict=True)))
    if len(sources) > 1:
        raise ValueError('Mixed mission endpoints require an explicitly composed matching reference state')
    return next(iter(sources), None)


def _reproject(config, report_dir):
    from reproject_map import restore_projection, reproject_layers, reproject_map
    # Layered projection owns its full receiver partition. Keep context meshes
    # render-visible, even though only the worker asset is selectable.
    restore_projection(config["map_name"])
    if config.get("projection_manifest"):
        # The projection annotator writes beside its manifest. Keep its changing
        # receiver report out of the immutable reference directory.
        source = Path(config["projection_manifest"])
        manifest = _absolute_manifest_images(json.loads(source.read_text()), source.parent)
        report_dir = Path(report_dir)
        report_dir.mkdir(parents=True, exist_ok=True)
        _json(report_dir / "layers.json", manifest)
        return reproject_layers(report_dir / "layers.json", report_dir,
                                ownership_nodes=config['part_ids'], preserve_authored=False,
                                exterior_source=_mission_review_source(config),
                                source_mask_manifest=config.get('source_mask_manifest'))
    report = reproject_map(config["map_name"], config["source_path"],
                           Path(report_dir) / "source.json",
                           elevation_deg=config["elevation_degrees"])
    from source_projection_bake import bake
    report['ownership'] = bake(config['map_name'], config['source_path'],
                               Path(report_dir) / 'ownership.json',
                               projection_label='exterior',
                               receiver_nodes=config['part_ids'],
                               elevation_deg=config['elevation_degrees'],
                               preserve_authored=False,
                               source_mask_manifest=config.get('source_mask_manifest'))
    return report


def _render(config, output, baseline=None):
    from refinement_review import render_review
    return render_review(output, scene_name=config["scene_name"],
                         collection_name=config["collection_name"], asset_id=config["asset_id"],
                         source_path=_mission_review_source(config) or config["source_path"], frame_manifest=baseline,
                         width=config["width"], height=config["height"],
                         elevation_degrees=config["elevation_degrees"],
                         context_padding=config["context_padding"],
                         projection_layers=_review_layers(config),
                         source_mask_manifest=config.get('source_mask_manifest'))


def prepare(workspace_dir, *, asset_id, scene_name, collection_name, source_path,
            grouping_manifest, inventory_path, review_path, projection_manifest=None, width=384, height=512,
            elevation_degrees=35.0, context_padding=24, source_mask_manifest=None):
    """Create a new workspace from the loaded scene; refuse an existing directory.

    Call from a disposable Blender process opened on the accepted full scene.
    The baseline includes context but only the chosen asset is editable in UI.
    """
    import bpy
    workspace = Path(workspace_dir).resolve()
    if workspace.exists():
        raise FileExistsError(workspace)
    if not collection_name.endswith(" Working"):
        raise ValueError("Projection expects '<map> Working' collection naming")
    grouping_path = Path(grouping_manifest).resolve()
    grouping = json.loads(grouping_path.read_text())
    review = json.loads(Path(review_path).read_text())
    if review.get("status") != "reviewed" or not review.get("reviewer"):
        raise ValueError("Finish and mark the grouping inventory reviewed before dispatch")
    if (review.get("catalog_sha256") != _sha(grouping_path)
            or review.get("inventory_sha256") != _sha(inventory_path)):
        raise ValueError("Grouping review does not match the catalog/inventory bytes")
    from refinement_inventory import validate_catalog
    validate_catalog(inventory_path, grouping_path)
    entry = next((a for a in grouping["groups"] if a["id"] == asset_id), None)
    if entry is None:
        raise ValueError(f"Asset absent from reviewed inventory: {asset_id}")
    config = dict(version=1, asset_id=asset_id, scene_name=scene_name,
                  collection_name=collection_name, map_name=collection_name[:-8],
                  width=width, height=height, elevation_degrees=elevation_degrees,
                  context_padding=context_padding, source_blend=str(Path(bpy.data.filepath).resolve()),
                  source_blend_sha256=_sha(bpy.data.filepath),
                  grouping_manifest_sha256=_sha(grouping_path))
    bpy.context.window.scene = bpy.data.scenes[scene_name]
    bpy.context.view_layer.update()
    targets, outside = _ownership(config)
    parts = sorted({o["source_node"] for o in targets})
    if sorted(f"building-{p['obstacle']:03d}" for p in entry["parts"]) != parts:
        raise ValueError("Current asset parts differ from the grouping review")
    workspace.mkdir(parents=True)
    reference = workspace / "reference"
    reference.mkdir()
    shutil.copy2(grouping_path, reference / "grouping.json")
    shutil.copy2(inventory_path, reference / "inventory.json")
    shutil.copy2(review_path, reference / "grouping-review.json")
    shutil.copy2(source_path, reference / "source.png")
    config["source_path"] = str(reference / "source.png")
    config["projection_manifest"] = None
    if source_mask_manifest:
        # Keep editable receiver assignments local; referenced inventories remain read-only.
        path = Path(source_mask_manifest).resolve(strict=True)
        masks = json.loads(path.read_text())
        masks['mask_inventory'] = str((path.parent / masks['mask_inventory']).resolve(strict=True))
        _json(workspace / 'source-masks.json', masks)
        config['source_mask_manifest'] = str(workspace / 'source-masks.json')
    if projection_manifest:
        path = Path(projection_manifest).resolve()
        layers = _copy_manifest_images(json.loads(path.read_text()), path.parent, reference)
        _json(reference / "layers.json", layers)
        config["projection_manifest"] = str(reference / "layers.json")
    for obj in _objects(config):
        obj.hide_select = obj not in targets
        obj.select_set(obj in targets)
    bpy.context.view_layer.objects.active = targets[0]
    bpy.ops.wm.save_as_mainfile(filepath=str(workspace / "baseline.blend"), copy=True)
    config["baseline_sha256"] = _sha(workspace / "baseline.blend")
    config["part_ids"] = parts
    config["outside_geometry"] = outside
    _reproject(config, workspace / "projection" / "input")
    _render(config, workspace / "input")
    config["input_files"] = _files(workspace / "input")
    config["reference_files"] = _files(reference)
    _json(workspace / "workspace.json", config)
    if projection_manifest:
        from asset_reference_views import prepare as prepare_asset_reference
        prepare_asset_reference(workspace)
    instructions = f'''# Refinement worker: {asset_id}

Own only this logical asset. Its stable source parts are {", ".join(parts)}.
Earlier refinements are hypotheses, not constraints. Remove or rebuild any owned
geometry that conflicts with original artwork or authored occlusion silhouettes.
Do not preserve a bad roof profile merely because a previous worker created it.
Preserve stable part identities, not inherited shapes; the baseline is your backup.
Edit `model.blend` in your own Blender process. All scene context remains in the
file for accurate occlusion; it is not selectable and is outside your scope.
Do not delete, transform, rename or edit other assets. Preserve source_node and
asset_group on every component; new component meshes must use one of the owned
source parts. Keep group-first, part-second editor selection intact.

`baseline.blend`, `reference/` and `input/` are immutable evidence. `context.png`
is an unmasked source-image crop, including background. `solid.png` and
`textured.png` show eight fixed views in a 4x2 sheet. Neutral gray means the
source view provides no reliable texture. Generated textures are never evidence.
Check actual silhouette against the context: terrain painted onto a roof means
geometry needs correction. Do not compensate for shape errors with generated art.

Feel free to generate zoomed-in detail views, additional camera angles, lower or
higher elevations, and section views whenever they help you understand or refine
the model. Do not limit your inspection to the eight standard views. Save these
extra renders in `inspection/` with descriptive names; include matching before
and after views when useful. The standard input/modified sheets remain the fixed
comparison, and extra views supplement them.

After each meaningful geometry pass run the modified command below. It reapplies
source projection and renders `modified/` with exactly the input camera framing
and file layout. Do not change the frozen framing to hide geometry differences.
Apply active modifiers before reprojection. Keep a reusable recipe and a short
`review.md` describing changes, checks, unresolved defects and inferred geometry.

```sh
blender --background "{workspace / 'model.blend'}" --python "{Path(__file__).resolve()}" -- modified "{workspace}"
```

Handoff `model.blend`, the recipe, `review.md`, and `modified/` only after validation
passes and you inspect context, solid and textured sheets. Do not publish the
whole copied scene: the coordinator imports only this asset into the main map.
Texture synthesis is a separate step after geometry review.
Start with asset-reference/ for focused original source crops and reviewed
asset-specific patches. The full reference/ directory is projection backing data;
its unrelated images are not assigned worker evidence.
For assets with interiors or changing outer patches, inspect reference/layers.json,
reference/covered.png, reference/revealed.png, each relevant patch PNG and alpha,
and the relevant reference/mission-patches/ state frames before refining. These
are original reference views, not synthesized textures. Record which states and
patch IDs were inspected in review.md. Render source-context crops and matching
solid/source-textured closeups for the exterior-covered and interior-revealed
states; hide only the covering geometry identified by that state. Keep interior
receivers, exterior receivers and their occluders separate during reprojection.
Do not mark an interior building reviewed from exterior eight-view sheets alone.
Save extra evidence in inspection/ without changing immutable input/reference.
Do not run GPT Sunburst texture generation yourself. The coordinator must show
the current solid and source-only textured views to the user and receive explicit
approval for this geometry revision before any Sunburst texture-fill request.
'''
    (workspace / "INSTRUCTIONS.md").write_text(instructions)
    bpy.ops.wm.save_as_mainfile(filepath=str(workspace / "model.blend"))
    return {"workspace": str(workspace), "asset_id": asset_id, "part_ids": parts,
            "input": str(workspace / "input"), "status": "prepared"}


def validate(workspace_dir):
    """Validate ownership of the loaded worker file and immutable input evidence."""
    import bpy
    workspace = Path(workspace_dir).resolve()
    config = json.loads((workspace / "workspace.json").read_text())
    bpy.context.window.scene = bpy.data.scenes[config["scene_name"]]
    bpy.context.view_layer.update()
    targets, outside = _ownership(config)
    errors = []
    if outside != config["outside_geometry"]:
        names = set(outside) | set(config["outside_geometry"])
        errors.append("Outside-asset geometry or identity changed: " + ", ".join(
            sorted(n for n in names if outside.get(n) != config["outside_geometry"].get(n))))
    if sorted({o["source_node"] for o in targets}) != config["part_ids"]:
        errors.append("Owned stable source parts changed")
    for name, expected in (("input", config["input_files"]),
                           ("reference", config["reference_files"])):
        if _files(workspace / name) != expected:
            errors.append(f"Immutable {name} files changed")
    if _sha(workspace / "baseline.blend") != config["baseline_sha256"]:
        errors.append("Immutable baseline.blend changed")
    if errors:
        raise ValueError("\n".join(errors))
    return {"status": "PASS", "asset_id": config["asset_id"],
            "part_ids": config["part_ids"], "meshes": len(targets),
            "protected_objects": len(outside)}


def modified(workspace_dir):
    """Reproject, render and validate; retain the last accepted packet on failure."""
    import bpy
    workspace = Path(workspace_dir).resolve()
    if Path(bpy.data.filepath).resolve() != workspace / "model.blend":
        raise ValueError("Open this workspace's model.blend before regenerating modified")
    report = validate(workspace)
    config = json.loads((workspace / "workspace.json").read_text())
    token = uuid.uuid4().hex[:12]
    stage = workspace / (".modified-" + token)
    try:
        _reproject(config, workspace / "projection" / token)
        _render(config, stage, workspace / "input" / "views.json")
        validate(workspace)
        if set(_files(stage)) != set(config["input_files"]):
            raise ValueError("Modified packet layout differs from immutable input")
        bpy.ops.wm.save_as_mainfile(filepath=str(workspace / "model.blend"))
        if (workspace / "modified").exists():
            history = workspace / "history"
            history.mkdir(exist_ok=True)
            (workspace / "modified").rename(history / token)
        stage.rename(workspace / "modified")
    except Exception:
        # Keep failed artifacts for diagnosis, never replace the previous packet.
        raise
    _json(workspace / "validation.json", report)
    return {**report, "modified": str(workspace / "modified")}


def dispatch(output_dir, *, source_blend, max_concurrency=4, **prepare_options):
    """Write one independently runnable preparation/refinement job per catalog asset.

    This plans jobs, not agent execution. The coordinator starts at most the
    declared concurrency and hands each agent only its matching workspace.
    """
    if max_concurrency < 1:
        raise ValueError("Concurrency must be positive")
    output = Path(output_dir).resolve()
    output.mkdir(parents=True, exist_ok=True)
    destination = output / "dispatch.json"
    if destination.exists():
        raise FileExistsError(destination)
    catalog = json.loads(Path(prepare_options["grouping_manifest"]).read_text())
    review = json.loads(Path(prepare_options["review_path"]).read_text())
    if (review.get("status") != "reviewed" or not review.get("reviewer")
            or review.get("catalog_sha256") != _sha(prepare_options["grouping_manifest"])
            or review.get("inventory_sha256") != _sha(prepare_options["inventory_path"])):
        raise ValueError("Dispatch requires matching reviewed grouping evidence")
    from refinement_inventory import validate_catalog
    validate_catalog(prepare_options["inventory_path"], prepare_options["grouping_manifest"])
    script = str(Path(__file__).resolve())
    source_blend = str(Path(source_blend).resolve(strict=True))
    jobs = []
    for asset in catalog["groups"]:
        asset_id = asset["id"]
        if Path(asset_id).name != asset_id or asset_id in (".", ".."):
            raise ValueError(f"Unsafe asset workspace name: {asset_id}")
        workspace = output / asset_id
        argv = ["blender", "--background", source_blend, "--python", script,
                "--", "prepare", str(workspace), "--asset-id", asset_id]
        for key, value in prepare_options.items():
            if value is not None:
                argv.extend(["--" + key.replace("_", "-"), str(value)])
        jobs.append({"asset_id": asset_id, "name": asset["name"],
                     "workspace": str(workspace),
                     "state": "existing_workspace" if workspace.exists() else "planned",
                     "prepare_argv": argv, "prepare_command": shlex.join(argv),
                     "agent_instructions": str(workspace / "INSTRUCTIONS.md"),
                     "handoff": ["model.blend", "modified/", "review.md", "recipe"]})
    result = {"version": 1, "max_concurrency": max_concurrency,
              "source_blend": source_blend, "source_blend_sha256": _sha(source_blend),
              "jobs": jobs, "review_scope": review.get("scope"),
              "unresolved": review.get("unresolved", []),
              "patch_asset_audit_status": review.get("patch_asset_audit_status"),
              "mission_patch_candidates": review.get("mission_patch_candidates", []),
              "known_missing_patch_assets": review.get("known_missing_patch_assets", []),
              "terrain": review.get("terrain", "Separate terrain worker required")}
    _json(destination, result)
    return {"dispatch": str(destination), "jobs": len(jobs), "max_concurrency": max_concurrency}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    for command in ("prepare", "dispatch"):
        p = sub.add_parser(command)
        p.add_argument("workspace")
        if command == "prepare":
            p.add_argument("--asset-id", required=True)
        else:
            p.add_argument("--source-blend", required=True)
            p.add_argument("--max-concurrency", type=int, default=4)
        for option in ("scene-name", "collection-name", "source-path", "grouping-manifest", "inventory-path", "review-path"):
            p.add_argument("--" + option, required=True)
        p.add_argument("--projection-manifest")
        p.add_argument("--width", type=int, default=384)
        p.add_argument("--height", type=int, default=512)
        p.add_argument("--elevation-degrees", type=float, default=35)
        p.add_argument("--context-padding", type=int, default=24)
    for command in ("modified", "validate"):
        sub.add_parser(command).add_argument("workspace")
    args = vars(parser.parse_args(argv))
    command, workspace = args.pop("command"), args.pop("workspace")
    result = globals()[command](workspace, **args)
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    main(sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else [])
