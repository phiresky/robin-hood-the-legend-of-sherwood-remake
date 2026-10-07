"""Propagate reviewed closed leaf clumps to unique native initial placements."""
from pathlib import Path
import sys,json,hashlib,math
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
P=Path(__file__).resolve().parent;sys.path.insert(0,str(P))
from restart6_source_gap_audit import OUT,RAY,SIN,COS
from leaf_state_scene_context import load_scene
from leaf_clump_support import triangle,clearance
from refinement_review import _tree
from restart4_stump_final_contact import frame,sheet
from render_slots import acquire,release
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def triangles(objects):
    result=[]
    for obj in objects:
        obj.data.calc_loop_triangles()
        for t in obj.data.loop_triangles:
            row=triangle([obj.matrix_world@obj.data.vertices[i].co for i in t.vertices],RAY,SIN,COS)
            if row is not None:row['object']=obj.name;result.append(row)
    return result

def main():
    parent=OUT/'restart11-hiding-mound/closed-small-clumps-v7';pr=json.loads((parent/'validation.json').read_text());guard=json.loads((parent/'saved-native-guard.json').read_text());assert guard['status']=='PASS'and sha(parent/'model.blend')==pr['model_sha256']==guard['model_sha256'];out=OUT/'restart15-hiding-mounds/all-placements-v1';out.mkdir(parents=True,exist_ok=False);source=json.loads((OUT/'restart9-hiding-scatter/mound-flat-v2/validation.json').read_text());rgba=np.array(Image.open(source['source']).convert('RGBA'));h,w=rgba.shape[:2];authority=json.loads((OUT/'restart9-hiding-scatter/terrain-receivers-v2/report.json').read_text());scene,static,pins,base=load_scene();ground=[o for o in static if o.name.startswith('Croisement02 Terrain')or o.get('asset_group')=='croisement02-north-woodland-bank'];wall=[o for o in static if o.get('asset_group')=='croisement02-southeast-stone-wall-and-gate'];terrain=triangles(ground);walls=triangles(wall)
    bpy.ops.wm.open_mainfile(filepath=str(parent/'model.blend'));scene=bpy.context.scene;bpy.context.view_layer.update();templates=[o for o in scene.objects if o.type=='MESH'];objects=[];records=[]
    groups={}
    for a in authority['records']:
        if a['profile']=='Croisement01 - hiding Pc':groups.setdefault(tuple(a['display_position']),[]).append(a)
    assert sum(map(len,groups.values()))==32 and len(groups)==20
    for index,aliases in enumerate(groups.values()):
        tag=f'site-{index:02}';instance=aliases[0]['id']
        ar=next(r for r in authority['records']if r['id']==instance);x0,y0=np.array(ar['display_position'])+ar['initial']['offset'];translation=Vector((float(x0+w/2),float(-(y0+h/2)/SIN),0));available=terrain+walls;receivers=[r for r in available if np.all(r['maximum']>=np.array([x0-1,y0-1]))and np.all(r['minimum']<=np.array([x0+w+1,y0+h+1]))];assert receivers;own=[];support=[]
        for template in templates:
            obj=template.copy();obj.data=template.data.copy();obj.name=tag+' '+template.name;scene.collection.objects.link(obj)
            for v in obj.data.vertices:v.co+=translation
            obj.data.update();bpy.context.view_layer.update();body=triangles([obj]);shift,witness,pairs=clearance(body,receivers);assert shift*SIN<100,(tag,obj.name,shift)
            for v in obj.data.vertices:v.co+=RAY*shift
            for i,m in enumerate(list(obj.data.materials)):obj.data.materials[i]=m.copy()
            obj.data.update();own.append(obj);objects.append(obj);support.append(dict(object=obj.name,ray_shift=shift,height_gain=shift*SIN,witness=witness,triangle_overlap_pairs=pairs))
        bpy.context.view_layer.update();tree,owners,_=_tree(own);assigned={o:np.zeros((h,w),bool)for o in own};missing=[];foreign=[]
        for y in range(h):
            for x in range(w):
                p,_,_,_=tree.ray_cast(Vector((float(x0+x+.5),float(-(y0+y+.5)/SIN),0))+RAY*2000,-RAY)
                if rgba[y,x,3]>0 and p is None:missing.append([x,y])
                if rgba[y,x,3]==0 and p is not None:foreign.append([x,y])
        assert not missing and not foreign
        for y,x in np.argwhere(rgba[:,:,3]>0):
            for dx,dy in [(a,b)for a in [.01,.25,.5,.75,.99]for b in [.01,.25,.5,.75,.99]]:
                p,_,ti,_=tree.ray_cast(Vector((float(x0+x+dx),float(-(y0+y+dy)/SIN),0))+RAY*2000,-RAY)
                if p is not None:assigned[owners[ti]][y,x]=True
        for obj in own:
            pixels=rgba.copy();pixels[:,:,:3]=137;pixels[assigned[obj]]=rgba[assigned[obj]];file=out/(obj.name.replace(' ','-')+'-source.png');Image.fromarray(pixels).save(file);image=bpy.data.images.load(str(file));image.pack()
            for mat in obj.data.materials:
                for n in mat.node_tree.nodes:
                    if n.type=='TEX_IMAGE':n.image=image
        records.append(dict(tag=tag,instance=instance,aliases=[dict(id=a['id'],contract_sha256=a['contract_sha256'])for a in aliases],source_origin=[int(x0),int(y0)],objects=[o.name for o in own],support=support,native_centers=1011,missing=missing,foreign=foreign))
    for o in templates:bpy.data.objects.remove(o,do_unlink=True)
    model=out/'model.blend';bpy.ops.wm.save_as_mainfile(filepath=str(model))
    (out/'validation.json').write_text(json.dumps(dict(status='PRIVATE_PROPAGATION',model_sha256=sha(model),parent_sha256=sha(parent/'model.blend'),source_sha256=source['source_sha256'],static_base_sha256=sha(base),substitutions=pins,records=records,scope='Twenty unique positions bind all32 initial instances. Reopened guards and exceptional contact views required. Exact projected triangle overlap extrema determine rigid ray clearance, rather than sampled texel corners. Actual closed coverage is gray geometry; no alpha discard. Source colors reprojected after placement. No all-instance propagation or texture approval.'),indent=2)+'\n')
if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
