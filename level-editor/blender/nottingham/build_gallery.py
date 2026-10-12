"""Collect verified Nottingham worker packets without granting approval.

Run with Python, after workers produce packets. Defaults target
work/needs-review/nottingham-refinement/{grouping/catalog-v15.json,round-1/assets,gallery}.
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


def neutral_source_constraint_notes(assignments, layers, part_ids, asset_id):
    """Describe effective layer rules without treating node fallbacks as mesh facts.

    Component rules override node rules, which override group rules. A packet
    without component inventory cannot establish whether an unselected default
    has an actual receiver; retain that uncertainty explicitly in its note.
    """
    definite, conditional = set(), set()
    for layer in layers:
        label = layer.get("projection_label", "exterior")
        rows = [row for row in assignments if row["projection"] == label]
        selectors = {}
        for selector in layer.get("receiver_components", []):
            selectors.setdefault(selector["source_node"], set()).update(selector["projection_components"])
        for node in set(layer.get("receiver_nodes", [])) & set(part_ids):
            node_rows = [row for row in rows if row.get("source_node") == node]
            components = {row["projection_component"]: row for row in node_rows
                          if row.get("projection_component")}
            default = next((row for row in node_rows if not row.get("projection_component")), None)
            if default is None:
                default = next((row for row in rows if row.get("asset_group") == asset_id
                                and not row.get("source_node")), None)
            selected = selectors.get(node)
            effective = [(component, components.get(component, default)) for component in selected] if selected else list(components.items()) + [(None, default)]
            for component, row in effective:
                if not row or row.get("constraint_kind") != "unknown-no-approved-source":
                    continue
                name = f"{node}/{component}" if component else node
                description = f"{name} [{label}]"
                if component is None and components:
                    conditional.add(description)
                else:
                    definite.add(description)
    notes = []
    if definite:
        notes.append("Neutral-only source rules for active receivers: " + ", ".join(sorted(definite))
                     + ". These rules accept no native source pixels; source-coverage audits must explain any gray geometry.")
    if conditional:
        notes.append("Neutral node defaults for components without a more-specific assignment: "
                     + ", ".join(sorted(conditional))
                     + ". Component-specific rules take precedence; these defaults alone do not show missing textures on the reviewed meshes.")
    return notes


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


def state_bundle_hash(state_records, complete_contracts=None):
    """Preserve existing revisions; explicit states also bind their material contract."""
    if not state_records and not complete_contracts:
        return None
    payload = ({"packets": state_records, "complete_state_contracts": complete_contracts}
               if complete_contracts else state_records)
    return hashlib.sha256(json.dumps(payload, sort_keys=True).encode()).hexdigest()


ORIGINAL_HALL_STATE_MANIFEST = Path(__file__).resolve().parents[3] / 'level-editor/work/needs-review/nottingham-refinement/round-42/assets/nottingham-castle-main-hall/inspection/state-models/manifest.json'


def complete_state_records(workspace, config, manifest, available, framing):
    """Validate complete appearances against frozen source and visibility authority.

    This opt-in contract supplements, rather than relaxes, the static partition.
    It currently describes the hall's single reviewed reveal patch.
    """
    require(config["asset_id"] == "nottingham-castle-main-hall",
            "Complete state layers require a supported asset contract")
    patch = "patch-008"
    visibility = manifest["projection_reviews"][patch]["render_visibility"]
    require(visibility.get("reviewed") is True, "State visibility is not reviewed")
    layers = read(workspace / "projection-state-layers.json")
    require(set(layers) == {"covered", "revealed"}, "Expected exactly two projection states")
    materials = read(workspace / "material-states.json")
    bindings = read(workspace / "inspection/state-models/manifest.json")
    model_hash = sha(workspace / "model.blend")
    require(materials.get("version") == bindings.get("version") == 1,
            "Unsupported complete-state contract version")
    require(materials.get("model_sha256") == bindings.get("primary_model_sha256") == model_hash,
            "State materials do not bind the primary model")
    require(bindings.get("material_states_sha256") == sha(workspace / "material-states.json"),
            "Saved states do not bind material assignments")
    records = materials["records"]
    names = {row["object"] for row in records}
    require(records and len(names) == len(records), "Duplicate or absent material receivers")
    require(all(row["source_node"] in config["part_ids"] for row in records),
            "State material receiver outside owned asset")
    for row in records:
        covered, revealed = (row[state + "_face_materials"] for state in ("covered", "revealed"))
        require(set(covered) == set(revealed), "State face inventories differ")
        for faces in (covered, revealed):
            require(set(faces) == {str(i) for i in range(len(faces))}, "Incomplete state face inventory")
            require(all(isinstance(v.get("slot"), int) and v["slot"] >= 0
                        and isinstance(v.get("material"), str) and v["material"] for v in faces.values()),
                    "Invalid state material assignment")
    supplement_path = workspace / "contact-state-supplement.json"
    supplement = read(supplement_path) if supplement_path.exists() else None
    if supplement:
        require(supplement.get("version") == 1 and supplement.get("model_sha256") == model_hash,
                "Contact state supplement has stale model")
        require((supplement.get("source_node"), supplement.get("component"), supplement.get("patch_id")) ==
                ("building-505", "castle-hall-northwest-contact", patch), "Unsupported contact state receiver")
        cap_binding = supplement["cap_proof"]
        cap_path = Path(cap_binding["path"])
        require(cap_path.resolve() == (workspace / "inspection/contact-material-provenance.json").resolve()
                and sha(cap_path) == cap_binding["sha256"], "Contact atlas proof changed")
        cap = read(cap_path)
        require(cap.get("status") == "PASS" and cap.get("model_sha256") == model_hash
                and cap.get("all_source_pixels") == cap.get("transferred_pixels") == 407,
                "Contact source atlas is not completely verified")
        require(len([r for r in records if r["source_node"] == "building-505"
                     and r.get("projection_component") == "castle-hall-northwest-contact"]) == 1,
                "Contact material receiver missing or duplicate")
    states = bindings["states"]
    require(len(states) == 2 and {s["state"] for s in states} == set(layers),
            "Saved state bindings must cover both states exactly")
    from occlusion_constraints import evidence_record
    mask_evidence = evidence_record(config["source_mask_manifest"])
    directory = Path(config["projection_manifest"]).parent
    expected = {}
    for state in ("covered", "revealed"):
        source = (directory / manifest["sources"]["exterior" if state == "covered" else "interior"]).resolve()
        hidden = visibility[state]
        row = {"source_path": str(source), "projection_label": "exterior" if state == "covered" else "interior-" + patch,
               "receiver_nodes": config["part_ids"],
               "occluder_nodes": sorted(available - set(hidden.get("hidden_nodes", [])))}
        if state == "revealed":
            row["exclude_occluder_components"] = hidden.get("hidden_components", [])
        rows = [row]
        if supplement and state == "revealed":
            row["receiver_components"] = [dict(source_node="building-505", patch_id=patch,
                projection_components=["castle-hall-retained-roof", "castle-hall-removable-cover"])]
            rows.append(dict(source_path=str((directory / manifest["sources"]["exterior"]).resolve()),
                projection_label="exterior", receiver_nodes=["building-505"], occluder_nodes=row["occluder_nodes"],
                receiver_components=[dict(source_node="building-505", patch_id=patch,
                    projection_components=["castle-hall-northwest-contact"])]))
        require(layers[state] == rows, "Complete state projection differs from source/ownership/visibility authority")
        expected[state] = [dict(r, source_sha256=sha(r["source_path"])) for r in rows]
        binding = next(s for s in states if s["state"] == state)
        require(binding.get("hidden_context_nodes") == sorted(set(hidden.get("hidden_nodes", [])) - set(config["part_ids"])),
                "Saved state foreign visibility differs from reviewed context")
        model, frame = Path(binding["model"]), Path(binding["frame_manifest"])
        require(model.resolve().is_relative_to(workspace.resolve()) and frame.resolve().is_relative_to(workspace.resolve()),
                "Saved state paths escape workspace")
        require(sha(model) == binding["model_sha256"] and sha(frame) == binding["frame_manifest_sha256"],
                "Saved state model or packet changed")
        if supplement:
            proof_binding = supplement["states"][state]
            proof_path = Path(proof_binding["path"])
            require(proof_path.resolve() == (workspace / "inspection/state-models" / state / "source-preservation.json").resolve()
                    and sha(proof_path) == proof_binding["sha256"], "Contact state preservation proof changed")
            proof = read(proof_path)
            require(proof.get("status") == "PASS" and proof.get("model_sha256") ==
                    proof_binding["model_sha256"] == binding["model_sha256"], "Contact state proof has stale model")
            require(bool(proof.get("extension_hash")), "Contact object/material signature absent")
            if state == "covered":
                contact_signature = proof["extension_hash"]
            else:
                require(proof["extension_hash"] == contact_signature, "Contact material or geometry differs between states")
        hidden_components = {(r["source_node"], r["projection_component"]) for r in hidden.get("hidden_components", [])}
        visible_names = {r["object"] for r in records if r["source_node"] not in hidden.get("hidden_nodes", [])
                         and (r["source_node"], r.get("projection_component")) not in hidden_components}
        packet = read(frame)
        require(all(set(value) == visible_names and len(value) == len(visible_names)
                    for value in (binding["object_names"], packet["object_names"], packet["render_object_names"])),
                "Saved state visibility differs from reviewed components")
        require(packet["projection_layers"] == expected[state], "Saved state projection layers differ")
        require(packet["source_sha256"] == sha(source) and Path(packet["source_image"]).resolve() == source,
                "Saved state source differs")
        require(packet.get("source_mask_evidence") == mask_evidence, "Saved state mask evidence differs")
        supplemental_packet(frame.parent, config["asset_id"], complete_state_framing(workspace, state, framing))
    return expected["covered"]


def complete_state_framing(workspace, state, default):
    """Resolve inherited state cameras only through exact preservation authority."""
    supplement_path = workspace / 'contact-state-supplement.json'
    if not supplement_path.exists():
        return default
    supplement = read(supplement_path)
    require(supplement.get('model_sha256') == sha(workspace / 'model.blend'), 'State framing has stale primary model')
    origin = supplement.get('original_state_manifest')
    if not origin:
        return default
    require(state in ('covered', 'revealed'), 'Unknown preserved framing state')
    origin_path = Path(origin['path'])
    require(origin_path.resolve() == ORIGINAL_HALL_STATE_MANIFEST.resolve(), 'Unrecognized original state framing authority')
    require(sha(origin_path) == origin['sha256'], 'Frozen original state manifest changed')
    binding = next(r for r in read(workspace / 'inspection/state-models/manifest.json')['states'] if r['state'] == state)
    proof_binding = supplement['states'][state]
    proof_path = workspace / 'inspection/state-models' / state / 'source-preservation.json'
    require(Path(proof_binding['path']).resolve() == proof_path.resolve() and sha(proof_path) == proof_binding['sha256'],
            'State framing preservation proof changed')
    proof = read(proof_path)
    require(proof.get('status') == 'PASS' and proof['model_sha256'] == binding['model_sha256'] == sha(binding['model']),
            'State framing has stale saved model')
    original_state = next(r for r in read(origin_path)['states'] if r['state'] == state)
    require(sha(original_state['model']) == original_state['model_sha256'] == proof['original_model_sha256'],
            'Frozen original state model differs from preservation authority')
    original_frame = Path(original_state['frame_manifest'])
    require(sha(original_frame) == original_state['frame_manifest_sha256'], 'Frozen original state framing changed')
    return read(original_frame)


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
    expected_parts = (['ground'] if asset.get('role') == 'terrain' else
                      sorted(f"building-{p['obstacle']:03d}" for p in asset["parts"]))
    require(config["part_ids"] == expected_parts,
            "Workspace parts differ from current catalog")
    selectors = sorted((f"building-{part['obstacle']:03d}", component)
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
        if (workspace / "projection-state-layers.json").exists():
            expected_layers = complete_state_records(workspace, config, reviewed_projection, available,
                                                     packets["input"])
        else:
            expected_layers = projection_records(config, reviewed_projection, available)
        if asset['id'] in ('nottingham-upper-prison', 'nottingham-southwest-prison') and any(
                '-prison-door-' in row.get('projection_label', '') for row in after_layers):
            expected_layers = prison_endpoint_records(workspace, config, expected_layers)
        require(after_layers == expected_layers,
                "Modified projection layers do not match validated own-asset projection reviews")
    elif config.get("source_projection_ground_exclusion"):
        from ground_projection_review import validate as validate_ground_projection
        validate_ground_projection(config, packets["input"]["projection_layers"],
                                   packets["modified"]["projection_layers"])
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
        limitations.extend(neutral_source_constraint_notes(
            mask_assignments, packets['modified'].get('projection_layers', []),
            config['part_ids'], asset['id']))
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
    if (workspace / "projection-state-layers.json").exists():
        evidence["complete_state_contracts"] = {
            name: sha(workspace / name) for name in (
                "projection-state-layers.json", "material-states.json",
                "inspection/state-models/manifest.json")}
        if (workspace / "contact-state-supplement.json").exists():
            evidence["complete_state_contracts"]["contact-state-supplement.json"] = sha(workspace / "contact-state-supplement.json")
    return status, limitations, evidence, review


def prison_endpoint_records(workspace, config, layers):
    """Validate the explicit closed/open door partition against frozen source authority."""
    upper = config['asset_id'] == 'nottingham-upper-prison'
    patch = 'patch-002' if upper else 'patch-007'
    prefix = 'upper-prison-door' if upper else 'southwest-prison-door'
    doors = ['building-453', 'building-455'] if upper else ['building-476', 'building-477']
    require(set(doors) <= set(config['part_ids']), 'Door endpoints are not owned by this prison')
    masks = read(config['source_mask_manifest'])
    references = read(workspace / 'door-reference/manifest.json')
    interior = next(row for row in layers if row['projection_label'] == 'interior-' + patch)
    for row in layers:
        row['receiver_nodes'] = [node for node in row['receiver_nodes'] if node not in doors]
        row['occluder_nodes'] = [node for node in row['occluder_nodes'] if node not in doors]
    for endpoint, node in zip(('initial', 'applied'), doors):
        label = prefix + '-' + endpoint
        source = Path(references[label]['source'])
        digest = sha(source)
        require(digest == references[label]['sha256'] == masks['projections'][label]['source_sha256'],
                'Prison endpoint artwork differs from immutable mask authority')
        layers.append({'source_path': str(source), 'source_sha256': digest,
                       'receiver_nodes': [node],
                       'occluder_nodes': sorted(set(interior['occluder_nodes']) | {node}),
                       'exclude_occluder_components': interior.get('exclude_occluder_components', []),
                       'projection_label': label})
    return layers


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
    root = Path(__file__).resolve().parents[2] / "work/needs-review/nottingham-refinement"
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--catalog", type=Path, default=root / "grouping/catalog-v15.json")
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
    require(catalog.get("map") == "nottingham", "Expected Nottingham catalog")
    catalog = {**catalog, 'groups': [*catalog['groups'], {
        'id': 'nottingham-terrain-ground', 'name': 'Terrain and bridge stream bed',
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
    audit_gate_path = root / "source-coverage-audit-gate.json"
    audit_gate = read(audit_gate_path)["assets"] if audit_gate_path.exists() else {}
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
                state_records['covered'] = supplemental_packet(folder, asset['id'], complete_state_framing(workspace, 'covered', framing))
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
                state_records['revealed'] = supplemental_packet(state_dir, asset['id'], complete_state_framing(workspace, 'revealed', framing))
                require(local(worker['revealed_textured']).resolve() == state_dir.resolve() / 'textured.png'
                        and local(worker['revealed_context']).resolve() == state_dir.resolve() / 'context.png',
                        'Revealed sheets must come from one validated packet')
                baseline_dir = local(worker.get('revealed_input', 'revealed/input'))
                config = read(workspace / 'workspace.json')
                if (workspace / 'contact-state-supplement.json').exists():
                    original_workspace = ORIGINAL_HALL_STATE_MANIFEST.parents[2]
                    require(baseline_dir.resolve() == (original_workspace / 'input').resolve(),
                            'Preserved hall baseline differs from frozen original input')
                    original_config = read(original_workspace / 'workspace.json')
                    require(file_hashes(baseline_dir) == original_config['input_files'],
                            'Frozen hall baseline packet changed')
                origin = frozen_mask_origins(workspace, config)
                if (workspace / 'contact-state-supplement.json').exists():
                    origin.update(frozen_mask_origins(original_workspace, original_config))
                state_records['revealed_input'] = supplemental_packet(
                    baseline_dir, asset['id'], complete_state_framing(workspace, 'revealed', framing), mask_origin=origin)
            evidence['state_packets'] = state_records
            contracts = evidence.get('complete_state_contracts')
            if contracts:
                bindings = read(workspace / 'inspection/state-models/manifest.json')
                for binding in bindings['states']:
                    displayed = state_records.get(binding['state'])
                    require(displayed is not None and Path(displayed['directory']).resolve() ==
                            Path(binding['frame_manifest']).resolve().parent,
                            'Displayed state differs from validated complete-state packet')
            evidence['state_bundle_sha256'] = state_bundle_hash(state_records, contracts)
            from lighting_review import load_lighting_review
            lighting_review, lighting_solids = load_lighting_review(
                workspace, evidence, root / 'lighting-calibration/map-lighting.json')
            evidence['lighting_review'] = lighting_review
            evidence['lighting_review_sha256'] = lighting_review['sha256'] if lighting_review else None
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
        if asset['id'] in audit_gate and status != 'approved':
            audit_path = workspace / 'source-coverage-audit.json'
            audit = read(audit_path) if audit_path.exists() else {}
            audit_current = source_coverage_audit_matches(audit, evidence)
            if not audit_current:
                status = 'audit-pending'
                limitations.append('Source-visible texture completeness audit pending; not ready for user review.')
            else:
                evidence['source_coverage_audit'] = {'path': str(audit_path), 'sha256': sha(audit_path)}
        evidence["status"] = status
        if lighting_review:
            limitations.append('Solid views use Nottingham artwork-calibrated sunlight (azimuth -69.8°, elevation 43.5°; approximate). Frozen source-textured evidence is preserved.')
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
            state_item = {'id': key,
                          'name': key.removeprefix('animation-').replace('-', ' ').capitalize(),
                          'description': ('The winch/lever sprite changes between these states; tower masonry is unchanged.'
                              if key.startswith('animation-mechanism-') else
                              'An animated state of this same asset. Compare the moving part with the other state.'),
                          'solid': str(folder / 'solid.png'),
                          'textured': str(folder / 'textured.png'),
                          'context': str(folder / 'context.png')}
            item.setdefault('animation_reviews', []).append(state_item)
        if lighting_review:
            for key in ('solid', 'revealed_solid'):
                if key in item:
                    item[key] = lighting_solids[str(Path(item[key]).resolve())]
            for state in item.get('animation_reviews', []):
                state['solid'] = lighting_solids[str(Path(state['solid']).resolve())]
        progress.append({**row, "status": status, "geometry_refined": evidence["geometry_refined"],
                         "geometry_reviewed": evidence["geometry_reviewed"],
                         "review_outcome": evidence["review_outcome"], "user_approval": user_approval})
    manifest = output.parent / (output.name + "-candidates.json")
    progress_path = output.parent / (output.name + "-progress.json")
    counts = dict(Counter(item["status"] for item in progress))
    shared_gallery = Path(__file__).resolve().parents[2] / 'refinement/blender/build_review_gallery.py'
    gallery_tooling = {'path': str(shared_gallery), 'sha256': sha(shared_gallery)}
    write(manifest, {"version": 1, "map": "Nottingham", "items": items, "tooling": tooling,
                     'gallery_tooling': gallery_tooling, 'total_groups': len(ids) - 1,
                     'supplemental_count': 1, 'status_counts': counts,
                     'without_packets': [row for row in progress if row['status'] in ('missing', 'validation-pending')]})
    write(progress_path, {"version": 1, "map": "Nottingham", "catalog": str(args.catalog.resolve()),
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
    gallery_module.build(manifest, output, pending_only=True, map_name="Nottingham")
    print(json.dumps({"progress": str(progress_path), "counts": counts,
                      "gallery": str(output / "index.html")}))


if __name__ == "__main__":
    main()
