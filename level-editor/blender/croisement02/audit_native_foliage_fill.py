"""Adversarial local fill check: unknown RGB may change, native fronts may not."""
import json,sys,hashlib
from pathlib import Path
import bpy,numpy as np
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender'),str(Path(__file__).parent)]
from catalog import tree_workspace
from fill_physical_foliage import fill
from render_slots import acquire,release
from review_evidence import sha


def snapshot(objects):
    result={}
    for obj in objects:
        for index,material in enumerate(obj.data.materials):
            if not material or not material.get('foliage_observed'):continue
            image=next(n.image for n in material.node_tree.nodes if n.type=='TEX_IMAGE')
            pixels=np.empty(len(image.pixels),np.float32);image.pixels.foreach_get(pixels)
            result[(obj.name,index)]=(material,image,pixels.copy())
    return result

def main(number,output):
    worker=tree_workspace(number);digest=sha(worker/'model.blend');acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(worker/'model.blend'))
        objects=[o for o in bpy.data.objects if o.type=='MESH' and o.get('asset_group')==f'croisement02-tree-{number:02d}']
        if not objects:objects=[o for o in bpy.data.objects if o.type=='MESH' and f'Tree {number:02d} /' in o.name]
        protected=snapshot(objects);assert protected
        calls=[]
        def sample(obj,normal,positions,accepted,colors,*,face_index,**kwargs):
            face=obj.data.polygons[face_index];mat=obj.data.materials[face.material_index]
            assert not mat.get('foliage_observed'),'Attempt to fill approved observed front'
            assert all(obj.data.color_attributes['Source ownership'].data[i].color[0]==0 for i in face.loop_indices)
            colors[:,:3]=[1,0,1];calls.append(len(colors));return np.ones(len(colors),bool)
        reports=fill(objects,sample,None,'adversarial-local-test-no-api')
        assert calls and sum(r['generated'] for r in reports)>0
        after=snapshot(objects);assert set(protected)==set(after)
        for key,(material,image,pixels) in protected.items():
            newmaterial,newimage,newpixels=after[key]
            assert newmaterial==material and newimage==image and np.array_equal(pixels,newpixels)
        assert sha(worker/'model.blend')==digest
        report=dict(status='PASS',model_sha256=digest,scope='In-memory adversarial magenta fill; no API request and no model save',protected_front_materials=len(protected),protected_front_rgb_and_alpha_changed=0,observed_front_callback_calls=0,unknown_samples=sum(calls),fill_reports=reports)
        output.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report,indent=2))
    finally:release()

if __name__=='__main__':
    number,path=sys.argv[sys.argv.index('--')+1:];main(int(number),Path(path))
