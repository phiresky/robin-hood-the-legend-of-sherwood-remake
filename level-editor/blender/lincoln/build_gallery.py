"""Collect verified Lincoln worker packets without granting approval.

Run with Python, after workers produce packets. Defaults target
work/lincoln-refinement/{grouping/catalog.json,round-1/assets,gallery}.
Workers provide candidate.json with version, asset_id, geometry_refined, status,
inspected_views, recipe, model_sha256, modified_views_sha256, changes, limitations.
Ready candidates also require review.md and all eight visually inspected views.
An unchanged candidate additionally requires geometry_reviewed=true and a
specific no_change_reason. --approvals accepts separate user decision records:
{"version":1,"approvals":[{"asset_id":"...","decision":"approved",
"exact_text":"...","model_sha256":"...","modified_views_sha256":"..."}]}.
Only decisions bound to the current model and packet can hide approved cards.
"""
import argparse
from collections import Counter
import hashlib
import importlib.util
import json
from pathlib import Path
import sys


def sha(path):
    digest = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def read(path):
    return json.loads(Path(path).read_text())


def write(path, data):
    Path(path).write_text(json.dumps(data, indent=2) + "\n")


def file_hashes(directory):
    return {str(path.relative_to(directory)): sha(path)
            for path in sorted(directory.rglob("*")) if path.is_file()}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def source_coverage_audit_matches(audit, evidence):
    """A completeness review is valid only for the inspected model and views."""
    return (audit.get('status') == 'PASS'
        and audit.get('model_sha256') == evidence['model_sha256']
        and audit.get('modified_views_sha256') == evidence['packet_hashes']['modified']['views.json']
        and audit.get('inspected_views') == list(range(8)))


def projection_available_nodes(before_layers, after_layers, owned_nodes):
    """Allow owned meshes to enter/leave visibility while protecting neighbors."""
    def nodes(layers):
        return {node for row in layers for key in ("receiver_nodes", "occluder_nodes")
                for node in row[key]}
    before, after = nodes(before_layers), nodes(after_layers)
    require((before ^ after) <= set(owned_nodes),
            "Projection visibility changed outside the reviewed asset")
    return after


def projection_records(config, manifest, available):
    """Rebuild the frozen helper's static review partition without loading Blender."""
    from interior_layers import (projection_receivers, projection_occluders,
        projection_component_exclusions, projection_receiver_components, projection_occluder_additions)
    from projection_regions import region_record
    directory = Path(config["projection_manifest"]).parent
    interiors = projection_receivers(manifest)
    components = projection_receiver_components(manifest)
    exclusions = projection_component_exclusions(manifest)
    additions = projection_occluder_additions(manifest)
    exterior = (available - {node for nodes in interiors.values() for node in nodes}) | {
        row["source_node"] for row in components.get("exterior", [])}
    blockers = exterior | (set(additions.get("exterior", [])) & available)
    occluders = projection_occluders(manifest, available)
    partitions = [("exterior", "exterior", sorted(exterior), sorted(blockers))]
    partitions += [("interior", "interior-" + patch, sorted(nodes), occluders[patch])
                   for patch, nodes in interiors.items()]
    covered = (directory / manifest["sources"]["exterior"]).resolve()
    result = []
    for source, label, nodes, obscurers in partitions:
        path = (directory / manifest["sources"][source]).resolve()
        row = {"source_path": str(path), "source_sha256": sha(path),
               "receiver_nodes": nodes, "occluder_nodes": obscurers}
        if label in components:
            row["receiver_components"] = components[label]
        patch = label.removeprefix("interior-")
        if source == "interior":
            if patch in exclusions:
                row.update(exclude_occluder_components=exclusions[patch], projection_label=label)
            row["projection_region"] = region_record(manifest, directory, patch, path, covered,
                blockers | set(nodes), covered_components=exclusions.get(patch))
        if config.get("source_mask_manifest"):
            row["projection_label"] = label
        result.append(row)
    return result


