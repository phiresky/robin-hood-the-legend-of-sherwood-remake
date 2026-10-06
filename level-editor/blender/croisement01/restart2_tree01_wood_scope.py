"""Restrict an approved tree packet to wood without changing its cameras or model."""
import json,sys,shutil
from pathlib import Path
import bpy,numpy as np
from mathutils import Matrix,Vector
from mathutils.bvhtree import BVHTree
from PIL import Image
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from review_evidence import sha
from render_slots import acquire
R=ROOT/'level-editor/work/croisement01-refinement/restart2'
E=R/'approved-tree01-fill-v1/croisement01-tree-01/experiment'

def main():
    old=E/'wood-scope-original';old.mkdir(exist_ok=False)
    frames=json.loads((E/'views.json').read_text());preparation=json.loads((E/'preparation.json').read_text())
    names=['views.json','mask.png','preparation.json']+[v['mask'] for v in frames['views']]
    for name in names:
        target=old/name;target.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(E/name,target)
    acquire();bpy.ops.wm.open_mainfile(filepath=str(E/'approved-model.blend'))
    objects=[bpy.data.objects[name] for name in frames['object_names']]
    wood=[o for o in objects if o.get('source_node')=='building-029']
    assert len(wood)==1 and len(objects)==2
    vertices=[];faces=[];owners=[]
    for obj in objects:
        offset=len(vertices);vertices.extend(obj.matrix_world@v.co for v in obj.data.vertices)
        faces.extend(tuple(offset+i for i in f.vertices) for f in obj.data.polygons)
        owners.extend([obj.name in {o.name for o in wood}]*len(obj.data.polygons))
    bvh=BVHTree.FromPolygons(vertices,faces)
    width,height=frames['tile_size'];sheet=Image.open(E/'mask.png').convert('RGBA');counts=[]
    for view in frames['views']:
        mask=np.array(Image.open(old/view['mask']).convert('RGBA'));editable=mask[:,:,3]==0
        camera=Matrix(view['camera_matrix_world']);rotation=camera.to_3x3();direction=rotation@Vector((0,0,-1))
        scale=view['ortho_scale'];kept=0
        for row,col in zip(*np.where(editable)):
            accepted=True
            for dx,dy in [(0,0),(-.35,-.35),(-.35,.35),(.35,-.35),(.35,.35)]:
                local=Vector(((col+.5+dx-width/2)*scale/width,(height/2-row-.5-dy)*scale/width,0))
                hit=bvh.ray_cast(camera@local,direction,20000)
                if hit[2] is None or not owners[hit[2]]:accepted=False;break
            if accepted:kept+=1
            else:mask[row,col,3]=255
        image=Image.fromarray(mask);image.save(E/view['mask'])
        sheet.paste(image,(view['index']%4*width,view['index']//4*height))
        counts.append(dict(view=view['index'],original_editable=int(editable.sum()),wood_editable=kept))
    sheet.save(E/'mask.png');frames['texture_receiver_object_names']=[o.name for o in wood]
    (E/'views.json').write_text(json.dumps(frames,indent=2)+'\n')
    changed=['views.json','mask.png']+[v['mask'] for v in frames['views']]
    report=dict(status='PASS',scope='WOOD ONLY; context crown protected and excluded from bake',model_sha256=sha(E/'approved-model.blend'),receivers=frames['texture_receiver_object_names'],foreign_objects=[o.name for o in objects if o not in wood],original_files={name:sha(old/name) for name in names},scoped_files={name:sha(E/name) for name in changed},input_sha256=sha(E/'input.png'),solid_sha256=sha(E/'solid.png'),counts=counts,method='Five ray samples per formerly editable pixel must all hit approved wood first; all other pixels become protected. No model or camera mutation.')
    (E/'wood-scope.json').write_text(json.dumps(report,indent=2)+'\n')
    for name in changed:
        assert name in preparation['files'];preparation['files'][name]=sha(E/name)
    preparation['wood_scope_sha256']=sha(E/'wood-scope.json')
    (E/'preparation.json').write_text(json.dumps(preparation,indent=2)+'\n')
    print(json.dumps(report))

if __name__=='__main__':main()
