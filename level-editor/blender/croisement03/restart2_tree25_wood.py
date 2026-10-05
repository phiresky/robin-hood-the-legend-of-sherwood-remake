"""Private source-traced branching wood; canopy and wall contact remain pending."""
import json,math,sys
from pathlib import Path
import bpy,bmesh
from mathutils import Vector
from PIL import Image,ImageDraw,ImageChops
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from refinement_workspace import prepare,modified
ASSET='croisement03-tree-25';SIN=math.sin(math.radians(35));COS=math.cos(math.radians(35));RAY=Vector((0,-COS,SIN))
PATHS=[[(1263, 800, 17), (1263, 767, 16), (1264, 744, 14), (1275, 720, 12), (1283, 694, 10), (1284, 670, 9), (1293, 646, 4.5), (1296, 625, 2.6), (1299, 606, 0.8)], [(1264, 744, 13), (1246, 724, 12), (1232, 709, 11), (1210, 700, 8), (1187, 690, 4), (1165, 680, 1)], [(1232, 709, 7), (1213, 691, 6), (1200, 681, 5), (1188, 672, 4), (1187, 658, 1.5)], [(1283, 704, 6), (1265, 697, 5), (1250, 687, 3.5), (1235, 676, 2), (1234, 652, 0.7)], [(1280, 711, 8), (1303, 699, 5), (1326, 687, 4), (1348, 685, 2.8), (1370, 681, 1)]]
def main():
    root=OUT/'restart2/tree25-wood-v4';root.mkdir(exist_ok=False);worker=root/'assets'/ASSET
    level=json.loads((OUT/'baseline/Croisement03.rhp.json').read_text());source=Image.open(OUT/'baseline/covered.png');domain=Image.new('L',source.size);d=ImageDraw.Draw(domain)
    # Only the lower continuous bark is positively owned at this stage.
    # Upper branch geometry is present, but its overlapping leaves need their
    # own receiver and source ownership before any appearance approval.
    d.polygon([(1246,740),(1254,746),(1266,741),(1274,731),(1278,747),(1278,780),(1281,789),(1268,791),(1256,795),(1247,780)],fill=255)
    native=Image.new('L',source.size);native.paste(Image.open(OUT/'baseline/masks/000025.png'),tuple(level['masks'][25]['box_top_left']));domain=ImageChops.darker(domain,native)
    for index in [70,113,116]:
        exclusion=Image.new('L',source.size);exclusion.paste(Image.open(OUT/f'baseline/masks/{index:06}.png'),tuple(level['masks'][index]['box_top_left']));domain=ImageChops.subtract(domain,exclusion)
    domain.save(root/'observed-lower-bark.png')
    masks=json.loads((OUT/'baseline/masks/manifest.json').read_text())
    for row in masks['masks']:row['png']=str(OUT/'baseline/masks'/row['png'])
    masks['masks'].append(dict(index=131,layer=0,layer_index=131,png=str(root/'observed-lower-bark.png'),box_top_left=[0,0],box_size=list(source.size),authored=True,mask_type=0,obstacle_indices=[46]));write_json(root/'mask-inventory.json',masks)
    write_json(root/'source-masks.json',dict(version=1,mask_inventory=str(root/'mask-inventory.json'),projections={'exterior':dict(state='Private lower bark ownership; upper limbs and crown pending',source_sha256=sha(OUT/'baseline/covered.png'),assignments=[dict(reviewed=True,asset_group=ASSET,mask_indices=[131])])}))
    write_json(root/'source-trace.json',dict(paths=PATHS,ground_source_y=800,scope='Branch centerlines and radius estimates from own native artwork. Ground root, concealed branch lengths and circular cross sections are inferred. Lower bark positive trace only; no finished tree or canopy claim.'))
    acquire();bpy.ops.wm.open_mainfile(filepath=str(OUT/'croisement03-grouped.blend'));bpy.context.preferences.filepaths.save_version=0
    obj=next(o for o in bpy.data.collections['Croisement03 Working'].all_objects if o.type=='MESH' and o.get('asset_group')==ASSET);vertices=[];faces=[];sides=16
    for path in PATHS:
        centers=[Vector((x,-800/SIN,(800-y)/COS)) for x,y,r in path];start=len(vertices)
        for i,(center,point) in enumerate(zip(centers,path)):
            axis=(centers[min(i+1,len(path)-1)]-centers[max(0,i-1)]).normalized();side=axis.cross(RAY).normalized();up=axis.cross(side).normalized()
            for k in range(sides):
                angle=k*math.tau/sides;radius=point[2]*(1+.025*math.cos(angle*5));vertices.append(tuple(center+radius*(side*math.cos(angle)+up*math.sin(angle))))
        faces.append(tuple(start+k for k in reversed(range(sides))))
        for i in range(len(path)-1):
            for k in range(sides):faces.append((start+i*sides+k,start+i*sides+(k+1)%sides,start+(i+1)*sides+(k+1)%sides,start+(i+1)*sides+k))
        faces.append(tuple(start+(len(path)-1)*sides+k for k in range(sides)))
    # A declared construction proxy freezes cameras around the full traced
    # branch extent. The native obstacle only spans the lower trunk.
    lo=[min(v[i] for v in vertices)-2 for i in range(3)];hi=[max(v[i] for v in vertices)+2 for i in range(3)]
    proxy=bpy.data.meshes.new('Tree25 full-branch framing proxy');proxy.from_pydata([(x,y,z) for z in [lo[2],hi[2]] for y in [lo[1],hi[1]] for x in [lo[0],hi[0]]],[],[(0,2,3,1),(4,5,7,6),(0,1,5,4),(2,6,7,3),(0,4,6,2),(1,3,7,5)]);proxy.update();proxy.uv_layers.new(name='UVMap');fallback=bpy.data.materials.new('Declared tree framing proxy unknown');fallback.diffuse_color=(.42,.42,.42,1);proxy.materials.append(fallback);obj.data=proxy;obj.matrix_world.identity();obj['construction_proxy']='Full traced wood bounds for fixed review cameras; not native obstacle geometry'
    prepare(worker,asset_id=ASSET,scene_name='Croisement03 Refinement',collection_name='Croisement03 Working',source_path=OUT/'baseline/covered.png',grouping_manifest=OUT/'catalog.json',inventory_path=OUT/'inventory/inventory.json',review_path=OUT/'grouping-review.json',source_mask_manifest=root/'source-masks.json',width=384,height=384,framing_padding=1.25,lighting=dict(toward_sun=[-.45,-.55,.70],ambient=.22,diffuse=.78,shadow_epsilon=.05))
    mesh=bpy.data.meshes.new('Tree25 closed traced limbs');mesh.from_pydata(vertices,[],faces);mesh.update();bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bm.to_mesh(mesh);bm.free();obj.data=mesh;obj.matrix_world.identity()
    bpy.context.view_layer.objects.active=obj;obj.select_set(True);remesh=obj.modifiers.new('Fused branch junctions','REMESH');remesh.mode='VOXEL';remesh.voxel_size=.65;remesh.use_smooth_shade=False;bpy.ops.object.modifier_apply(modifier=remesh.name)
    bm=bmesh.new();bm.from_mesh(obj.data);topology=dict(nonmanifold_edges=sum(not e.is_manifold for e in bm.edges),degenerate_faces=sum(f.calc_area()<1e-8 for f in bm.faces));bm.free();assert not any(topology.values())
    obj.data.uv_layers.new(name='UVMap');material=bpy.data.materials.new('Unknown tree25 bark');material.diffuse_color=(.42,.42,.42,1);obj.data.materials.append(material)
    modified(worker);(worker/'inspection').mkdir(exist_ok=True);write_json(worker/'inspection/construction.json',dict(status='PRIVATE HOLD: source-traced wood only; crown, branch ownership, actual material and wall joint review pending',model_sha256=sha(worker/'model.blend'),topology=topology,limitations=['Wood branching inferred in circular cross section from native source centerlines.','Canopy and animated leaf surfaces are not yet reconstructed.','Only lower positive bark source is assigned; upper gray is intentional unresolved ownership, not a ready texture fill request.','Tree25 root and southeast wall contact require combined geometric/source review.']))
    release();print(worker)
if __name__=='__main__':main()