def frozen_mask_origins(workspace, config):
    """Resolve historical input assignment paths only through proven clone evidence."""
    if not config.get('mask_reference'):
        return {}
    frozen = Path(config['mask_reference']) / 'assignments.json'
    origins = {Path(config['source_mask_manifest']).resolve(): frozen}
    clone = config.get('cloned_mask_origin')
    if clone:
        original = Path(clone['workspace']).resolve(strict=True)
        require(sha(original / 'workspace.json') == clone['workspace_sha256'],
                'Cloned mask origin workspace configuration changed')
        old = read(original / 'workspace.json')
        require(old['asset_id'] == config['asset_id'], 'Cloned mask origin asset differs')
        require(old['baseline_sha256'] == config['baseline_sha256'] ==
                sha(original / 'baseline.blend'), 'Cloned mask origin baseline differs')
        require(file_hashes(original / 'input') == config['input_files'] ==
                file_hashes(Path(workspace) / 'input'), 'Cloned immutable input differs')
        require(Path(old['source_mask_manifest']).resolve() == Path(clone['manifest']).resolve(),
                'Cloned mask origin path differs from original workspace')
        require(sha(Path(old['mask_reference']) / 'assignments.json') == sha(frozen),
                'Cloned frozen mask assignments differ')
        origins[Path(clone['manifest']).resolve()] = frozen
    return origins


