"""Private eastern stone draft; hidden joins require saved-scene review."""
import sys, math, json
from pathlib import Path
import bpy, bmesh
import numpy as np
from PIL import Image,ImageDraw,ImageChops
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from refinement_workspace import prepare,modified
from restart2_east_rocks_source import TRACES
ASSET='croisement03-east-tree-rocks'
SIN=math.sin(math.radians(35));COS=math.cos(math.radians(35));RAY=Vector((0,-COS,SIN))

def shell(outline,depth,bottom,top):
    center=np.mean(outline,axis=0);points=[];faces=[];n=len(outline)
    def point(x,y,d):return tuple(Vector((x,-SIN*y,-COS*y))+RAY*d)
    for fraction in (1.,.66,.30):
        for i,p in enumerate(outline):
            q=center+(np.array(p)-center)*fraction
            d=depth*math.sqrt(1-fraction*fraction)*(1+.025*math.sin(i*2.7))
            points.append(point(*q,d))
    front=len(points);points.append(point(*center,depth))
    for ring in range(2):
        for i in range(n):faces.append((ring*n+i,ring*n+(i+1)%n,(ring+1)*n+(i+1)%n,(ring+1)*n+i))
    for i in range(n):faces.append((2*n+i,2*n+(i+1)%n,front))
    for fraction in (.66,.30):
        for p in outline:
            q=center+(np.array(p)-center)*fraction
            points.append(point(*q,-depth*math.sqrt(1-fraction*fraction)))
    back=len(points);points.append(point(*center,-depth))
    for i in range(n):
        faces.extend([(i,front+1+i,front+1+(i+1)%n,(i+1)%n),
                      (front+1+i,front+1+n+i,front+1+n+(i+1)%n,front+1+(i+1)%n),
                      (front+1+n+i,back,front+1+n+(i+1)%n)])
    lo=min(p[2] for p in points);hi=max(p[2] for p in points)
    result=[]
    for point0 in points:
        height=bottom+(point0[2]-lo)/(hi-lo)*(top-bottom)
        if height-bottom<2.5:height=bottom
        result.append(tuple(Vector(point0)+RAY*((height-point0[2])/SIN)))
    return result,faces

def main():
    root=OUT/'restart2/east-rocks-v5';root.mkdir(exist_ok=False)
    worker=root/'assets'/ASSET;preflight=OUT/'restart2/east-rocks98-preflight'
    inventory=json.loads((OUT/'baseline/masks/manifest.json').read_text())
    for row in inventory['masks']:row['png']=str(OUT/'baseline/masks'/row['png'])
    source=Image.open(OUT/'baseline/covered.png')
    domain=Image.open(preflight/'tentative-stone-domain.png').convert('L')
    foreign=Image.new('L',source.size)
    ImageDraw.Draw(foreign).polygon([(1116,407),(1123,409),(1124,414),(1129,420),(1129,424),(1124,425),(1124,432),(1118,430)],fill=255)
    level=json.loads((OUT/'baseline/Croisement03.rhp.json').read_text())
    leaf=Image.new('L',source.size)
    leaf.paste(Image.open(OUT/'baseline/masks/000061.png'),tuple(level['masks'][61]['box_top_left']))
    foreign=ImageChops.darker(foreign,leaf)
    domain=ImageChops.subtract(domain,foreign);domain.save(root/'observed-stone.png')
    foreign.save(root/'foreground-exclusion.png')
    inventory['masks'].append(dict(index=131,layer=0,layer_index=131,png=str(root/'observed-stone.png'),box_top_left=[0,0],box_size=list(source.size),authored=True,mask_type=0,obstacle_indices=[74,75]))
    write_json(root/'mask-inventory.json',inventory)
    write_json(root/'source-masks.json',dict(version=1,mask_inventory=str(root/'mask-inventory.json'),projections={'exterior':dict(state='Tentatively traced stone only; broad foreground shrub excluded',source_sha256=sha(OUT/'baseline/covered.png'),assignments=[dict(reviewed=True,asset_group=ASSET,mask_indices=[131])])}))
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(OUT/'croisement03-grouped.blend'))
        bpy.context.preferences.filepaths.save_version=0
        objects={o['source_node']:o for o in tuple(bpy.data.collections['Croisement03 Working'].all_objects) if o.type=='MESH' and o.get('asset_group')==ASSET}
        assert set(objects)=={'building-074','building-075'}
        native_heights={n:max((o.matrix_world@v.co).z for v in o.data.vertices) for n,o in objects.items()}
        spec={'building-074':[('front-block',14.,0.,24.),('left-ledge',15.,0.,24.)],
              'building-075':[('upright-face',18.,14.,67.)]}
        receipt=[]
        for node,parts in spec.items():
            points=[];faces=[]
            for name,depth,bottom,top in parts:
                p,f=shell(TRACES[name],depth,bottom,top)
                if name=='upright-face':
                    # Complete the hidden rear body down to ground behind the
                    # low front stones; do not leave a floating source facade.
                    p=[tuple(Vector(q)+RAY*((max(0.,(q[2]-14.)*2.4)-q[2])/SIN)) if q[2]<24. else q for q in p]
                offset=len(points)
                points.extend(p);faces.extend(tuple(offset+i for i in face) for face in f)
            mesh=bpy.data.meshes.new(node+' complete inferred stone')
            mesh.from_pydata(points,[],faces);mesh.update()
            bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
            topology=dict(nonmanifold_edges=sum(not e.is_manifold for e in bm.edges),degenerate_faces=sum(f.calc_area()<1e-8 for f in bm.faces))
            assert not any(topology.values()),topology
            bm.to_mesh(mesh);bm.free()
            mat=bpy.data.materials.new(node+' unknown stone');mat.diffuse_color=(.42,.42,.42,1)
            mesh.materials.append(mat);mesh.uv_layers.new(name='UVMap')
            obj=objects[node];obj.data=mesh;obj.matrix_world.identity()
            receipt.append(dict(node=node,surfaces=parts,topology=topology,bounds=dict(min=[min(p[i] for p in points) for i in range(3)],max=[max(p[i] for p in points) for i in range(3)])))
        prepare(worker,asset_id=ASSET,scene_name='Croisement03 Refinement',collection_name='Croisement03 Working',source_path=OUT/'baseline/covered.png',grouping_manifest=OUT/'catalog.json',inventory_path=OUT/'inventory/inventory.json',review_path=OUT/'grouping-review.json',source_mask_manifest=root/'source-masks.json',width=384,height=256,framing_padding=1.2,lighting=dict(toward_sun=[-.45,-.55,.70],ambient=.22,diffuse=.78,shadow_epsilon=.05))
        modified(worker);(worker/'inspection').mkdir(exist_ok=True)
        write_json(worker/'inspection/construction.json',dict(model_sha256=sha(worker/'model.blend'),nodes=receipt,native_heights=native_heights,status='PRIVATE draft; saved actual, source and support review required',limitations=['Three traced visible regions do not prove separate stones.','Low pieces height24 and upper stone top67 are inferred. The upper stone lower body completes down to ground along native source rays, preserving its observed silhouette; separate stone seating requires review.', 'The upper visual height exceeds its native coarse sight-obstacle maximum51.29; the original gameplay obstacle remains unchanged.', 'Upper-left foreground exclusion is bounded by both the manually observed leaf area and native shrub mask61; future actual vegetation must cover this unknown region.','Native obstacle gameplay is retained separately; visual curvature does not redefine obstacles.','No foliage ownership or completed terrain claim.']))
        print(worker)
    finally:release()

if __name__=='__main__':main()
