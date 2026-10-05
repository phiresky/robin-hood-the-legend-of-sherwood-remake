"""Private dry-stone wall with native changing heights and complete hidden volume."""
import json,sys,math
from pathlib import Path
import bpy,bmesh,numpy as np
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from refinement_workspace import prepare,modified
ASSET='croisement03-southeast-stone-wall'
def main():
    root=OUT/'restart2/stone-wall-v3';root.mkdir(exist_ok=False);worker=root/'assets'/ASSET
    masks=json.loads((OUT/'baseline/masks/manifest.json').read_text())
    for row in masks['masks']:row['png']=str(OUT/'baseline/masks'/row['png'])
    write_json(root/'mask-inventory.json',masks)
    write_json(root/'source-masks.json',dict(version=1,mask_inventory=str(root/'mask-inventory.json'),projections={'exterior':dict(state='Initial native southeast dry-stone wall',source_sha256=sha(OUT/'baseline/covered.png'),assignments=[dict(reviewed=True,asset_group=ASSET,mask_indices=[113])])}))
    acquire();bpy.ops.wm.open_mainfile(filepath=str(OUT/'croisement03-grouped.blend'));bpy.context.preferences.filepaths.save_version=0
    prepare(worker,asset_id=ASSET,scene_name='Croisement03 Refinement',collection_name='Croisement03 Working',source_path=OUT/'baseline/covered.png',grouping_manifest=OUT/'catalog.json',inventory_path=OUT/'inventory/inventory.json',review_path=OUT/'grouping-review.json',source_mask_manifest=root/'source-masks.json',width=384,height=256,framing_padding=1.2,lighting=dict(toward_sun=[-.45,-.55,.70],ambient=.22,diffuse=.78,shadow_epsilon=.05))
    records=[]
    for obj in bpy.data.collections['Croisement03 Working'].all_objects:
        if obj.type!='MESH' or obj.get('asset_group')!=ASSET:continue
        v=np.array([tuple(obj.matrix_world@p.co) for p in obj.data.vertices]);xy=v[:,:2];center=xy.mean(axis=0);_,_,vv=np.linalg.svd(xy-center,full_matrices=False);axis=vv[0];axis*=1 if axis[0]>0 else -1;side=np.array([-axis[1],axis[0]]);along=(xy-center)@axis;across=(xy-center)@side
        lo,hi=along.min(),along.max();width=across.max()-across.min();offset=(across.max()+across.min())/2;h0=max(v[along<lo+.2*(hi-lo),2]);h1=max(v[along>hi-.2*(hi-lo),2]);rng=np.random.default_rng(87300+int(obj['source_node'].split('-')[-1]));vertices=[];faces=[];count=0
        def height(s):return max(.25,h0+(h1-h0)*(s-lo)/(hi-lo))
        def wedge(start,end,bottom,left_top,right_top,depth,lateral=0):
            if min(left_top,right_top)<=bottom+.1:return False
            n=len(vertices)
            for z,s,u in [(bottom,start,-depth/2),(bottom,end,-depth/2),(bottom,end,depth/2),(bottom,start,depth/2),(left_top,start,-depth/2),(right_top,end,-depth/2),(right_top,end,depth/2),(left_top,start,depth/2)]:
                p=center+axis*s+side*(offset+lateral+u);vertices.append((float(p[0]),float(p[1]),float(z)))
            faces.extend(tuple(n+i for i in f) for f in [(3,2,1,0),(4,5,6,7),(0,1,5,4),(1,2,6,5),(2,3,7,6),(3,0,4,7)]);return True
        # Recessed closed rubble core fills real inter-stone seams; it follows
        # the same changing top profile rather than a uniform-height slab.
        wedge(lo,hi,0,max(.25,h0-.65),max(.25,h1-.65),width*.86)
        for course in range(math.ceil(max(h0,h1)/8)):
            bottom=course*8+.04;cursor=lo
            while cursor<hi-.1:
                end=min(hi,cursor+rng.uniform(7,13));left=min(bottom+8.15,height(cursor));right=min(bottom+8.15,height(end))
                if min(left,right)>bottom+.6:
                    count+=int(wedge(cursor+.06,end-.06,bottom,left,right,width*rng.uniform(.96,1.015),rng.uniform(-.15,.15)))
                cursor=end
        # Native crest evidence at the root junction distinguishes a lower
        # bark-facing saddle from two higher foreground stone shoulders.
        number=int(obj['source_node'].split('-')[-1])
        if number in (88,89):
            controls=([(1244.,0.),(1248.,0.),(1250.,1.7),(1253.,7.25),(1256.,7.25)]
                      if number==88 else [(1250.,-2.0),(1258.,-2.0),(1263.,0.),(1271.,4.),(1278.,0.),(1290.,0.)])
            adjusted=[]
            for x,y,z in vertices:
                delta=float(np.interp(x,[v[0] for v in controls],[v[1] for v in controls]))
                adjusted.append((x,y,z+delta*min(1.,z/20.)))
            vertices=adjusted
        mesh=bpy.data.meshes.new(obj.name+' closed stone courses');mesh.from_pydata(vertices,[],faces);mesh.update();bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
        bmesh.ops.bevel(bm,geom=list(bm.edges),offset=.32,segments=1,affect='EDGES',clamp_overlap=True)
        bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));topology=dict(nonmanifold_edges=sum(not e.is_manifold for e in bm.edges),degenerate_faces=sum(f.calc_area()<1e-8 for f in bm.faces));assert not any(topology.values());bm.to_mesh(mesh);bm.free()
        material=bpy.data.materials.new(obj.name+' unknown stone');material.diffuse_color=(.42,.42,.42,1);mesh.materials.append(material);mesh.uv_layers.new(name='UVMap');obj.data=mesh;obj.matrix_world.identity()
        records.append(dict(node=obj['source_node'],native_heights=[float(h0),float(h1)],native_depth=float(width),stone_count=count,topology=topology))
    modified(worker);(worker/'inspection').mkdir(exist_ok=True)
    write_json(worker/'inspection/construction.json',dict(status='PRIVATE HOLD: actual material, native edge and terrain joint review pending',model_sha256=sha(worker/'model.blend'),parts=records,limitations=['Native height gradients and the low central break are retained. Individual stone count, hidden courses and rubble core are inferred.','Right end follows the native volume beyond the image boundary instead of being cropped at map edge.','Native113 is provisional source scope, not proof every overlapping green pixel belongs to stone; actual material review must resolve foliage contamination.','Unrelated trees, terrain and wall-neighbor joints remain unfinished.']))
    release();print(worker)
if __name__=='__main__':main()
