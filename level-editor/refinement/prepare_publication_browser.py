"""Prepare immutable staged or live library inputs for the private browser audit.

Preserves the live editor document when canonical group and part identities match.
The scope lists asset_ids, already_published, and optional required_patches.
"""
import argparse
import hashlib
import json
import re
from asset_index import write_asset_index, discover_asset_index
import subprocess
import tempfile
from pathlib import Path
from scene_manifest import scene_metadata
from stored_map import expand_document, store_document


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def preserved_ungrouped(document, previous):
    """Carry editor-owned placements only with their exact original asset pins."""
    objects = lambda doc: {obj['id']: obj for obj in doc['objects'] if not obj.get('group')}
    current, before = objects(document), objects(previous)
    if current != before:
        raise ValueError('Ungrouped editor placements changed during publication')
    ids = {obj['node'].split(':', 2)[1] for obj in current.values()}
    pins = lambda doc: {ref['id']: ref for ref in doc.get('assetSources', []) if ref['id'] in ids}
    if set(pins(document)) != ids or pins(document) != pins(previous):
        raise ValueError('Ungrouped editor asset pins changed during publication')
    return len(current)


def bound_patches(nodes, document):
    """Mission patch IDs the editor exposes for this map.

    Models name reusable appearances asset-locally (appearance-N); placements (group or
    ungrouped part `patches`) bind them to mission patch IDs, which is what the editor and the
    audit see. Direct mission IDs in older models pass through unchanged. Every local
    appearance must be bound by some placement.
    """
    triggers = set()
    for node in nodes:
        extras = node.get("extras", {})
        if extras.get("reveal_material_patch"):
            triggers.add(extras["reveal_material_patch"])
        for key in ("reveal_hide_when_applied", "reveal_show_when_applied"):
            triggers.update(extras.get(key, []))
    mappings = [mapping for item in document.get("groups", []) + document.get("objects", [])
                for mapping in (item.get("patches") or {}).values()]
    bound = {local for mapping in mappings for local in mapping}
    local = {trigger for trigger in triggers if re.fullmatch(r"appearance-\d+", trigger)}
    if local - bound:
        raise ValueError("Asset-local appearances without a placement patch binding: " + repr(sorted(local - bound)))
    return (triggers - local) | {patch for mapping in mappings for patch in mapping.values()}