def inspect(workspace, asset):
    config = read(workspace / "workspace.json")
    require(config["asset_id"] == asset["id"], "Workspace asset ID mismatch")
    from catalog_schema import source_for_part
    expected_parts = (['ground'] if asset.get('role') == 'terrain' else
                      sorted(source_for_part(p) for p in asset["parts"]))
    require(config["part_ids"] == expected_parts,
            "Workspace parts differ from current catalog")
    selectors = sorted((source_for_part(part), component)
                       for part in asset.get("parts", []) for component in part.get("components", []))
    if selectors:
        from workspace_components import validated_scope
        scope = validated_scope(config)
        require(scope is not None, "Component catalog requires a component-aware workspace")
        actual = sorted((row["source_node"], row["projection_component"])
                        for row in scope["owned_components"])
        require(actual == selectors, "Workspace component selectors differ from current catalog")
    validation = read(workspace / "validation.json")
    require(validation.get("status") == "PASS" and validation.get("asset_id") == asset["id"],
            "Missing successful asset validation")
    require(validation.get("part_ids") == config["part_ids"], "Validation source parts differ")
    if selectors:
        require(validation.get("component_ownership") == config["component_ownership"],
                "Validation component ownership differs")
    require(sha(workspace / "baseline.blend") == config["baseline_sha256"], "Frozen baseline changed")
    for folder, key in (("input", "input_files"), ("reference", "reference_files")):
        require(file_hashes(workspace / folder) == config[key], f"Immutable {folder} evidence changed")
    from refinement_workspace import _validated_masks, _validated_projection
    mask_revision = None
    if config.get("mask_reference"):
        mask_revision = _validated_masks(config)
    mask_origins = frozen_mask_origins(workspace, config)
    reviewed_projection = _validated_projection(config) if config.get("projection_manifest") else None
    required = {"solid.png", "textured.png", "context.png", "views.json"}
    required.update(f"views/view-{i}-{kind}.png" for i in range(8)
                    for kind in ("solid", "known", "textured"))
    packets, hashes = {}, {}
    for name in ("input", "modified"):
        folder = workspace / name
        hashes[name] = file_hashes(folder)
        require(required <= hashes[name].keys(), f"Incomplete {name} eight-view packet")
        packet = read(folder / "views.json")
        if selectors:
            require(packet.get("component_ownership") == config["component_ownership"],
                    f"{name} packet component ownership differs")
        require(packet.get("asset_id") == asset["id"] and packet.get("version") == 1,
                f"Invalid {name} framing manifest")
        require(packet.get("layout") == {"columns": 4, "rows": 2}, "Expected fixed 4x2 layout")
        require([view["index"] for view in packet["views"]] == list(range(8)), "Expected all eight views")
        for view in packet["views"]:
            require(view["ownership_sha256"] == hashes[name][f"views/view-{view['index']}-known.png"],
                    f"{name} ownership bitmap hash mismatch")
        for path, digest in (packet.get("source_mask_evidence") or {}).items():
            check_path = Path(path)
            if name == "input" and mask_revision:
                check_path = mask_origins.get(check_path.resolve(), check_path)
            require(sha(check_path) == digest, f"Mask evidence changed: {path}")
        if name == "modified" and config.get("source_mask_manifest"):
            from occlusion_constraints import evidence_record
            require(packet.get("source_mask_evidence") == evidence_record(config["source_mask_manifest"]),
                    "Modified packet does not bind current working mask evidence")
        require(sha(packet["source_image"]) == packet["source_sha256"], "Source artwork changed")
        packets[name] = packet
    require(set(hashes["input"]) == set(hashes["modified"]), "Input/modified file layout differs")
    for key in ("tile_size", "elevation_degrees", "context_crop", "source_sha256", "lighting"):
        require(packets["input"].get(key) == packets["modified"].get(key), f"Frozen {key} differs")
    if not mask_revision:
        require(packets["input"].get("source_mask_evidence") == packets["modified"].get("source_mask_evidence"),
                "Unmigrated mask evidence differs from frozen input")
    if reviewed_projection is not None:
        before_layers = packets["input"]["projection_layers"]
        after_layers = packets["modified"]["projection_layers"]
        available = projection_available_nodes(before_layers, after_layers, config["part_ids"])
        expected_layers = projection_records(config, reviewed_projection, available)
        require(after_layers == expected_layers,
                "Modified projection layers do not match validated own-asset projection reviews")
    else:
        require(packets["input"]["projection_layers"] == packets["modified"]["projection_layers"],
                "Projection layers changed without reviewed working manifest")
    for before, after in zip(packets["input"]["views"], packets["modified"]["views"]):
        for key in ("camera_matrix_world", "camera_location", "camera_rotation_euler", "ortho_scale"):
            require(before[key] == after[key], f"Frozen camera {before['index']} {key} differs")
    changed = any(hashes["input"][f"views/view-{i}-solid.png"] !=
                  hashes["modified"][f"views/view-{i}-solid.png"] for i in range(8))
    model_hash = sha(workspace / "model.blend")
    candidate_path = workspace / "candidate.json"
    worker = read(candidate_path) if candidate_path.exists() else {}
    blockers = []
    bound = (worker.get("model_sha256") == model_hash and
             worker.get("modified_views_sha256") == hashes["modified"]["views.json"])
    if worker:
        require(worker.get("version") == 1 and worker.get("asset_id") == asset["id"],
                "Invalid candidate identity/version")
        require(isinstance(worker.get("limitations"), list) and isinstance(worker.get("changes"), list),
                "Candidate requires changes and limitations lists")
    if not bound:
        blockers.append("Worker report is absent or does not bind the current model and packet hashes.")
    recipe = Path(worker["recipe"]) if worker.get("recipe") else None
    if recipe and not recipe.is_absolute():
        recipe = workspace / recipe
    if recipe and recipe.is_file():
        recipe_evidence = {"path": str(recipe.resolve()), "sha256": sha(recipe)}
    else:
        recipe_evidence = None
        blockers.append("Reproducible geometry recipe missing.")
    review = workspace / "review.md"
    if not review.is_file() or not review.read_text().strip():
        blockers.append("Worker review and limitations missing.")
    if worker.get("inspected_views") != list(range(8)):
        blockers.append("Worker has not recorded inspection of all eight modified views.")
    refined = worker.get("geometry_refined") is True and changed and bound
    no_change_reason = worker.get("no_change_reason")
    reviewed_unchanged = (worker.get("geometry_reviewed") is True
                          and worker.get("geometry_refined") is False and not changed and bound
                          and isinstance(no_change_reason, str) and bool(no_change_reason.strip()))
    if refined and not worker.get("changes"):
        blockers.append("Worker has not described the geometry changes.")
    if not refined and not reviewed_unchanged:
        blockers.append("No completed geometry refinement or explicit unchanged-geometry audit is established.")
    source_pixels = sum(view['counts']['source'] for view in packets['modified']['views'])
    unknown_pixels = sum(view['counts']['unknown'] for view in packets['modified']['views'])
    if source_pixels == 0:
        blockers.append("No source texture is present in any reviewed view. Source ownership or state evidence must be resolved before this asset is ready.")
    status = "refinement-in-progress"
    if worker.get("status") == "fix-needed" or source_pixels == 0:
        status = "fix-needed"
    elif worker.get("status") == "ready-for-user" and not blockers:
        status = "ready-for-user"
    limitations = list(packets["modified"].get("limitations", [])) + worker.get("limitations", []) + blockers
    if reviewed_unchanged:
        limitations.append("Reviewed without geometry changes: " + no_change_reason.strip())
    if not config.get("projection_manifest"):
        limitations.append("Static exterior packet only; revealed and animated state ownership is not validated.")
    unconstrained = sorted({row["source_node"] for row in
                            packets["modified"].get("source_constraint_status", [])
                            if not row.get("constrained")})
    if unconstrained:
        limitations.append("No reviewed native mask constrains: " + ", ".join(unconstrained) + ".")
    mask_assignments = []
    if config.get("source_mask_manifest"):
        masks = read(config["source_mask_manifest"])
        for projection, definition in masks["projections"].items():
            for assignment in definition["assignments"]:
                if (assignment.get("source_node") in config["part_ids"]
                        or assignment.get("asset_group") == asset["id"]):
                    mask_assignments.append({"projection": projection, **assignment})
        active_labels = {row.get('projection_label') for row in packets['modified'].get('projection_layers', [])}
        unknown_nodes = sorted({row.get("source_node", asset["id"]) for row in mask_assignments
                                if row.get("constraint_kind") == "unknown-no-approved-source"
                                and row['projection'] in active_labels})
        if unknown_nodes:
            limitations.append("Explicit neutral-only source constraints, with no accepted native ownership: "
                               + ", ".join(unknown_nodes) + ". Black rejection masks are not native silhouette evidence.")
    preparation = workspace / "preparation.json"
    if preparation.exists() and read(preparation).get("scope"):
        limitations.append(read(preparation)["scope"])
    evidence = {"version": 1, "asset_id": asset["id"], "status": status,
                "geometry_refined": refined, "geometry_reviewed": refined or reviewed_unchanged,
                "review_outcome": "refined" if refined else "reviewed-no-change" if reviewed_unchanged else "baseline-only",
                "solid_views_changed": changed,
                "source_coverage": {"source_pixels": source_pixels, "unknown_pixels": unknown_pixels,
                                    "fraction": source_pixels / max(1, source_pixels + unknown_pixels)},
                "model_sha256": model_hash, "baseline_sha256": config["baseline_sha256"],
                "packet_hashes": hashes, "source_sha256": packets["modified"]["source_sha256"],
                "source_mask_evidence": packets["modified"].get("source_mask_evidence"),
                "source_constraints": packets["modified"].get("source_constraint_status"),
                "mask_assignments": mask_assignments,
                "reviewed_mutable_evidence": {
                    "mask_revision": mask_revision,
                    "projection_manifest": config.get("projection_manifest"),
                    "projection_manifest_sha256": sha(config["projection_manifest"]) if reviewed_projection is not None else None,
                    "projection_reviews": reviewed_projection.get("projection_reviews") if reviewed_projection is not None else None,
                    "guard": "Frozen helper validates immutable authority and rejects foreign assignment changes"},
                "recipe": recipe_evidence, "worker_report": worker,
                "candidate_sha256": sha(candidate_path) if worker else None,
                "validation": validation, "validation_sha256": sha(workspace / "validation.json"),
                "limitations": limitations}
    return status, limitations, evidence, review


