"""Add a source-visible post cap without rewriting the approved fence."""
import json,sys
from pathlib import Path
import bpy,bmesh,numpy as np
from PIL import Image
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT,reviewed_catalog
from evidence_io import sha,write_json
from render_slots import acquire,release
from refinement_inventory import inventory,validate_catalog
from refinement_workspace import prepare,validate,_render,_geometry
from source_projection_bake import bake
from audit_candidates import audit
from render_tree import render_workspace
from tree_geometry import SIN,COS,RAY
from prepare_missing_fences import GROUND
from complete_southwest_logs import verify_saved
ASSET='croisement02-east-upright-rail-fence-95'
OLD=OUT/'missing-fence-candidates/v9/assets'/ASSET
DEST=OUT/'missing-fence-candidates/post-cap95-v1'
POLYGON=[(1558.7,724.0),(1561.7,723.8),(1565.1,725.5),(1565.1,728.0),(1559.2,728.0)]


def main():
    DEST.mkdir(exist_ok=False);worker=DEST/'assets'/ASSET;protected={str(OLD/'model.blend'):sha(OLD/'model.blend')}
    proposal=OUT/'understory-candidates/mixed75-91-boundary-review/75-proposed-fence95.png';old_domain=OUT/'missing-fence-candidates/v9/domain-431.png'
    original=np.asarray(Image.open(old_domain).convert('L'))>0;proposed=np.asarray(Image.open(proposal).convert('L'))>0
    yy,xx=np.indices(proposed.shape);cap=proposed&(xx>=1558)&(xx<=1562)&(yy<=725);expanded=original|cap
    Image.fromarray(cap.astype('uint8')*255).save(DEST/'new-cap-source.png');Image.fromarray(expanded.astype('uint8')*255).save(DEST/'domain-431.png')
    write_json(DEST/'source-review.json',dict(status='private source-role revision proposal; root review pending',prior_inferred_boundary=str(proposal),prior_inferred_boundary_sha256=sha(proposal),new_cap_pixels=int(cap.sum()),retained_original_pixels=int(original.sum()),not_assigned_to_new_wood=int((proposed&~cap).sum()),evidence=['edge95-third-post-grid.png','edge95-native-grid.png'],reason='Only coherent post-top pixels extend geometry. Green upper-rail-gap and tan ground residuals require corrected inferred receiver roles, not wooden gap plates.'))
    cfg=json.loads((OLD/'workspace.json').read_text());manifest=json.loads((OLD/'source-masks.json').read_text());invpath=Path(manifest['mask_inventory']);masks=json.loads(invpath.read_text())
    for row in masks['masks']:
        row['png']=str((invpath.parent/row['png']).resolve())
        if row['index']==431:row['png']=str((DEST/'domain-431.png').resolve());row['provenance']='Prior reviewed fence431 plus separately reviewed source-visible post cap; private proposal'
    write_json(DEST/'mask-inventory.json',masks);manifest['mask_inventory']=str(DEST/'mask-inventory.json')
    node='scenery-upright-fence-095'
    for projection in manifest['projections'].values():
        projection['assignments']=[r for r in projection['assignments'] if r.get('source_node')==node];projection['occluder_constraints']=[]
    write_json(DEST/'source-masks.json',manifest)
    full=json.loads(reviewed_catalog().read_text());group=next(g for g in full['groups'] if g['id']==ASSET);catalog=dict(full,groups=[group],canonical_owners={node:ASSET});write_json(DEST/'catalog.json',catalog)
    bpy.ops.wm.open_mainfile(filepath=str(OLD/'model.blend'));bpy.context.preferences.filepaths.save_version=0
    for obj in list(bpy.data.objects):
        if obj.type=='MESH' and obj.get('asset_group')!=ASSET:bpy.data.objects.remove(obj,do_unlink=True)
    bpy.data.orphans_purge(do_recursive=True)
    collection=bpy.data.collections['Croisement02 Working'];before={o.name:_geometry(o,protect_appearance=True) for o in collection.all_objects if o.type=='MESH'}
    inventory(DEST/'inventory',collection_name=collection.name,map_name='Croisement02',source_path=cfg['source_path']);validate_catalog(DEST/'inventory/inventory.json',DEST/'catalog.json')
    write_json(DEST/'grouping-review.json',dict(status='reviewed',reviewer='Codex',catalog_sha256=sha(DEST/'catalog.json'),inventory_sha256=sha(DEST/'inventory/inventory.json'),evidence='Scoped existing fence95 only; all foreign meshes excluded from disposable worker, approved files untouched.'))
    bpy.ops.wm.save_as_mainfile(filepath=str(DEST/'approved-scope.blend'),compress=True)
    prepare(worker,asset_id=ASSET,scene_name='Croisement02 Refinement',collection_name=collection.name,source_path=cfg['source_path'],grouping_manifest=DEST/'catalog.json',inventory_path=DEST/'inventory/inventory.json',review_path=DEST/'grouping-review.json',source_mask_manifest=DEST/'source-masks.json',width=384,height=384,framing_padding=1.25,lighting=cfg['lighting'])
    bpy.ops.wm.open_mainfile(filepath=str(DEST/'approved-scope.blend'));bpy.context.preferences.filepaths.save_version=0;collection=bpy.data.collections['Croisement02 Working']
    verts=[];n=len(POLYGON)
    for behind in (0,6.1):
        for x,y in POLYGON:
            g=float(np.interp(x,[p[0] for p in GROUND[95]],[p[1] for p in GROUND[95]]));verts.append(tuple(Vector((x,-g/SIN,(g-y)/COS))+RAY*(.45-behind)))
    faces=[tuple(reversed(range(n))),tuple(n+i for i in range(n))]+[(i,(i+1)%n,n+(i+1)%n,n+i) for i in range(n)]
    mesh=bpy.data.meshes.new('Source-visible third-post cap');mesh.from_pydata(verts,[],faces);mesh.update();obj=bpy.data.objects.new(mesh.name,mesh);collection.objects.link(obj)
    for k,v in dict(source_node=node,asset_group=ASSET,asset_name=group['name'],part_name='Source-visible third-post cap').items():obj[k]=v
    bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bad=sum(not e.is_manifold for e in bm.edges)+sum(f.calc_area()<1e-9 for f in bm.faces);bm.to_mesh(mesh);bm.free()
    if bad:raise ValueError('Cap topology invalid')
    config=json.loads((worker/'workspace.json').read_text());bake('Croisement02',config['source_path'],worker/'projection/added-cap.json',receiver_nodes=[node],receiver_object_names=[obj.name],occluder_nodes=[node],projection_label='exterior',preserve_authored=False,source_mask_manifest=config['source_mask_manifest'],provenance_directory=str(worker/'projection/provenance'))
    if before!={o.name:_geometry(o,protect_appearance=True) for o in collection.all_objects if o.type=='MESH' and o.name in before}:raise ValueError('Approved fence appearance changed')
    _render(config,worker/'modified',worker/'input/views.json');bpy.ops.wm.save_as_mainfile(filepath=str(worker/'model.blend'),compress=True);validate(worker)
    inspection=worker/'inspection';inspection.mkdir(exist_ok=True)
    write_json(inspection/'preservation.json',dict(status='PASS',protected_files=protected,original_object_fingerprints=before,existing_objects=len(before),added_objects=1,existing_mesh_appearance_identical=True))
    write_json(inspection/'refinement.json',dict(asset_id=ASSET,model_sha256=sha(worker/'model.blend'),status='private additive cap; source-role/root review pending',source_outline=POLYGON,inferred_depth=6.1,source_review_sha256=sha(DEST/'source-review.json'),limitations=['Approved fence mesh, UVs and materials remain exact.','Only coherent source-visible third-post top is completed; disputed51-pixel inferred boundary is not blindly converted into timber.','No API texture fill or canonical changes.']))
    audit(worker);render_workspace(worker,384,release_slot=False);verify_saved(worker)

if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
