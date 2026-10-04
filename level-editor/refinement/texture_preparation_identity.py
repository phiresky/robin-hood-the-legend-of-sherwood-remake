"""Validate the separate approved geometry and normalized preparation identities."""
import json
import hashlib
from pathlib import Path
from review_evidence import sha


def resolve(item, workspace, protected, experiment, approval, frames, *, supplemental=False):
    parent = item.get('parent_geometry_revision')
    if not parent:
        return item, workspace, item['revision']['sha256'], item['revision']['model_sha256']
    receipt_path = experiment / 'preparation.json'
    receipt = json.loads(receipt_path.read_text())
    manifest = Path(receipt['source_review_manifest']).resolve(strict=True)
    if sha(manifest) != receipt['review_manifest_sha256']:
        raise ValueError('Normalized preparation manifest changed')
    # A supplemental state has its own exact preparation approval, while sharing
    # the same parent geometry approval. Never reuse the primary state's model.
    if approval.get('preparation_revision') != item['revision']['sha256']:
        if not supplemental:
            raise ValueError('Normalized preparation revision differs from approval')
        from stage_approved_editor_asset import validate
        selected, selected_workspace, selected_files = validate(manifest, item['id'])
        original_provenance = item.get('approval_provenance', {})
        selected_provenance = selected.get('approval_provenance', {})
        if (selected.get('parent_geometry_revision') != parent or
                original_provenance.get('selected_state') != item.get('preparation_state') or
                selected_provenance.get('selected_state') != selected.get('preparation_state') or
                {k: v for k, v in selected_provenance.items() if k != 'selected_state'} !=
                {k: v for k, v in original_provenance.items() if k != 'selected_state'}):
            raise ValueError('Supplemental preparation belongs to different approved geometry')
        item, workspace = selected, selected_workspace
        protected.update(selected_files)
        decisions = manifest.parent / 'decisions.json'
        protected[decisions] = sha(decisions)
    revision = item['revision']['sha256']
    manifest_items = [entry for entry in json.loads(manifest.read_text()).get('items', [])
                      if entry.get('id') == item['id']]
    if len(manifest_items) != 1 or manifest_items[0].get('revision') != item['revision']:
        raise ValueError('Normalized preparation receipt selects another revision')
    if (approval.get('preparation_revision') != revision or
            frames.get('preparation_revision') != revision or
            approval.get('geometry_revision') != parent or
            frames.get('geometry_revision') != parent or
            receipt.get('approved_revision') != parent):
        raise ValueError('Normalized parent/preparation identity differs from approved evidence')
    evidence = {str(Path(e['path']).resolve()): e['sha256']
                for e in item['revision']['evidence'].values()}
    def bound(path):
        path = Path(path).resolve(strict=True)
        digest = sha(path)
        if evidence.get(str(path)) != digest:
            raise ValueError('Normalized selection artifact is not approved: ' + str(path))
        protected[path] = digest
        return path
    selection_path = bound(item['preparation_selection'])
    selection = json.loads(selection_path.read_text())
    if (selection.get('parent_geometry_revision') != parent or
            any(item.get(key) != value for key, value in selection.items())):
        raise ValueError('Normalized preparation selection changed')
    if (approval.get('approval_provenance') != item.get('approval_provenance') or
            approval.get('source_decision', {}).get('revision_sha256') != revision or
            approval.get('source_decision', {}).get('decision') != 'approved'):
        raise ValueError('Normalized preparation lost its exact geometry approval lineage')
    model = bound(item['preparation_model'])
    if 'preparation_state' in item:
        state = item['preparation_state']
        if approval.get('review_state') != state or frames.get('review_state') != state:
            raise ValueError('Normalized preparation state differs from approved selection')
    else:
        # Legacy covered-only packets bind their model and cameras directly to
        # the recorded gallery decision, without a separate state selection.
        provenance = item['approval_provenance']
        source = provenance.get('source_approval', {})
        identity = {key: source.get(key) for key in
                    ('asset_id', 'model_sha256', 'modified_views_sha256', 'state_bundle_sha256', 'lighting_review_sha256')}
        source_parent = hashlib.sha256(json.dumps(identity, sort_keys=True, separators=(',', ':')).encode()).hexdigest()
        if (supplemental or provenance.get('selected_state') != 'covered' or
                approval.get('review_state') is not None or frames.get('review_state') is not None or
                source.get('decision') != 'approved' or source.get('asset_id') != item['id'] or
                source.get('model_sha256') != sha(model) or source_parent != parent or
                source.get('modified_views_sha256') != sha(bound(workspace/'modified/views.json'))):
            raise ValueError('Covered preparation differs from its explicit geometry approval')
    protected[receipt_path] = sha(receipt_path)
    protected[manifest] = sha(manifest)
    return item, workspace, parent, sha(model)
