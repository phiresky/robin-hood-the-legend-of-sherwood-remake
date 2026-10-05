"""One source-traced boulder with a complete inferred back and seated base."""
import json,sys,math
from pathlib import Path
import bpy,bmesh,numpy as np
from mathutils import Vector
from PIL import Image,ImageDraw,ImageChops
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from refinement_workspace import prepare,modified
ASSET='croisement03-central-shrub-boulder';SIN=math.sin(math.radians(35));COS=math.cos(math.radians(35));RAY=Vector((0,-COS,SIN))
OUTLINE=[(732,619),(744,620),(757,623),(767,630),(775,638),(781,648),(780,658),(774,666),(761,672),(744,677),(730,674),(720,669),(716,662),(716,651),(711,643),(711,635),(718,626),(726,622)]
def main():
    root=OUT/'restart2/central-boulder-v1';root.mkdir(exist_ok=False);worker=root/'assets'/ASSET
    level=json.loads((OUT/'baseline/Croisement03.rhp.json').read_text());source=Image.open(OUT/'baseline/covered.png');native=Image.new('L',source.size);native.paste(Image.open(OUT/'baseline/masks/000097.png'),tuple(level['masks'][97]['box_top_left']))
    domain=Image.new('L',source.size);ImageDraw.Draw(domain).polygon(OUTLINE,fill=255);domain=ImageChops.darker(domain,native);domain.save(root/'observed-rock.png');ImageChops.subtract(native,domain).save(root/'excluded-foreground.png')
    masks=json.loads((OUT/'baseline/masks/manifest.json').read_text())
    for row in masks['masks']:row['png']=str(OUT/'baseline/masks'/row['png'])
    masks['masks'].append(dict(index=131,layer=0,layer_index=131,png=str(root/'observed-rock.png'),box_top_left=[0,0],box_size=list(source.size),authored=True,mask_type=0,obstacle_indices=[70]))
    write_json(root/'mask-inventory.json',masks);write_json(root/'source-masks.json',dict(version=1,mask_inventory=str(root/'mask-inventory.json'),projections={'exterior':dict(state='Observed stone excludes foreground left and lower foliage',source_sha256=sha(OUT/'baseline/covered.png'),assignments=[dict(reviewed=True,asset_group=ASSET,mask_indices=[131])])}))
    write_json(root/'source-trace.json',dict(outline=OUTLINE,native_mask=97,observed_pixels=int(np.count_nonzero(np.asarray(domain))),excluded_pixels=int(np.count_nonzero(np.asarray(ImageChops.subtract(native,domain)))),interpretation='One rounded boulder; left/base shrub is distinct foreground, not stone color or a fabricated rock lobe. Hidden rear depth and facets are inferred.'))
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(OUT/'croisement03-grouped.blend'));bpy.context.preferences.filepaths.save_version=0
        prepare(worker,asset_id=ASSET,scene_name='Croisement03 Refinement',collection_name='Croisement03 Working',source_path=OUT/'baseline/covered.png',grouping_manifest=OUT/'catalog.json',inventory_path=OUT/'inventory/inventory.json',review_path=OUT/'grouping-review.json',source_mask_manifest=root/'source-masks.json',width=384,height=256,framing_padding=1.2,lighting=dict(toward_sun=[-.45,-.55,.70],ambient=.22,diffuse=.78,shadow_epsilon=.05))
        objs=[o for o in tuple(bpy.data.collections['Croisement03 Working'].all_objects) if o.type=='MESH' and o.get('asset_group')==ASSET];assert len(objs)==1 and objs[0]['source_node']=='building-070';obj=objs[0]
        center=np.array([743.,649.]);depth=37.;points=[];faces=[];n=len(OUTLINE)
        def point(x,y,d):return tuple(Vector((x,-SIN*y,-COS*y))+RAY*d)
        # Rings share one native silhouette rim; both hemispheres close into
        # physical rock volume instead of a cutout or cropped back surface.
        for f in [1.,.72,.38]:
            for i,p in enumerate(OUTLINE):
                q=center+(np.array(p)-center)*f;d=depth*math.sqrt(max(0,1-f*f))*(1+.035*math.sin(i*2.7))
                points.append(point(q[0],q[1],d))
        front=len(points);points.append(point(*center,depth*1.01))
        for ring in range(2):
            for i in range(n):faces.append((ring*n+i,ring*n+(i+1)%n,(ring+1)*n+(i+1)%n,(ring+1)*n+i))
        for i in range(n):faces.append((2*n+i,2*n+(i+1)%n,front))
        for f in [.72,.38]:
            for i,p in enumerate(OUTLINE):
                q=center+(np.array(p)-center)*f;d=-depth*math.sqrt(max(0,1-f*f))*(1+.03*math.cos(i*1.9));points.append(point(q[0],q[1],d))
        back=len(points);points.append(point(*center,-depth))
        for i in range(n):faces.append((i,front+1+i,front+1+(i+1)%n,(i+1)%n));faces.append((front+1+i,front+1+n+i,front+1+n+(i+1)%n,front+1+(i+1)%n));faces.append((front+1+n+i,back,front+1+n+(i+1)%n))
        shift=-min(p[2] for p in points)/SIN;points=[tuple(Vector(p)+RAY*shift) for p in points]
        points=[tuple(Vector(p)-RAY*(max(0,p[2])/SIN)) if p[2]<2. else p for p in points]
        mesh=bpy.data.meshes.new('Central boulder complete faceted volume');mesh.from_pydata(points,[],faces);mesh.update();bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));topology=dict(nonmanifold_edges=sum(not e.is_manifold for e in bm.edges),degenerate_faces=sum(f.calc_area()<1e-8 for f in bm.faces));assert not any(topology.values()),topology;bm.to_mesh(mesh);bm.free()
        mat=bpy.data.materials.new('Central boulder inferred unknown stone');mat.diffuse_color=(.42,.42,.42,1);mesh.materials.append(mat);mesh.uv_layers.new(name='UVMap');obj.data=mesh;obj.matrix_world.identity()
        modified(worker);(worker/'inspection').mkdir(exist_ok=True)
        write_json(worker/'inspection/construction.json',dict(model_sha256=sha(worker/'model.blend'),topology=topology,nodes=['building-070'],source_ray_depth=depth*2,bounds=dict(min=[min(p[i] for p in points) for i in range(3)],max=[max(p[i] for p in points) for i in range(3)]),ground_contact_vertices=sum(abs(p[2])<1e-4 for p in points),status='PRIVATE: saved actual/native and diagnostic contact review pending',limitations=['Rear curvature and facet positions are inferred from one original view.','Foreground shrub and ground remain unrefined; excluded native97 pixels must not be called recovered stone.']))
        print(worker)
    finally:release()
if __name__=='__main__':main()
