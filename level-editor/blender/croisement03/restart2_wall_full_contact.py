"""Private full static tree/wall root contact; surrounding ground remains unfinished."""
import json,math,sys,hashlib,collections
from pathlib import Path
import bpy
from mathutils import Matrix,Vector
from mathutils.bvhtree import BVHTree
from PIL import Image,ImageDraw,ImageChops
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from render_views import render_views
from texture_camera import depth_clip_range
OUT=ROOT/'level-editor/work/croisement03-refinement';R=OUT/'restart2'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
    wall=R/'stone-wall-v4/assets/croisement03-southeast-stone-wall';wood=R/'tree25-full-v13/assets/croisement03-tree-25';out=R/'tree25-wall-ground-joint-v6';out.mkdir(exist_ok=False)
    hashes={str(p):sha(p) for p in [wall/'model.blend',wood/'model.blend']};acquire();bpy.ops.wm.open_mainfile(filepath=str(wall/'model.blend'));bpy.context.preferences.filepaths.save_version=0
    scene=bpy.data.scenes.new('Private tree25 wall contact');objects=[]
    originals=tuple(bpy.data.collections['Croisement03 Working'].all_objects)
    assert all(obj is not None for obj in originals)
    wall_objects=[obj for obj in originals if obj.type=='MESH' and obj.get('asset_group')=='croisement03-southeast-stone-wall']
    assert len(wall_objects)==7
    for original in wall_objects:
        obj=original.copy();obj.parent=None;obj.matrix_world=original.matrix_world.copy();obj.hide_render=False;scene.collection.objects.link(obj);objects.append(obj)
    with bpy.data.libraries.load(str(wood/'model.blend'),link=False) as (src,dst):dst.objects=src.objects
    for obj in dst.objects:
        if obj is not None and obj.type=='MESH' and obj.get('asset_group')=='croisement03-tree-25':
            # The wood recipe stores world-space vertices with identity matrix.
            # Assign this declared transform explicitly after linking.
            scene.collection.objects.link(obj);obj.parent=None;obj.matrix_world=Matrix.Identity(4);obj.hide_render=False;objects.append(obj)
    assert len(objects)==8
    trees=[(o,BVHTree.FromPolygons([o.matrix_world@v.co for v in o.data.vertices],[list(p.vertices) for p in o.data.polygons])) for o in objects]
    sin=math.sin(math.radians(35));cos=math.cos(math.radians(35));ray=Vector((0,-cos,sin));audit=json.loads((R/'southeast-wall-source/gray-owner-audit.json').read_text());rows=[];counts=collections.Counter()
    level=json.loads((OUT/'baseline/Croisement03.rhp.json').read_text());stone=Image.new('L',(1408,960));ImageDraw.Draw(stone).polygon([(1245,787),(1253,790),(1260,793),(1267,788),(1271,786),(1278,790),(1282,795),(1280,800),(1267,798),(1259,801),(1247,795)],fill=255)
    native=Image.new('L',stone.size);native.paste(Image.open(OUT/'baseline/masks/000113.png'),tuple(level['masks'][113]['box_top_left']));stone=ImageChops.darker(stone,native);stone.save(out/'foreground-stone-probe.png');sp=stone.load();stone_pixels=[(x,y) for y in range(784,803) for x in range(1243,1284) if sp[x,y]]
    probes=[(x,y,'native-bark') for x,y in audit['pixels']]+[(x,y,'foreground-stone') for x,y in stone_pixels]
    for x,y,owner in probes:
        origin=Vector((x+.5,-(y+.5)/sin,0))+ray*10000;hits=[]
        for obj,tree in trees:
            point,normal,face,distance=tree.ray_cast(origin,-ray)
            if point is not None:hits.append((distance,obj,point))
        hits.sort(key=lambda h:h[0]);counts[owner+' -> '+hits[0][1]['asset_group']]+=1;rows.append(dict(pixel=[x,y],expected_owner=owner,hits=[dict(asset=h[1]['asset_group'],node=h[1].get('source_node'),point=list(h[2])) for h in hits]))
    wall_points=[o.matrix_world@v.co for o in objects if o.get('asset_group')=='croisement03-southeast-stone-wall' for v in o.data.vertices]
    x0,x1=min(p.x for p in wall_points)-30,max(p.x for p in wall_points)+30
    y0,y1=min(p.y for p in wall_points)-35,max(p.y for p in wall_points)+35
    mesh=bpy.data.meshes.new('Native zero-height diagnostic ground');mesh.from_pydata([(x0,y0,0),(x1,y0,0),(x1,y1,0),(x0,y1,0)],[],[(0,1,2,3)]);mesh.update()
    ground=bpy.data.objects.new(mesh.name,mesh);scene.collection.objects.link(ground);objects.append(ground)
    mat=bpy.data.materials.new('Neutral ground contact diagnostic');mat.use_nodes=True;mat.node_tree.nodes.get('Principled BSDF').inputs['Base Color'].default_value=(.22,.25,.19,1);mat.node_tree.nodes.get('Principled BSDF').inputs['Roughness'].default_value=1;mesh.materials.append(mat)
    light_data=bpy.data.lights.new('Joint sun','SUN');light_data.energy=2;light=bpy.data.objects.new(light_data.name,light_data);scene.collection.objects.link(light);light.rotation_euler=Vector((-.45,-.55,.70)).to_track_quat('Z','Y').to_euler();scene.world=bpy.data.worlds.new('Joint world');scene.world.color=(.18,.18,.18)
    scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.transparent_max_bounces=256;scene.view_settings.view_transform='Standard';scene.view_settings.look='None'
    packet=json.loads((wall/'modified/views.json').read_text());points=[o.matrix_world@v.co for o in objects for v in o.data.vertices];views={}
    for v in packet['views']:
        data=bpy.data.cameras.new('Joint '+str(v['index']));data.type='ORTHO';data.ortho_scale=v['ortho_scale']*1.15;inverse=Matrix(v['camera_matrix_world']).inverted();data.clip_start,data.clip_end=depth_clip_range(-(inverse@p).z for p in points);camera=bpy.data.objects.new(data.name,data);scene.collection.objects.link(camera);camera.matrix_world=Matrix(v['camera_matrix_world']);views[f'view-{v["index"]}']=camera.name
    bpy.ops.wm.save_as_mainfile(filepath=str(out/'joint.blend'));render_views(scene.name,views,out,modes=('textured',),width=384)
    images=[Image.open(out/f'view-{i}-textured.png').convert('RGB') for i in range(8)];tw,th=images[0].size;sheet=Image.new('RGB',(tw*4,th*2))
    for i,im in enumerate(images):sheet.paste(im,((i%4)*tw,(i//4)*th))
    sheet.save(out/'sheet.png');(out/'evidence.json').write_text(json.dumps(dict(source_hashes=hashes,joint_sha256=sha(out/'joint.blend'),sheet_sha256=sha(out/'sheet.png'),native_view_index=0,ground_z=0,ground_is_diagnostic=True,wall_bottom_range=[min(p.z for p in wall_points),max(min((o.matrix_world@v.co).z for v in o.data.vertices) for o in objects if o.get('asset_group')=='croisement03-southeast-stone-wall')],gray_cap_first_hits=dict(counts),pixel_hits=rows,limitations=['Neutral zero-height ground demonstrates seating but does not complete authored terrain, shadow or vegetation ownership.','Private root contact for the full static tree candidate. Tree independent review, wind states and ground environment remain unfinished.','Tree root at source y800 and circular depth are hypotheses; no approval implied by hit counts.','Wall-centered cameras intentionally show the local junction. Complete tree review has its own larger frozen cameras.']),indent=2)+'\n')
    assert all(sha(Path(p))==digest for p,digest in hashes.items());release();print(dict(counts))
if __name__=='__main__':main()