def prepare(stage, scope_path, output, *, map_name="leicester", live=False, migration_path=None, document_path=None):
    if document_path is not None and (live or migration_path is not None):
        raise ValueError("Explicit staged document cannot replace live or migration authority")
    stage, output = Path(stage).resolve(), Path(output).resolve()
    scope = json.loads(Path(scope_path).read_text())
    library = Path("level-editor/library").resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    live_document = library / f"scenes/{map_name}.rhlos-map.json"
    source_document = Path(document_path).resolve(strict=True) if document_path is not None else live_document
    asset_library = library if live else stage / 'map-assets'
    document = expand_document(asset_library if document_path is not None else library,
                               json.loads(source_document.read_text()))
    staged_document = document if live else expand_document(asset_library,
        json.loads((stage / f"{map_name}.rhlos-map.json").read_text()))
    model = scene_metadata(asset_library, staged_document)
    nodes = model["nodes"]
    map_node = next(node for node in nodes if node.get("name") == "map")
    ungrouped_assets = {obj['node'].split(':', 2)[1] for obj in document['objects']
                        if not obj.get('group') and obj['node'].startswith('asset:')}
    groups = [nodes[index] for index in map_node["children"]
              if nodes[index].get("name") != "ground"
              and nodes[index].get('extras', {}).get('asset_group') not in ungrouped_assets]
    part_names = {nodes[index]["name"] for group in groups for index in group.get("children", [])}
    if migration_path is not None and not live:
        migration = json.loads(Path(migration_path).read_text())
        if sha(live_document) != migration["prior_document_sha256"]:
            raise ValueError("Editor document changed after migration preparation")
        for evidence in migration.get("evidence", []):
            if sha(Path(evidence["path"])) != evidence["sha256"]:
                raise ValueError("Migration evidence changed")
        group_records = {group["id"]: group for group in document["groups"]}
        objects = {obj["id"]: obj for obj in document["objects"]}
        for transfer in migration.get("group_transfers", []):
            obj = objects[transfer["id"]]
            if obj["group"] != transfer["from"]:
                raise ValueError("Unexpected previous part ownership")
            if group_records[transfer["from"]]["transform"] != group_records[transfer["to"]]["transform"]:
                raise ValueError("Ownership transfer needs explicit world-transform migration")
            obj["group"] = transfer["to"]
            if "name" in transfer:
                obj["name"] = transfer["name"]
        for obj in migration.get("new_objects", []):
            if obj["id"] in objects or obj["node"] not in part_names:
                raise ValueError("New editor part is duplicated or absent from staged scene")
            if obj["group"] not in group_records:
                raise ValueError("New editor part has unknown ownership")
            document["objects"].append(obj)
            objects[obj["id"]] = obj
    # An explicit document already uses the staged catalog and its local pivots.
    # Rebasing it through the live library fails for first publications/new IDs.
    if not live and document_path is None and staged_document.get('assetSources'):
        with tempfile.NamedTemporaryFile(mode='w', suffix='.json', dir=stage) as previous:
            json.dump(document, previous); previous.flush()
            document=expand_document(asset_library, json.loads(subprocess.check_output(['node',
                str(Path(__file__).resolve().parents[1]/'pipeline/src/rebase-map-assets.ts'),
                previous.name, str(library), str(stage/f'{map_name}.rhlos-map.json'), str(asset_library)], text=True)))
    def canonical(node): return node.split(':', 2)[-1] if node.startswith('asset:') else node
    ungrouped_count = 0
    if live_document.exists():
        previous = expand_document(library, json.loads(live_document.read_text()))
        ungrouped_count = preserved_ungrouped(document, previous)
    elif any(not obj.get('group') for obj in document['objects']):
        raise ValueError('Ungrouped placements require existing publication evidence')
    grouped_objects = [obj for obj in document['objects'] if obj.get('group')]
    if {canonical(obj["node"]) for obj in grouped_objects} != part_names:
        raise ValueError("Canonical part identities changed; explicit editor document migration required")
    if {group["id"] for group in document["groups"]} != {group["extras"]["asset_group"] for group in groups}:
        raise ValueError("Canonical group identities changed; explicit editor document migration required")
    part_groups = {nodes[index]["name"]: group["extras"]["asset_group"]
                   for group in groups for index in group.get("children", [])}
    if any(obj["group"] != part_groups[canonical(obj["node"])] for obj in grouped_objects):
        raise ValueError("Editor part ownership differs from staged canonical hierarchy")
    document_path = live_document
    if not live:
        document["sceneAssets"] = staged_document["sceneAssets"]
        document.pop("glb", None)
        document.setdefault("provenance", {}).pop("glb_sha256", None)
        document_path = stage / "browser-document.rhlos-map.json"
        stored = store_document(asset_library, document)
        if document_path.exists():
            if json.loads(document_path.read_text()) != stored:
                raise ValueError("Existing staged document differs from current canonical editor state")
        else:
            document_path.write_text(json.dumps(stored, indent=2) + "\n")
    sources = {entry["id"]: (entry, library / "3d-assets") for entry in
               discover_asset_index(library / "3d-assets")["assets"]}
    if not live:
        # Same rule as promotion: the staged catalog carries the refreshed lossy/preview
        # derivatives; the raw standalone export does not, and falling back to live
        # derivatives would pair them with the new staged models.
        staged_root = stage / "map-assets/3d-assets"
        if not any(staged_root.rglob("asset.json")):
            staged_root = stage / "assets"
        sources.update({entry["id"]: (entry, staged_root) for entry in
                        discover_asset_index(staged_root)["assets"]})
    # Descriptor-pinned scene assets (the map ground) are always loaded by the editor, which
    # validates their pins against the index; they belong to the private index whether or not
    # the scope names them. Descriptor-less local scene models need no index entry.
    scene_ids = {reference["id"] for reference in staged_document["sceneAssets"] if reference.get("descriptor")}
    expected_ids = (set(scope["asset_ids"]) | set(scope["already_published"]) | scene_ids
                    | {ref['id'] for ref in document.get('assetSources', [])})
    if not expected_ids <= sources.keys():
        raise ValueError("Missing expected assets: " + repr(sorted(expected_ids - sources.keys())))
    entries = [sources[identity][0] for identity in sorted(expected_ids)]
    private_index = output.with_name("private-index.json")
    prospective = {}
    for entry in entries:
        source = sources[entry["id"]][1]
        for key in ("descriptor", "model", "lossy_model", "preview_model"):
            if entry.get(key): prospective[entry[key]] = source / entry[key]
        if entry.get("lossy_model"):
            receipt = entry["lossy_model"] + ".receipt.json"
            prospective[receipt] = source / receipt
        # Absence in the selected catalog also overrides live derivatives. A
        # newly staged original must never inherit a receipt for an older GLB.
        for kind in ('lossy', 'preview'):
            if entry.get(kind + '_model'):
                continue
            model_path = Path(entry['model'])
            for basename in {kind + '.glb', model_path.stem + '.' + kind + '.glb'}:
                candidate = model_path.with_name(basename).as_posix()
                prospective[candidate] = None
                prospective[candidate + '.receipt.json'] = None
    write_asset_index(library / "3d-assets", target=private_index, files=prospective,
                      descriptors=[entry["descriptor"] for entry in entries])
    files, seen = [], set()

    def add(path, source):
        if path in seen:
            return
        source = source.resolve(strict=True)
        files.append({"path": path, "url": "/@fs/" + str(source), "sha256": sha(source)})
        seen.add(path)

    for reference in document["sceneAssets"] + document.get("assetSources", []):
        add(reference["model"], asset_library / reference["model"])
        for resource in reference.get("resources", []):
            add(resource["path"], asset_library / resource["path"])
    add(f"scenes/{map_name}.rhlos-map.json", document_path)
    add("3d-assets/index.json", private_index)
    expanded = []
    for entry in entries:
        source = sources[entry["id"]][1]
        descriptor = json.loads((source / entry["descriptor"]).read_text())
        add("3d-assets/" + entry["model"], source / entry["model"])
        for resource in descriptor.get("resources", []):
            add(resource["path"], source / Path(resource["path"]).relative_to("3d-assets"))
        if entry.get("preview_model"):
            add("3d-assets/" + entry["preview_model"], source / entry["preview_model"])
        if entry.get("lossy_model"):
            # The private index was validated above; the browser needs only model bytes.
            add("3d-assets/" + entry["lossy_model"], source / entry["lossy_model"])
        variants = descriptor.get("state_variants") or descriptor.get("standalone_variants")
        # The palette has one card per asset; alternate appearances open inside
        # that card. Keep their model files pinned without counting extra cards.
        expanded.append({**entry, **({"inserted_id": entry["id"] + "--state-initial"}
                                    if descriptor.get("state_variants") else {})})
        if not variants:
            continue
        for state in ("initial", "applied"):
            if state not in variants:
                continue
            variant = variants[state]
            model_path = str(Path(entry["descriptor"]).parent / variant["model"])
            add("3d-assets/" + model_path, source / model_path)
    # The editor opens mission/game data alongside its asset library. Include
    # the indexed read-only files in the same hash-pinned private HTTP catalog.
    game_index = library / 'game-data/index.json'
    if game_index.is_file():
        add('game-data/index.json', game_index)
        for path in json.loads(game_index.read_text())['files']:
            add('game-data/' + path, library / 'game-data' / path)
    generated = {}
    for material in model.get("materials", []):
        identity = material.get("extras", {}).get("generated_source_sha256")
        if identity:
            generated[identity] = generated.get(identity, 0) + 1
    patches = bound_patches(nodes, staged_document)
    required = scope.get("required_patches", sorted(patches))
    if not set(required) <= patches:
        raise ValueError("Required runtime state triggers missing: " + repr(sorted(set(required) - patches)))
    protected = {str(path): sha(path) for path in library.rglob("*")
                 if path.is_file() and path.suffix in (".json", ".gltf", ".glb", ".bin", ".png", ".jpg")}
    config = {"map": map_name, "mode": "live" if live else "staged", "files": files,
              "shared_module_url": "/@fs/" + str(Path(__file__).resolve().parents[1] / 'shared/src/index.ts'),
              "expected": {"groups": len(document["groups"]), "parts": len(document["objects"]),
                           "ungrouped_parts": ungrouped_count,
                           "width": document["size"][0], "assets": expanded,
                           "base_asset_ids": sorted(expected_ids), "new_asset_ids": scope["asset_ids"],
                           "generated_materials": generated, "required_patches": required},
              "stage": str(stage), "protected_live_files": protected}
    output.write_text(json.dumps(config, indent=2) + "\n")
    return {"config": str(output), "groups": len(document["groups"]),
            "parts": len(document["objects"]), "palette": len(expanded),
            "files": len(files), "patches": sorted(patches)}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("stage", type=Path)
    parser.add_argument("scope", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--map", default="leicester")
    parser.add_argument("--live", action="store_true")
    parser.add_argument("--migration", type=Path)
    parser.add_argument("--document", type=Path, help="Explicit canonical staged document for first publication")
    args = parser.parse_args()
    print(json.dumps(prepare(args.stage, args.scope, args.output, map_name=args.map,
                             live=args.live, migration_path=args.migration,
                             document_path=args.document)))
