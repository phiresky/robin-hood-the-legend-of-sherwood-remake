"""Separate the foreground kindling from tree 18 in a new frozen worker."""
import json
import uuid
import sys
from pathlib import Path
import bpy
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent));sys.path.insert(0,str(ROOT/'level-editor/refinement'));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from refinement_workspace import prepare,validate,modified
from tree_geometry import wood_geometry,replace_mesh
from bark_materials import fill
from audit_candidates import audit
from render_tree import render_workspace


def main():
    asset='croisement02-tree-18';old=OUT/'forest-v4-round-1/assets'/asset;w=OUT/'forest-v4-round-2/assets'/asset;domain=OUT/'tree18-source-revision';receipt=w/'inspection/source-domain-revision.json'
    latest={r['asset_id']:r for r in json.loads((OUT/'user-feedback.json').read_text())['records']}
    if latest.get(asset,{}).get('decision')=='approved':raise ValueError('Approved geometry is frozen')
    try:
        acquire()
        if not (w/'workspace.json').exists():
            bpy.ops.wm.open_mainfile(filepath=str(old/'model.blend'));bpy.context.preferences.filepaths.save_version=0
            prepare(w,asset_id=asset,scene_name='Croisement02 Refinement',collection_name='Croisement02 Working',source_path=OUT/'animation-references/composite-frame-0.png',grouping_manifest=OUT/'catalog.json',inventory_path=OUT/'forest-v4-inventory/inventory.json',review_path=OUT/'forest-v4-grouping-review.json',source_mask_manifest=domain/'assignments.json',width=256,height=256,framing_padding=1.35,lighting=dict(toward_sun=[-.45,-.55,.70],ambient=.22,diffuse=.78,shadow_epsilon=.05))
        if receipt.exists() and '--redo' in sys.argv:receipt.rename(receipt.with_name('source-domain-revision-archive-'+uuid.uuid4().hex[:8]+'.json'))
        if not receipt.exists():
            bpy.ops.wm.open_mainfile(filepath=str(w/'model.blend'));bpy.context.preferences.filepaths.save_version=0;validate(w)
            objects=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o.get('asset_group')==asset]
            wood=[o for o in objects if o.get('projection_component')!='crown']
            if len(wood)!=1:raise ValueError('Expected one wood part')
            row=next(r for r in json.loads((OUT/'forest-v4-sources/manifest.json').read_text()) if r['mask']==18)
            trace=json.loads((domain/'trace.json').read_text())
            paths=[p for p in trace['paths'] if not (max(v[0] for v in p)<1600 and min(v[1] for v in p)>210)]
            vertices,faces=wood_geometry(paths,row['ground_y'])
            geometry=replace_mesh(wood[0],vertices,faces,materials=list(wood[0].data.materials));geometry['source_node']=wood[0]['source_node']
            for face in wood[0].data.polygons:face.use_smooth=len(face.vertices)==4
            (w/'inspection').mkdir(exist_ok=True)
            write_json(w/'inspection/bark-donor-selection.json',dict(native_mask=18,source_box=[1620,150,1634,185],source_sha256=sha(w/'reference/source.png'),reviewer='Codex',notes='Inspected enlarged native trunk crop. This broad bark strip lies above the foreground kindling; the previous sample lay on the sticks.'))
            modified(w);bark=fill(w,objects,18,receiver_only=True);validate(w);bpy.ops.wm.save_as_mainfile(filepath=str(w/'model.blend'))
            report=json.loads((old/'inspection/refinement.json').read_text());report.update(model_sha256=sha(w/'model.blend'),wood=[geometry],bark=bark,wood_domain_mask=301)
            report['limitations'].append('Foreground kindling domain 300 is excluded from wood geometry and bark ownership. Domain 301 records the corrected silhouette; hidden trunk completion remains inferred.')
            write_json(w/'inspection/refinement.json',report);audit(w)
            write_json(receipt,dict(model_sha256=report['model_sha256'],previous_model_sha256=sha(old/'model.blend'),source_mask=301,excluded_foreground=300,omitted_kindling_edge_paths=len(trace['paths'])-len(paths),trace_sha256=sha(domain/'trace.json')))
        elif json.loads(receipt.read_text())['model_sha256']!=sha(w/'model.blend'):raise ValueError('Revised tree changed')
        render_workspace(w,256,release_slot=False)
    finally:release()

if __name__=='__main__':main()
