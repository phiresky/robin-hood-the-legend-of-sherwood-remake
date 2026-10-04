"""Add reviewed supplemental stems as authored source parts, never obstacles."""
import json
import sys
from pathlib import Path
import bpy
import numpy as np
from PIL import Image

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(Path(__file__).parent))
sys.path.insert(0, str(ROOT/'level-editor/refinement'))
sys.path.insert(0, str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT, reviewed_catalog
from catalog_schema import source_for_part
from evidence_io import sha, write_json, digest
from render_slots import acquire, release
from refinement_inventory import inventory, validate_catalog
from refinement_workspace import prepare, modified, validate
from audit_candidates import audit
from render_tree import render_workspace

DIRECTORY = OUT/'authored-stem-integration'


def geometry(obj):
    return digest(dict(vertices=[list(v.co) for v in obj.data.vertices],
                       faces=[list(p.vertices) for p in obj.data.polygons],
                       transform=[list(row) for row in obj.matrix_world]))


def source_domains(catalog):
    source_inventory = OUT/'forest-v4-mask-inventory.json'
    native = json.loads(source_inventory.read_text())
    rows = {r['index']: r for r in native['masks']}
    def bitmap(index):
        row = rows[index]; x,y = row['box_top_left']; w,h = row['box_size']
        result = np.zeros((1152,1792),bool)
        result[y:y+h,x:x+w] = np.asarray(Image.open(source_inventory.parent/row['png']).convert('L'))>0
        return result
    for row in native['masks']:
        if row.get('png'):row['png']=str((source_inventory.parent/row['png']).resolve())
    masks = json.loads((OUT/'forest-v4-round-1/assets/croisement02-tree-08/source-masks.json').read_text())
    projection = masks['projections']['exterior']
    domains = {}
    for stem, exclusions, domain_index in [(9,[133],400),(44,[89,128],401)]:
        own = bitmap(stem)
        for exclusion in exclusions:own &= ~bitmap(exclusion)
        path = DIRECTORY/f'domain-{domain_index}.png'
        Image.fromarray(own.astype('uint8')*255).save(path)
        native['masks'].append(dict(index=domain_index,layer=0,png=str(path),box_top_left=[0,0],box_size=[1792,1152],
            provenance=f'Authored visible stem domain: native{stem} minus reviewed foreground{exclusions}'))
        node=f'foliage-wood-{stem:03}'
        projection['assignments'].append(dict(source_node=node,mask_indices=[domain_index],reviewed=True))
        domains[node]=dict(mask_index=domain_index,pixels=int(own.sum()),sha256=sha(path),excluded_foreground=exclusions)
    ground_path=DIRECTORY/'ground-extent.png'
    Image.new('L',(1792,1152),255).save(ground_path)
    native['masks'].append(dict(index=402,layer=0,png=str(ground_path),box_top_left=[0,0],box_size=[1792,1152],
        provenance='Source image extent for future terrain receiver; not an assertion of ground ownership'))
    if any(a.get('source_node')=='ground' for a in projection['assignments']):raise ValueError('Ground assignment requires explicit merge')
    projection['assignments'].append(dict(source_node='ground',mask_indices=[402],exclude_mask_indices=[400,401],
        reviewed=True,exclusions_reviewed=True,exclusion_reason='These two visible wood domains belong to authored stems. Full terrain foreground cleanup remains pending.'))
    receivers={'ground',*(source_for_part(p) for g in catalog['groups'] for p in g['parts'])}
    projection.setdefault('occluder_constraints',[]).extend(dict(reviewed=True,source_node=node,
        receiver_nodes=sorted(receivers-{node}),mask_indices=[record['mask_index']],
        reason='Authored stem can occlude foreign source receivers only inside its observed wood domain; hidden continuation must not erase painted ground.',
        review_evidence=str(OUT/'missing-wood-review/ownership-proposal.json')) for node,record in domains.items())
    write_json(DIRECTORY/'mask-inventory.json',native)
    masks['mask_inventory']=str(DIRECTORY/'mask-inventory.json')
    write_json(DIRECTORY/'source-masks.json',masks)
    write_json(DIRECTORY/'source-domains.json',dict(status='reviewed authored stem domains; other foreground cleanup remains pending',
        domains=domains,source_sha256=sha(OUT/'animation-references/composite-frame-0.png'),
        ground_assignment_note='Manifest prepared for fresh terrain work; existing approved ground is not silently rebaked.'))


def main():
    DIRECTORY.mkdir(exist_ok=False)
    catalog_path=reviewed_catalog(); catalog=json.loads(catalog_path.read_text())
    if any(p.get('node','').startswith('foliage-wood-') for g in catalog['groups'] for p in g['parts']):
        raise ValueError('Authored stems already integrated')
    write_json(DIRECTORY/'previous-catalog.json',catalog)
    for stem in (9,44):
        catalog['groups'].append(dict(id=f'croisement02-supplemental-wood-{stem:02}',
            name='Thin Companion Stem 09' if stem==9 else 'Southern Forked Stem 44',
            authored_scenery=True,native_wood_mask=stem,
            classification='Standalone authored stem; no separate crown or native sight obstacle',
            parts=[dict(node=f'foliage-wood-{stem:03}',name=f'Native stem {stem:02}',foliage_domain_mask=400 if stem==9 else 401)]))
    catalog['version']=2
    catalog['canonical_owners']={source_for_part(p):g['id'] for g in catalog['groups'] for p in g['parts']}
    if len(catalog['canonical_owners'])!=152:raise ValueError('Expected 150 native and two authored parts')
    catalog['review_notes']+=' Authored bare stems09/44 have independent source nodes. Mask22 is a northern foliage fragment, not automatically a missing trunk.'
    write_json(DIRECTORY/'catalog.json',catalog)
    bpy.ops.wm.open_mainfile(filepath=str(OUT/'forest-v4-input.blend'))
    bpy.context.preferences.filepaths.save_version=0
    collection=bpy.data.collections['Croisement02 Working']
    source_models=[]; imported=[]
    for stem in (9,44):
        worker=OUT/f'missing-wood-round-1/assets/croisement02-supplemental-wood-{stem:02}'
        saved=json.loads((worker/'inspection/saved-model-audit.json').read_text())
        model=worker/'model.blend'; assert saved['model_sha256']==sha(model)
        with bpy.data.libraries.load(str(model),link=False) as (source,target):
            target.objects=[r['object'] for r in saved['objects']]
        for obj in target.objects:collection.objects.link(obj)
        bpy.context.view_layer.update()
        before=[]
        for obj in target.objects:
            signature=geometry(obj); matrix=obj.matrix_world.copy();obj.parent=None;obj.matrix_world=matrix
            assert geometry(obj)==signature
            before.append(dict(source_node=obj['source_node'],geometry_sha256=signature))
            imported.append(obj)
        source_models.append(dict(worker=str(worker),model_sha256=sha(model),objects=before))
    groups={g['id']:g for g in catalog['groups']}
    for obj in collection.all_objects:
        if obj.type=='MESH' and obj.get('source_node') in catalog['canonical_owners']:
            group=groups[catalog['canonical_owners'][obj['source_node']]]
            obj['asset_group']=group['id'];obj['asset_name']=group['name']
    # Inventory hidden state metadata too; visibility is restored immediately.
    visibility={obj:obj.hide_render for obj in collection.all_objects if obj.type=='MESH'}
    for obj in visibility:obj.hide_render=False
    inventory(DIRECTORY/'inventory',collection_name='Croisement02 Working',map_name='Croisement02',
        source_path=OUT/'animation-references/composite-frame-0.png',patch_manifest=OUT/'source-states/layers.json')
    for obj,state in visibility.items():obj.hide_render=state
    validate_catalog(DIRECTORY/'inventory/inventory.json',DIRECTORY/'catalog.json')
    write_json(DIRECTORY/'grouping-review.json',dict(status='reviewed',reviewer='Codex',catalog_sha256=sha(DIRECTORY/'catalog.json'),
        inventory_sha256=sha(DIRECTORY/'inventory/inventory.json'),previous_catalog_sha256=sha(DIRECTORY/'previous-catalog.json'),
        evidence=['All150 native parts retain current ownership, including37 in west rocks; two authored source nodes added.',
                  'Standalone stems09/44 reviewed jointly with approved07/08/45. No new crown pixels assigned.',
                  'Mask22 is a northern source foliage fragment; a wood asset is not justified by the visible artwork.']))
    source_domains(catalog)
    write_json(DIRECTORY/'source-classifications.json',dict(native_mask_22=dict(
        source_classification='northern boundary foliage fragment',
        evidence='Individual native mask22 and source crop visually inspected. Visible pixels are leaves; no exposed bark establishes a separate trunk.',
        geometry_status='unmodeled foliage ownership; not counted as a missing trunk',
        mask_sha256=sha(OUT/'baseline/masks/000022.png'),
        source_sha256=sha(OUT/'animation-references/composite-frame-0.png'))))
    bpy.ops.wm.save_as_mainfile(filepath=str(DIRECTORY/'input.blend'))
    records=[]
    for stem in (9,44):
        bpy.ops.wm.open_mainfile(filepath=str(DIRECTORY/'input.blend'))
        worker=OUT/f'authored-stems-round-1/assets/croisement02-supplemental-wood-{stem:02}'
        old=OUT/f'missing-wood-round-1/assets/{worker.name}'
        prepare(worker,asset_id=worker.name,scene_name='Croisement02 Refinement',collection_name='Croisement02 Working',
            source_path=OUT/'animation-references/composite-frame-0.png',grouping_manifest=DIRECTORY/'catalog.json',
            inventory_path=DIRECTORY/'inventory/inventory.json',review_path=DIRECTORY/'grouping-review.json',
            source_mask_manifest=DIRECTORY/'source-masks.json',width=256,height=384,framing_padding=1.25)
        modified(worker)
        bpy.ops.wm.save_as_mainfile(filepath=str(worker/'model.blend'))
        inspection=worker/'inspection';inspection.mkdir(exist_ok=True)
        report=json.loads((old/'inspection/refinement.json').read_text())
        report.update(model_sha256=sha(worker/'model.blend'),status='authored standalone stem candidate; independent review pending',
            limitations=['Standalone bare stem; no separate crown is claimed or assigned.',
                         'Unknown bark remains neutral in source-only materials; texture fill requires geometry approval.',
                         'Source camera silhouette and joint neighbouring canopy views are reviewed independently.',
                         'The native map has no sight obstacle for this authored source node.',
                         'Terrain exclusion and scoped occluder rules are prepared; full terrain refresh and mission integration remain pending.'])
        if stem==44:report['limitations'].append('Base continues beyond the southern map edge; smoothed branch joins remain visible when surrounding canopy is hidden.')
        else:report['limitations'].append('Side views show a slender bare stem beside07/08; the native unbranched outline does not establish a separate leafy crown.')
        write_json(inspection/'refinement.json',report)
        audit(worker);render_workspace(worker,256,release_slot=False)
        objects=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o.get('asset_group')==worker.name]
        expected=next(r['objects'] for r in source_models if Path(r['worker']).name==worker.name)
        actual=[dict(source_node=o['source_node'],geometry_sha256=geometry(o)) for o in objects]
        if actual!=expected:raise ValueError('Fresh ownership preparation changed stem geometry')
        receipt=dict(model_sha256=sha(worker/'model.blend'),previous_worker=str(old),previous_model_sha256=sha(old/'model.blend'),
            geometry_preserved=True,objects=actual,catalog_sha256=sha(DIRECTORY/'catalog.json'),source_masks_sha256=sha(worker/'source-masks.json'))
        write_json(inspection/'authored-integration.json',receipt);records.append(receipt)
    # Install the canonical ownership only after both fresh workers validate.
    write_json(OUT/'ownership-revision/catalog.json',catalog)
    write_json(OUT/'ownership-revision/grouping-review.json',json.loads((DIRECTORY/'grouping-review.json').read_text()))
    write_json(DIRECTORY/'integration.json',dict(status='canonical ownership prepared; manual gallery readiness pending',
        catalog_sha256=sha(DIRECTORY/'catalog.json'),inventory=str(DIRECTORY/'inventory/inventory.json'),
        native_parts=150,authored_parts=2,groups=len(catalog['groups']),workers=records,source_models=source_models))
    print('INTEGRATED',len(catalog['groups']),'groups; two authored stems')


if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
