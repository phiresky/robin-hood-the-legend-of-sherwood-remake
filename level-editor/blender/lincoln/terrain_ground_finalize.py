"""Bind the Lincoln terrain review records to the current model and packet (system python).

    python3 level-editor/blender/lincoln/terrain_ground_finalize.py [<workspace>]

Writes source-coverage-audit.json and candidate.json from the saved workspace
evidence (coverage-ray-audit.json, ground-domain-ground.json, geometry-recipe.json,
validation.json). review.md must already exist. Fails if any evidence is stale.
"""
import hashlib
import json
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1] / 'work/needs-review/lincoln-refinement'
WORKSPACE = ROOT / 'round-4/assets/lincoln-terrain'


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main():
    ws = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else WORKSPACE
    read = lambda name: json.loads((ws / name).read_text())
    model, views = sha(ws / 'model.blend'), sha(ws / 'modified/views.json')
    rays = read('inspection/coverage-ray-audit.json')
    domain = read('inspection/ground-domain-ground.json')
    geometry = read('geometry-recipe.json')
    validation = read('validation.json')
    if rays['model_sha256'] != model or rays['modified_views_sha256'] != views:
        raise ValueError('Coverage ray audit is stale')
    if validation['status'] != 'PASS' or validation['model_sha256'] != model:
        raise ValueError('Validation is stale or failed')
    if not (ws / 'review.md').read_text().strip():
        raise ValueError('review.md missing')
    counts = rays['counts']
    exact = counts.get('ground-visible-exact', 0)
    other = {k: v for k, v in counts.items() if k != 'ground-visible-exact'}
    foliage = {k: v for k, v in other.items() if k.startswith(('blocked:foliage-', 'blocked:scenery-'))}
    rest = {k: v for k, v in other.items() if k not in foliage}
    # Known exceptions: the documented footbridge boundary pixel and up to two BVH edge misses.
    if set(rest) - {'blocked:building-045', 'blocked:none'} or rest.get('blocked:building-045', 0) > 1 \
            or rest.get('blocked:none', 0) > 2:
        raise ValueError(f'Unexpected coverage failures: {rest}')
    modified = read('modified/views.json')
    evidence_files = ['inspection/coverage-ray-audit.json', 'inspection/coverage-audit-failures.png',
                      'inspection/ground-domain-ground.json', 'inspection/ground-domain-ground.png',
                      'inspection/ground-source-domain-full.png', 'inspection/source-comparison.png',
                      'inspection/footbridge-source-camera.png', 'inspection/footbridge-oblique.png',
                      'geometry-recipe.json', 'validation.json', 'terrain-packet.json', 'source-masks.json',
                      'projection/ground-material.json']
    evidence_files += sorted('inspection/' + p.name for p in (ws / 'inspection').glob('ground-domain-ground-evidence-*.png'))
    limitations = [
        'Bed depth (world Z -58, native -47.5) comes from the approved footbridge pier feet (world Z -60) and the '
        'painted waterline at the pier; it is not measured bathymetry, and the whole course keeps that one level.',
        'Banks are an inferred 1:1 native slope (edge-on to the source camera on the south side). The painted '
        'north-bank bands are narrower in places than the geometry implies; the source view is exact by construction, '
        'other azimuths show the inferred bank.',
        'The heightfield is sampled on a 3-unit native grid; steep abutment-bank transitions and bank crests show '
        'small facets/serrations in oblique views.',
        'The channel is open at the top-map tongue (native y -60) and at the west map edge.',
        'Terrain beneath foreground scenery and all back-facing banks remain unknown (neutral); no generated textures.',
        'Unmasked foliage painted over the ground stays in the domain (listed in ground-domain-ground.json for the tree/bush lane).',
        'Omitted painted props carved out of the domain (landing stage and mooring pole, plank, tool, log, two poles, '
        'stake) have no 3D node or native mask and stay neutral; the landing stage needs an asset.',
        'One boundary pixel (648, 673) is blocked by the footbridge node 045 in the BVH ray audit although the '
        'z-buffer gives the ground; it stays in the domain and is neutral in the packet.',
        'The village stream punt (building-051) still sits at z 0 and now floats about 47 native units above the '
        'channel water; that approved asset needs reseating on the bed.',
        'Authored ground mask 454 lives in this workspace (source-domain/manifest.json) until the coordinator folds it into mask v5.',
        'Static covered (exterior) state only.']
    audit = {'version': 1, 'status': 'PASS', 'asset_id': 'lincoln-terrain', 'model_sha256': model,
             'modified_views_sha256': views, 'inspected_views': list(range(8)),
             'source_sha256': domain['source_sha256'],
             'expected_source_pixels': rays['expected_source_pixels'], 'ray_verified_exact': exact,
             'ray_exceptions': other,
             'occluded_by_foliage_assets': {'pixels': sum(foliage.values()), 'by_node': foliage,
                 'note': ('Ground-domain pixels where an approved foliage/scenery asset mesh is physically in front '
                          'of the ground at the source camera (card volume beyond its painted domain, trunks in '
                          'native canopy-mask holes). They stay neutral on the ground; ownership of their artwork '
                          'is a foliage-lane decision (extend the foliage domain or add a ground occluder constraint).')}, 'atlas_observed_texels': rays['atlas_observed_texels'],
             'known_review_pixels': sum(v['counts']['source'] for v in modified['views']),
             'domain_statistics': domain['statistics'],
             'method': ('Independent domain review against the artwork (overview, stream/village, southwest road and '
                        'keep-north sheets with removed scenery and carve-outs marked), then an exhaustive ray audit: '
                        'every authored ground-domain pixel is cast into the complete saved scene with receiver-specific '
                        'mask eligibility, must hit the saved ground first, reproject into the same pixel and read an '
                        'observed packed atlas texel with the exact source RGB. All eight solid and textured modified '
                        'views and the saved-material source comparison were inspected.'),
             'observation': ('The ground was previously a flat plane with no reviewed domain, so the stream water, '
                             'village earth, fields, paths and grass had no receiver. The authored domain covers '
                             f"{domain['statistics']['domain']:,} source pixels: all painted ground where the refined "
                             'ground is the first hit, minus every native scenery silhouette, all other authored '
                             'domains, omitted props and the map-edge plateau gaps. Substantial neutral areas are '
                             'foreign ownership (buildings, trees, bushes, fences, rocks, footbridge), back-facing '
                             'banks, and ground hidden under scenery.'),
             'evidence': {p: sha(ws / p) for p in evidence_files},
             'limitations': limitations}
    (ws / 'source-coverage-audit.json').write_text(json.dumps(audit, indent=2) + '\n')
    candidate = {
        'version': 1, 'asset_id': 'lincoln-terrain', 'status': 'ready-for-user',
        'geometry_refined': True, 'geometry_reviewed': True, 'inspected_views': list(range(8)),
        'recipe': str(HERE / ('refold_terrain_masks.py' if read('workspace.json').get('previous_workspace') else 'prepare_terrain.py')),
        'geometry_recipe': str(HERE / 'terrain_ground.py'),
        'domain_recipe': str(HERE / 'terrain_ground_domain.py'),
        'audit_recipe': str(HERE / 'terrain_ground_audit.py'),
        'model_sha256': model, 'modified_views_sha256': views,
        'ground_z': 'native z 0 (village and outer ground); stream bed world Z -58 (native -47.5)',
        'changes': [
            'Cut the painted stream channel into the flat ground along its whole visible course: north map edge, '
            'the bend, the footbridge, the pond and the west map edge. Painted water becomes the flat bed at world '
            'Z -58; its native footprint is the traced painted water shifted by the bed depth.',
            'Banks rise at 1:1 native slope to z 0; under the village footbridge the bed spans the abutments with '
            'vertical channel walls, so the approved pier and arches stand in the channel and are visible from the '
            'source camera. The pier/arch feet (world Z -60) are seated 2 units into the bed.',
            'Village-lane follow-up: the bank stays at z 0 against both abutment faces and under the deck ramps '
            '(bridge frame x -35..13 / 155..191, y -7..87, also over painted water there) and falls off at 4:1, so '
            'the channel is open only between the abutments and no trench exposes the bridge body.',
            f"Heightfield terrain of {geometry['vertices']} vertices / {geometry['faces']} faces; the rest of the "
            'map stays at z 0, plus a tongue to native y -60 so the channel reaches the top source rows.',
            f"Authored reviewed ground receiver domain ({domain['statistics']['domain']:,} px, mask 454) and the "
            'saved observed-source ground material (UV = source pixel projection).'],
        'limitations': limitations,
        'source_comparison': 'inspection/source-comparison.png',
        'source_trace': 'inspection/ground-domain-ground-evidence-stream-village.png',
        'projection_errors': 'inspection/footbridge-source-camera.png',
        'source_coverage_audit': 'source-coverage-audit.json',
        'geometry_approval': 'pending', 'texture_generation': 'not-started'}
    config = read('workspace.json')
    if config.get('previous_workspace'):
        # Mask-manifest refold: same reviewed geometry and domain, new ground-row exclusions.
        excluded = rays.get('excluded_domain_pixels', 0)
        fold = (f"Foliage fold-in: the ground row keeps authored domain {config['authored_ground_mask_index']} and "
                f"now excludes the approved foliage asset domains {rays.get('ground_row_exclusions')} "
                f"({excluded:,} px leave the ground receiver). Geometry, UVs, framing and the authored domain are "
                f"unchanged from {config['previous_workspace']} (model {config['previous_model_sha256'][:12]}); the "
                'context scene now includes the foliage assets as occluders.')
        candidate['changes'] = [fold] + candidate['changes']
        if foliage:
            note = (f'{sum(foliage.values()):,} ground-domain px are occluded at the source camera by approved '
                    'foliage asset meshes (beyond their painted domains or trunks in canopy-mask holes); they stay '
                    'neutral on the ground until the foliage lane owns them.')
            candidate['limitations'].append(note)
            audit['limitations'].append(note)
        audit['observation'] = fold + ' ' + audit['observation']
        stale = 'Authored ground mask 454 lives in this workspace'
        for record in (candidate, audit):
            record['limitations'] = [l for l in record['limitations'] if not l.startswith(stale)] + [
                f"Mask manifest: {config['source_mask_parent_manifest']} (sha {config['source_mask_parent_sha256'][:12]})."]
            record['limitations'] = [l.replace('(listed in ground-domain-ground.json for the tree/bush lane)',
                                               '(the five flagged bushes are now foliage assets and excluded)')
                                     for l in record['limitations']]
        candidate['previous_workspace'] = config['previous_workspace']
        (ws / 'source-coverage-audit.json').write_text(json.dumps(audit, indent=2) + '\n')
    (ws / 'candidate.json').write_text(json.dumps(candidate, indent=2) + '\n')
    print(json.dumps({'model_sha256': model, 'modified_views_sha256': views, 'audit': 'PASS'}))


if __name__ == '__main__':
    main()
