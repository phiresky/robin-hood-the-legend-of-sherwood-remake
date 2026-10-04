"""Enumerate every native mask without confusing state graphics with scenery.

This is an ownership/work backlog, not geometry, texture, or user approval.
Run again after catalog additions; private proposals never count as integrated.
"""
import argparse
from collections import Counter, defaultdict
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / 'level-editor/work/croisement02-refinement'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def read(path):
    return json.loads(path.read_text())


# Visually inspected source cutouts, supplemented by explicit source-domain recipes.
SCENERY = {
    49: ['west-rock-outcrop'], 50: ['northwest-rock-outcrop'],
    51: ['northwest-rock-outcrop'], 52: ['southwest-rock-outcrop'],
    53: ['northwest-rock-outcrop'],
    96: ['southeast-stone-wall-and-gate', 'east-rail-fence'],
    97: ['southeast-stone-wall-and-gate', 'east-rail-fence'],
    98: ['south-field-wattle-fence'], 99: ['southwest-path-wattle-fence'],
    100: ['southwest-field-wattle-fence'], 101: ['east-stone-wall-and-gate'],
    102: ['southwest-log-pile'], 103: ['southwest-log-pile'],
    104: ['southwest-kindling-bundle'], 105: ['southwest-stumps'],
    106: ['southwest-stumps'], 107: ['logging-clearing-stumps'],
    108: ['logging-clearing-stumps', 'logging-clearing-log'],
    109: ['north-firewood-stack'], 110: ['woodcutters-shed'],
    124: ['south-field-haystack'], 125: ['north-woodland-bank'],
    126: ['north-woodland-bank'], 127: ['woodcutters-shed'],
    136: ['central-covered-state'], 137: ['west-covered-state'],
    138: ['south-fence-applied-state'],
    139: ['north-applied-state-assembly'], 140: ['north-applied-state-assembly'],
    141: ['north-applied-state-assembly'],
}
NOTES = {
    9: 'Standalone stem under existing canopy; no uniquely owned new crown is asserted.',
    18: 'Tree wood and north kindling use separate authored domains301/300; native mask alone is not exclusive ownership.',
    21: 'Existing tree21 uses obstacle132; historical terrain-bank label was corrected. Sparse original canopy context is explicit.',
    22: 'Northern boundary foliage fragment, not exposed trunk. Crown association and source ownership remain unresolved.',
    44: 'Standalone companion stem near tree45; no separate crown inferred.',
    48: 'Same visible trunk as native47 on gameplay layer1, confirmed in 47-48-gameplay-layer-source.png; not another physical tree.',
    52: 'Current rock domain excludes foreground81 and83. Approved stump106 is disjoint.',
    54: 'Northwest boundary shrub; required foreground/contact companion for rocks50/51/53. Infer off-map continuation.',
    55: 'Existing approved geometry retained. Repeated foliage rows flagged; texture API held pending separate geometry revision.',
    56: 'One integrated western bank with61; isolated round5 microfragment revision awaits donor cleanup and renewed joint evidence.',
    57: 'Mixed rock and foliage: reviewed rock350 belongs west rocks; complementary foliage351 still needs authored geometry.',
    58: 'Existing approved geometry retained. Repeated foliage rows flagged; texture API held pending separate geometry revision.',
    59: 'Existing approved geometry retained. Repeated foliage rows flagged; texture API held pending separate geometry revision.',
    60: 'Mixed rock, wood and foliage. Complement352 excludes reviewed rock350; remaining wood/leaf ownership needs inspection before shrub construction.',
    61: 'Same authored west bank as56, not a second independent owner.',
    62: 'Overlaps wood3/4, shrub63, canopy133 and covered-state137; resolve shared pixels and state behavior before authoring.',
    63: 'Overlaps wood5, shrub62, canopies130/133 and covered-state136; resolve shared pixels and state behavior before authoring.',
    64: 'Mixed exposed birch trunks and foliage; reconcile existing tree00/01 wood before assigning a leaf domain.',
    65: 'Isolated domain415=native65 minus canopy134 (47pixels). Microfragments and donor alpha under review; joint with bank/tree14 pending.',
    66: 'Isolated domain416=native66. Joint with current bank and trees12/13 plus donor cleanup pending.',
    70: 'Narrow foliage fragments around existing wood; source ownership first, no duplicate trunk.',
    73: 'Northern boundary foliage silhouette; infer missing continuation without inventing source-observed pixels.',
    75: 'Mixed foliage and upright fence95: subtract reviewed fence domain431, not the whole overlapping native box.',
    76: 'Substantial wattle fence within native foliage cutout; partition approved fence pixels before authoring foliage.',
    81: 'Isolated native-only domain414; southwest rock contact reviewed. Donor cleanup and current joint receipt required before integration.',
    83: 'Foreground fringe overlaps southwest rock52 by121pixels; rock excludes this native plant.',
    85: 'Foreground foliage overlaps fence95; excluded from its observed wood domain.',
    87: 'Foreground foliage overlaps fence94; excluded from its observed wood domain.',
    89: 'Several boundary clumps near stem44; excluded from observed stem wood and require inferred off-map continuation.',
    91: 'Mixed exposed wood and foliage near tree45; split existing wood before assigning new leaf owner.',
    93: 'Mixed stump, leaves and obscured fence95; reconcile existing stump and fence ownership before geometry.',
    94: 'Missing upright rail fence, isolated authored domain430 being prepared by fence refinement worker.',
    95: 'Missing upright rail fence, isolated authored domain431 being prepared by fence refinement worker.',
    96: 'Stone wall returns share two existing groups; group name east-rail-fence does not mean native94/95.',
    97: 'Stone wall/gate partition shares two existing groups; exact source-domain split remains in worker evidence.',
    108: 'Two physical groups share native source context; preserve existing stump/log partition.',
    110: 'Small stump beside shed is explicitly retained as building139, a flared stump with separate cut cap; checked refinement receipt and all eight actual-material views.',
    125: 'North woodland bank parts0–4: audited authored receiver420 explicitly retains native125/126; not the northeast oak root bank.',
    126: 'North woodland bank parts0–4: audited authored receiver420 explicitly retains native125/126; not the northeast oak root bank.',
}