def supplemental_packet(directory, asset_id, framing, *, mask_origin=None):
    """Check state sheets against the same fixed cameras and their source bytes."""
    directory = Path(directory).resolve(strict=True)
    packet = read(directory / 'views.json')
    require(packet.get('asset_id') == asset_id, 'Supplemental packet asset differs')
    require(packet.get('layout') == {'columns': 4, 'rows': 2}, 'Supplemental layout differs')
    require(len(packet.get('views', [])) == 8, 'Supplemental state requires eight views')
    for key in ('tile_size', 'context_crop', 'elevation_degrees', 'lighting'):
        require(packet.get(key) == framing.get(key), 'Supplemental framing differs: ' + key)
    hashes = file_hashes(directory)
    for name in ('solid.png', 'textured.png', 'context.png', 'views.json'):
        require(name in hashes, 'Missing supplemental sheet: ' + name)
    for before, after in zip(framing['views'], packet['views']):
        for key in ('index', 'camera_matrix_world', 'camera_location', 'camera_rotation_euler', 'ortho_scale'):
            require(before[key] == after[key], 'Supplemental camera differs: ' + key)
        for kind in ('solid', 'textured', 'known'):
            require(f"views/view-{after['index']}-{kind}.png" in hashes, 'Incomplete state view')
        require(after['ownership_sha256'] == hashes[f"views/view-{after['index']}-known.png"],
                'Supplemental known-pixel bitmap differs')
    require(sha(packet['source_image']) == packet['source_sha256'], 'Supplemental source changed')
    for path, digest in (packet.get('source_mask_evidence') or {}).items():
        actual = Path(path)
        if mask_origin:
            actual = mask_origin.get(actual.resolve(), actual)
        require(sha(actual) == digest, 'Supplemental mask evidence changed: ' + path)
    return {'directory': str(directory), 'hashes': hashes, 'source_sha256': packet['source_sha256']}


