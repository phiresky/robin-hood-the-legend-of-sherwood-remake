"""Fresh actual-material and source receipts for the additive boundary package."""
import json
import sys
from pathlib import Path
import bpy
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from render_tree import render_workspace


def main():
    worker=OUT/'restart2-vegetation/shrub93-package-v3/assets/croisement02-shrub-93'
    if '--validate-only' in sys.argv:
        from refinement_workspace import validate
        bpy.ops.wm.open_mainfile(filepath=str(worker/'model.blend'))
        write_json(worker/'validation.json',validate(worker))
        return
    proof=json.loads((worker/'inspection/package-preservation.json').read_text())
    raw=Path(proof['source_candidate'])
    preserved=json.loads((raw/'preservation.json').read_text())
    old=Path(preserved['original_worker'])
    assert sha(old/'model.blend')==preserved['original_model_sha256']
    report=json.loads((old/'inspection/refinement.json').read_text())
    report['model_sha256']=sha(worker/'model.blend')
    report['status']='Preserved original three foliage volumes with separate25 inferred boundary fragments; independent candidate review pending'
    report['boundary_addition']=dict(preservation=str(raw/'preservation.json'),preservation_sha256=sha(raw/'preservation.json'),source_role_mask=6003,pixels=25)
    report['limitations']+=['Original observed503 unchanged; separate25 inferred boundary6003 pixels use native RGB.','New fragments use nearby inferred leaf depth and .05 source-ray plane separation; one fragment is positioned ahead of the existing tree35 root.','Prior geometry approval does not transfer to this derivative.']
    write_json(worker/'inspection/refinement.json',report)
    if '--classify-only' in sys.argv:
        bpy.ops.wm.open_mainfile(filepath=str(worker/'model.blend'))
    else:
        render_workspace(worker,384,release_slot=False,transparent_bounces=256)
    path=worker/'inspection/actual-materials/opacity-bounds.json'
    bounds=json.loads(path.read_text())
    objects=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o.get('asset_group')==worker.name and o.get('projection_component')=='crown']
    assert len(objects)==len(bounds['crowns'])==4
    pairs=list(zip(objects,bounds['crowns']))
    bounds['crowns']=[row for obj,row in pairs if not obj.get('inferred_boundary_domain')]
    bounds['attached_boundary_fragments']=[dict(object=obj.name,source_domain=obj['inferred_boundary_domain'],**row)for obj,row in pairs if obj.get('inferred_boundary_domain')]
    assert len(bounds['crowns'])==3 and len(bounds['attached_boundary_fragments'])==1
    bounds['scope']='Three preserved full-volume foliage lobes. Sparse25 source-boundary attachments are measured separately, not treated as a fourth independent tree/shrub volume.'
    write_json(path,bounds)

if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