def audit(out):
    manifest_path = out / 'baseline/masks/manifest.json'
    level_path = out / 'baseline/Croisement02.rhp.json'
    catalog_path = out / 'ownership-revision/catalog.json'
    canopy_path = out / 'forest-v4-sources/manifest.json'
    native = read(manifest_path)['masks']
    level = read(level_path)
    catalog = read(catalog_path)
    groups = {g['id']: g for g in catalog['groups']}
    ids = {m['index'] for m in native}
    assert ids == set(range(142)), 'Native inventory changed: review classifications'
    by_local = {(m['layer'], m['layer_index']): m['index'] for m in native}
    phase = defaultdict(list)
    for index, patch in enumerate(level['patches']):
        for key in ('old_masks', 'new_masks'):
            for local in patch[key]:
                global_index = by_local[(local['layer'], local['index'])]
                phase[global_index].append({'patch': index, 'role': key})
    owner = defaultdict(set)
    for asset, group in groups.items():
        for field in ('wood_mask', 'native_wood_mask', 'native_foliage_mask'):
            if field in group:
                owner[group[field]].add(asset)
        for mask in group.get('native_foliage_masks', []):
            owner[mask].add(asset)
    for mask, suffixes in SCENERY.items():
        for suffix in suffixes:
            asset = 'croisement02-' + suffix
            assert asset in groups, asset
            owner[mask].add(asset)
    owner[48].add('croisement02-tree-47')
    for row in read(canopy_path):
        owner[128 + row['animation']].add(row['asset_id'])
    records = []
    for native_row in native:
        index = native_row['index']
        state = phase[index]
        new = any(p['role'] == 'new_masks' for p in state)
        old = any(p['role'] == 'old_masks' for p in state)
        assert not (old and new), 'Review ambiguous initial mask activity'
        plant = index == 22 or 54 <= index <= 93 or 111 <= index <= 123
        if index <= 48:
            kind = 'foliage_fragment' if index == 22 else 'tree_wood'
        elif plant:
            kind = 'grass' if 111 <= index <= 116 else 'fern' if 117 <= index <= 123 else 'understory_foliage'
        elif 128 <= index <= 135:
            kind = 'animated_canopy'
        elif state:
            kind = 'patch_state_graphic'
        elif index in (49, 50, 51, 52, 53, 125, 126):
            kind = 'rock_or_root_bank'
        else:
            kind = 'scenery_prop'
        existing = sorted(owner[index])
        status = 'existing_group' if existing else 'pending_authored_foliage' if plant else 'pending_fence' if index in (94, 95) else 'owner_reconciliation'
        if index in (65, 66, 81) and not existing:
            status = 'isolated_foliage_candidate'
        x, y = native_row['box_top_left']; w, h = native_row['box_size']
        record = dict(native_mask=index, layer=native_row['layer'], layer_index=native_row['layer_index'],
            mask_type=native_row['mask_type'], bbox=[x, y, w, h], kind=kind,
            initial_active=not new, state_controlled=bool(state), patch_roles=state,
            existing_groups=existing, ownership_status=status,
            missing_foliage_domain=plant and not existing,
            map_boundary_sides=[side for side, condition in [('west',x <= 0),('north',y <= 0),('east',x+w >= 1791),('south',y+h >= 1151)] if condition],
            note=NOTES.get(index, 'Existing visual owner; geometry, texture and state validation remain separate.' if existing else 'Source cutout is vegetation. Authored source domain, full-depth geometry and joint/context review remain pending.'),
            native_png_sha256=sha(manifest_path.parent/native_row['png']))
        if index in (57, 60):
            record['existing_partial_groups'] = ['croisement02-west-rock-outcrop']
            record['pending_complement_domain'] = 351 if index == 57 else 352
        if index in (65, 66, 81):
            record['candidate_domain'] = {65:415,66:416,81:414}[index]
            worker = out / f'understory-round-1/assets/croisement02-shrub-{index:02}'
            record['candidate_worker'] = str(worker)
            record['candidate_model_sha256'] = sha(worker/'model.blend') if (worker/'model.blend').exists() else None
        if index in (94, 95):
            record['reserved_authored_domain'] = index + 336
        records.append(record)
    assert len(records) == len(ids)
    counts = Counter(r['ownership_status'] for r in records)
    missing = [r['native_mask'] for r in records if r['missing_foliage_domain']]
    evidence_paths = [out/'animation-references/composite-frame-0.png',
        out/'authored-stem-integration/source-classifications.json',
        out/'scenery-round-1/assets/croisement02-woodcutters-shed/inspection/refinement.json',
        out/'scenery-round-1/assets/croisement02-woodcutters-shed/inspection/actual-materials/sheet.png',
        *[out/f'understory-review/{name}' for name in ('masks-54-73.png', 'masks-74-93.png', 'masks-94-123.png', 'masks-124-141.png', '47-48-gameplay-layer-source.png')]]
    report = dict(version=1, map='Croisement02', purpose='Exhaustive native-mask ownership and geometry backlog; does not grant any approval.',
        inputs={str(p.relative_to(out)): sha(p) for p in (manifest_path, level_path, catalog_path, canopy_path)},
        classification_evidence={str(p.relative_to(out)): sha(p) for p in evidence_paths},
        summary=dict(native_masks=len(records), active_catalog_groups=len(groups), initially_active=sum(r['initial_active'] for r in records),
            applied_only=sum(not r['initial_active'] for r in records), state_controlled=sum(r['state_controlled'] for r in records),
            ownership_status_counts=dict(sorted(counts.items())), missing_foliage_domains=len(missing), missing_foliage_native_masks=missing,
            geometry_completion_is_not_asset_count=True),
        limits=[
            'Native masks and native sight obstacles are different namespaces; no mask index is fabricated as a sight obstacle.',
            'A pending source domain may join another clump or require mixed-material splitting; pending mask count is not a future object count.',
            'Existing ownership is not a texture or geometry approval. Approved foliage55/58/59 remains unchanged while row cleanup is held.',
            'Applied-only patch masks are state assets, never permanent scenery. Initial old-state masks136/137 disappear when their patches apply.',
            'This mask census does not cover unmasked painted remnants, whole-map terrain repairs, animation phase completion, export or publication checks.',
        ], records=records)
    return report


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, default=OUT/'understory-review/native-mask-backlog.json')
    args = parser.parse_args()
    report = audit(OUT)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report['summary'], indent=2))