def main(argv=None):
    root = Path(__file__).resolve().parents[2] / "work/lincoln-refinement"
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--catalog", type=Path, default=root / "grouping/catalog.json")
    parser.add_argument("--assets", type=Path, default=root / "round-1/assets")
    parser.add_argument("--output", type=Path, default=root / "gallery")
    parser.add_argument('--workspace-map', type=Path, default=root / 'workspace-overrides.json',
                        help='Explicit newer revision paths; frozen older workspaces remain in place')
    parser.add_argument("--approvals", type=Path, help="Separate explicit user decisions bound to model/packet hashes")
    parser.add_argument("--tooling-dir", type=Path,
                        help="Frozen helper snapshot; defaults to tooling/current.json")
    args = parser.parse_args(argv)
    if args.approvals is None and (root / 'approvals.json').exists():
        args.approvals = root / 'approvals.json'
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    from freeze_tooling import select_tooling
    tooling = select_tooling(args.tooling_dir)
    catalog = read(args.catalog)
    require(catalog.get("map") == "lincoln", "Expected Lincoln catalog")
    if catalog.get('terrain'):
        catalog = {**catalog, 'groups': [*catalog['groups'], {
            'id': catalog['terrain']['id'], 'name': catalog['terrain']['name'],
            'role': 'terrain', 'parts': [],
        }]}
    ids = [asset["id"] for asset in catalog["groups"]]
    require(len(ids) == len(set(ids)), "Duplicate catalog asset IDs")
    historical_ids = set()
    for prior_path in args.catalog.parent.glob('catalog-v*.json'):
        prior = read(prior_path)
        if (prior.get('map') == catalog['map'] and
                prior.get('revision', 0) < catalog.get('revision', 0)):
            historical_ids.update(group['id'] for group in prior['groups'])
    retired_ids = historical_ids - set(ids)
    overrides = {}
    if args.workspace_map.exists():
        override_record = read(args.workspace_map)
        require(override_record.get('version') == 1, 'Unknown workspace map version')
        overrides = override_record['assets']
        require(set(overrides) <= set(ids) | retired_ids, 'Workspace overrides contain unknown assets')
    approvals = {}
    if args.approvals:
        records = read(args.approvals)
        require(records.get("version") == 1, "Unsupported approval record version")
        seen_decisions = set()
        for record in records["approvals"]:
            identifier = record["asset_id"]
            require(identifier in set(ids) | retired_ids and identifier not in seen_decisions,
                    "Unknown or duplicate approval asset ID")
            seen_decisions.add(identifier)
            if identifier in retired_ids:
                continue  # The persistent log retains the retired revision; never transfer its approval.
            require(record.get("decision") in ("approved", "rejected", "revision-requested"), "Invalid user decision")
            require(isinstance(record.get("exact_text"), str) and record["exact_text"].strip(), "Exact user decision text missing")
            for key in ("model_sha256", "modified_views_sha256"):
                require(isinstance(record.get(key), str) and len(record[key]) == 64,
                        "User decision must bind exact model and packet hashes")
            approvals[identifier] = record
    output = args.output.resolve()
    evidence_dir = output.parent / (output.name + "-packet-evidence")
    evidence_dir.mkdir(parents=True, exist_ok=True)
    items, progress = [], []
    for asset in catalog["groups"]:
        require(Path(asset["id"]).name == asset["id"] and asset["id"] not in (".", ".."), "Unsafe asset ID")
        workspace = (Path(overrides[asset['id']]).resolve() if asset['id'] in overrides else
                     args.assets.resolve() / asset["id"])
        row = {"id": asset["id"], "name": asset["name"], "workspace": str(workspace)}
        if not workspace.exists():
            progress.append({**row, "status": "missing"})
            continue
        try:
            status, limitations, evidence, review = inspect(workspace, asset)
            worker = evidence['worker_report']
            state_records = {}
            def local(path):
                path = Path(path)
                return path if path.is_absolute() else workspace / path
            framing = read(workspace / 'input/views.json')
            if worker.get('covered_solid'):
                folder = local(worker['covered_solid']).parent
                state_records['covered'] = supplemental_packet(folder, asset['id'], framing)
                require(local(worker['covered_textured']).resolve() == folder.resolve() / 'textured.png'
                        and local(worker['covered_context']).resolve() == folder.resolve() / 'context.png',
                        'Covered sheets must come from one validated packet')
            for state in worker.get('animation_states', []):
                identifier = state['id']
                require(isinstance(identifier, str) and identifier and
                        Path(identifier).name == identifier and identifier not in ('.', '..'),
                        'Invalid animation state ID')
                key = 'animation-' + identifier
                require(key not in state_records, 'Duplicate animation state ID')
                state_records[key] = supplemental_packet(local(state['directory']), asset['id'], framing)
            if worker.get('revealed_solid'):
                state_dir = local(worker['revealed_solid']).parent
                state_records['revealed'] = supplemental_packet(state_dir, asset['id'], framing)
                require(local(worker['revealed_textured']).resolve() == state_dir.resolve() / 'textured.png'
                        and local(worker['revealed_context']).resolve() == state_dir.resolve() / 'context.png',
                        'Revealed sheets must come from one validated packet')
                baseline_dir = local(worker.get('revealed_input', 'revealed/input'))
                config = read(workspace / 'workspace.json')
                origin = frozen_mask_origins(workspace, config)
                state_records['revealed_input'] = supplemental_packet(
                    baseline_dir, asset['id'], framing, mask_origin=origin)
            evidence['state_packets'] = state_records
            evidence['state_bundle_sha256'] = (hashlib.sha256(json.dumps(
                state_records, sort_keys=True).encode()).hexdigest() if state_records else None)
            # Lincoln packets are rendered with the calibrated map lighting at preparation;
            # there is no supplemental relit solid sheet.
            map_lighting = read(root / 'lighting-calibration/map-lighting.json')['lighting']
            packet_lighting = read(workspace / 'modified/views.json').get('lighting') or {}
            # Packets store float32-rounded values; compare within float32 precision.
            require(set(packet_lighting) == set(map_lighting) and all(
                        abs(a - b) < 1e-6 for key in map_lighting
                        for a, b in zip(*(([v] if not isinstance(v, list) else v)
                                          for v in (packet_lighting[key], map_lighting[key])))),
                    'Packet lighting differs from the Lincoln calibration')
            lighting_review = None
            evidence['lighting_review'] = None
            evidence['lighting_review_sha256'] = None
        except (OSError, ValueError, KeyError, TypeError) as error:
            progress.append({**row, "status": "validation-pending", "reason": str(error)})
            continue
        evidence_path = evidence_dir / (asset["id"] + ".json")
        approval = approvals.get(asset["id"])
        approval_current = bool(approval and approval["model_sha256"] == evidence["model_sha256"]
                                and approval["modified_views_sha256"] == evidence["packet_hashes"]["modified"]["views.json"]
                                and (not evidence['state_bundle_sha256'] or
                                     approval.get('state_bundle_sha256') == evidence['state_bundle_sha256'])
                                and approval.get('lighting_review_sha256') == evidence['lighting_review_sha256'])
        evidence["user_decision"] = approval
        evidence["user_decision_matches_revision"] = approval_current
        user_approval = "pending"
        if approval and approval.get('projection_review') == 'revision-requested':
            user_approval = 'geometry-approved; projection-pending'
            correction_path = evidence['worker_report'].get('projection_correction')
            correction = None
            if correction_path:
                correction_path = Path(correction_path)
                if not correction_path.is_absolute():
                    correction_path = workspace / correction_path
                correction = read(correction_path)
                require(correction.get('status') == 'PASS'
                        and correction['approved_model_sha256'] == approval['model_sha256']
                        and correction['model_sha256'] == evidence['model_sha256']
                        and correction['geometry_before_sha256'] == correction['geometry_after_sha256']
                        and len(correction['geometry_before_sha256']) == 64
                        and correction.get('inspected_views') == list(range(8)),
                        'Projection correction lacks proof that approved geometry was preserved')
                evidence['projection_correction'] = {**correction, 'report_sha256': sha(correction_path)}
            if correction is None:
                status = 'fix-needed'
            limitations.append('Geometry explicitly approved. ' + (
                'Projection corrected without geometry changes; projection review remains pending.' if correction else
                'The reported projection clipping is being corrected.'))
        elif approval_current:
            user_approval = approval["decision"]
            if user_approval == "approved":
                status = "approved"
            elif user_approval == "rejected":
                status = "rejected"
            else:
                status = "fix-needed"
        elif approval:
            limitations.append("Previous user decision applies to a different model/packet revision; current review is pending.")
        # Procedure section 6: every ready candidate needs a hash-bound source coverage audit.
        if status == 'ready-for-user':
            audit_path = workspace / 'source-coverage-audit.json'
            audit = read(audit_path) if audit_path.exists() else {}
            audit_current = source_coverage_audit_matches(audit, evidence)
            if not audit_current:
                status = 'audit-pending'
                limitations.append('Source-visible texture completeness audit pending; not ready for user review.')
            else:
                evidence['source_coverage_audit'] = {'path': str(audit_path), 'sha256': sha(audit_path)}
        evidence["status"] = status
        limitations.append('Solid views use Lincoln artwork-calibrated sunlight (see lighting-calibration/map-lighting.json). Source-textured evidence is preserved.')
        write(evidence_path, evidence)
        item = {**row, "status": status, "user_approval": user_approval,
                "geometry_refined": evidence["geometry_refined"],
                "geometry_reviewed": evidence["geometry_reviewed"],
                "review_outcome": evidence["review_outcome"], "user_decision": approval,
                "notes": limitations,
                "model": str(workspace / "model.blend"),
                "solid": str(workspace / "modified/solid.png"),
                "textured": str(workspace / "modified/textured.png"),
                "context": str(workspace / "modified/context.png"),
                "validation": str(workspace / "validation.json"), "ownership": str(evidence_path)}
        if review.is_file():
            item["review"] = str(review)
        for key in ('source_comparison', 'source_comparison_secondary', 'source_trace', 'projection_errors'):
            value = evidence['worker_report'].get(key)
            if value:
                image = Path(value)
                if not image.is_absolute():
                    image = workspace / image
                image = image.resolve(strict=True)
                require(image.is_relative_to(workspace.resolve()), 'Review image must belong to its workspace')
                item[key] = str(image)
        if evidence['state_packets'].get('covered'):
            folder = Path(evidence['state_packets']['covered']['directory'])
            item.update(solid=str(folder/'solid.png'), textured=str(folder/'textured.png'),
                        context=str(folder/'context.png'))
        if evidence['state_packets'].get('revealed'):
            folder = Path(evidence['state_packets']['revealed']['directory'])
            item.update(revealed_solid=str(folder/'solid.png'), revealed_textured=str(folder/'textured.png'),
                        revealed_context=str(folder/'context.png'))
        items.append(item)
        for key, packet in evidence['state_packets'].items():
            if not key.startswith('animation-'):
                continue
            folder = Path(packet['directory'])
            declared = next((s for s in evidence['worker_report'].get('animation_states', [])
                             if 'animation-' + s['id'] == key), {})
            state_item = {'id': key,
                          'name': declared.get('name') or key.removeprefix('animation-').replace('-', ' ').capitalize(),
                          'description': declared.get('description') or (
                              'The winch/lever sprite changes between these states; tower masonry is unchanged.'
                              if key.startswith('animation-mechanism-') else
                              'An animated state of this same asset. Compare the moving part with the other state.'),
                          'solid': str(folder / 'solid.png'),
                          'textured': str(folder / 'textured.png'),
                          'context': str(folder / 'context.png')}
            item.setdefault('animation_reviews', []).append(state_item)
        progress.append({**row, "status": status, "geometry_refined": evidence["geometry_refined"],
                         "geometry_reviewed": evidence["geometry_reviewed"],
                         "review_outcome": evidence["review_outcome"], "user_approval": user_approval})
    manifest = output.parent / (output.name + "-candidates.json")
    progress_path = output.parent / (output.name + "-progress.json")
    counts = dict(Counter(item["status"] for item in progress))
    shared_gallery = Path(__file__).resolve().parents[2] / 'refinement/blender/build_review_gallery.py'
    gallery_tooling = {'path': str(shared_gallery), 'sha256': sha(shared_gallery)}
    write(manifest, {"version": 1, "map": "Lincoln", "items": items, "tooling": tooling,
                     'gallery_tooling': gallery_tooling, 'total_groups': len(ids) - 1,
                     'supplemental_count': 1, 'status_counts': counts,
                     'without_packets': [row for row in progress if row['status'] in ('missing', 'validation-pending')]})
    write(progress_path, {"version": 1, "map": "Lincoln", "catalog": str(args.catalog.resolve()),
                         "catalog_sha256": sha(args.catalog), "total_assets": len(ids),
                         "gallery_assets": sum('parent_asset_id' not in item for item in items),
                         "gallery_cards": len(items), "counts": counts, "assets": progress,
                         "approval_records": str(args.approvals.resolve()) if args.approvals else None,
                         "approval_records_sha256": sha(args.approvals) if args.approvals else None,
                         "tooling": tooling,
                         "complete": len(progress) > 0 and all(row["status"] == "approved" for row in progress)})
    spec = importlib.util.spec_from_file_location('_shared_review_gallery', shared_gallery)
    gallery_module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(gallery_module)
    gallery_module.build(manifest, output, pending_only=True, map_name="Lincoln")
    print(json.dumps({"progress": str(progress_path), "counts": counts,
                      "gallery": str(output / "index.html")}))


if __name__ == "__main__":
    main()
