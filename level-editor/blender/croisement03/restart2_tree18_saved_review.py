"""Independently review provisional saved Tree18 wood, source priority and joints."""
import hashlib
import itertools
import json
import math
import sys
from pathlib import Path

import bpy
import bmesh
import numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from PIL import Image

sys.path.insert(0,str(Path(__file__).resolve().parent))
import restart2_tree01_canopy_saved as guard

B,S,C=guard.B,guard.S,guard.C
F=B/'tree18-saved-wood-v1'
O=F/'saved-review-v1'
RAY=Vector((0,-C,S))


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    global F,O
    if '--folder' in sys.argv:F=Path(sys.argv[sys.argv.index('--folder')+1]).resolve()
    if '--out' in sys.argv:O=Path(sys.argv[sys.argv.index('--out')+1]).resolve()
    guard.resources();guard.acquire(slots=2)
    try:
        guard.resources();assert not O.exists();O.mkdir()
        model=F/'worker.blend';bank=B/'bank-convex-source-saved-v1/worker.blend'
        pins={str(p):sha(p)for p in [model,bank]}
        bpy.ops.wm.open_mainfile(filepath=str(bank));scene=bpy.context.scene
        for obj in scene.objects:
            if obj.type=='MESH':obj.hide_render=True
        with bpy.data.libraries.load(str(model),link=False)as(src,dst):
            dst.objects=[name for name in src.objects if name.startswith('Provisional Tree18 ')]
        wood=dst.objects;assert len(wood)==4
        for obj in wood:scene.collection.objects.link(obj);obj.hide_render=False
        rows=[]
        for obj in wood:
            obj.data.calc_loop_triangles()
            rows.append((obj,BVHTree.FromPolygons([obj.matrix_world@v.co for v in obj.data.vertices],
                [tuple(t.vertices)for t in obj.data.loop_triangles],all_triangles=True)))
        source=json.loads((B/'bank-foreground-source-audit-v2/receipt.json').read_text())
        union=set()
        domains=[]
        for frame in source['frames']:
            path=Path(frame['source']);assert sha(path)==frame['source_sha256'];a=np.array(Image.open(path).convert('RGBA'))
            x0,y0=frame['bbox'][:2];d={(int(x+x0),int(y+y0))for y,x in zip(*np.nonzero(a[:,:,3]))};domains.append(d);union|=d
        blocked={};plane_y=-462/S
        for x,y in sorted(union):
            native=Vector((x+.5,plane_y,(462-y-.5)/C));origin=native+RAY*5000
            hits=[]
            for obj,bvh in rows:
                point,normal,face,distance=bvh.ray_cast(origin,-RAY)
                if point is not None and distance<4999.999:hits.append([obj.name,float(distance),int(face)])
            if hits:blocked[(x,y)]=min(hits,key=lambda h:h[1])
        joints=[]
        for a,b in itertools.combinations(wood,2):
            clone=a.copy();clone.data=a.data.copy();scene.collection.objects.link(clone)
            bpy.context.view_layer.objects.active=clone
            modifier=clone.modifiers.new('Private junction intersection','BOOLEAN');modifier.operation='INTERSECT';modifier.solver='EXACT';modifier.object=b
            bpy.ops.object.modifier_apply(modifier=modifier.name)
            bm=bmesh.new();bm.from_mesh(clone.data)
            joints.append({'parts':[a.name,b.name],'intersection_volume':abs(float(bm.calc_volume())),
                           'intersection_faces':len(bm.faces),'closed':all(e.is_manifold for e in bm.edges)})
            bm.free();mesh=clone.data;bpy.data.objects.remove(clone,do_unlink=True);bpy.data.meshes.remove(mesh)
        report={'status':'HOLD_PRIVATE_WOOD_ANATOMY_AND_CONTACT_REVIEW','pins':pins,'native_union':len(union),
                'priority_measurement':'Current editor vertical-plane diagnostic only; authored native displayline priority is separate and not proved by these depths.','diagnostic_plane_native_ground_y':462,'blocked_native_union':len(blocked),'blocked_pixels':[{'xy':list(p),'hit':hit}for p,hit in blocked.items()],
                'phase_blocked':[len(d&blocked.keys())for d in domains],'junction_intersections':joints,
                'limits':['Exact saved geometry audit; static foliage, complete root contacts and runtime integration remain separate.',
                          'Four closed sweeps are not yet a united branching trunk. No bark or native leaf source assignment.']}
        (O/'receipt.json').write_text(json.dumps(report,indent=2)+'\n')
        scene.render.engine='CYCLES';scene.cycles.samples=16;scene.cycles.use_denoising=False
        scene.render.threads_mode='FIXED';scene.render.threads=2;scene.render.film_transparent=True
        scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA'
        scene.view_settings.view_transform='Standard'
        camera=bpy.data.objects.new('Saved Tree18 review camera',bpy.data.cameras.new('Saved Tree18 review camera'))
        scene.collection.objects.link(camera);camera.data.type='ORTHO';camera.data.sensor_fit='HORIZONTAL';camera.data.clip_end=10000;scene.camera=camera
        points=np.array([list(obj.matrix_world@v.co)for obj in wood for v in obj.data.vertices])
        origin=Vector(points.mean(axis=0));sheet=Image.new('RGB',(1280,768),'#333333')
        scene.render.resolution_x=320;scene.render.resolution_y=384;scene.render.resolution_percentage=100
        for index in range(8):
            angle=math.radians(index*45);direction=Vector((math.sin(angle)*C,-math.cos(angle)*C,S));rotation=(-direction).to_track_quat('-Z','Y')
            local=np.array([list(rotation.conjugated()@(Vector(p)-origin))for p in points]);middle=(local[:,:2].min(0)+local[:,:2].max(0))/2
            target=origin+rotation@Vector((*middle,0));camera.location=target+direction*5000;camera.rotation_euler=rotation.to_euler()
            camera.data.ortho_scale=max(np.ptp(local[:,0]),np.ptp(local[:,1])*320/384)*1.15
            scene.render.filepath=str(O/f'actual-{index*45:03}.png');bpy.ops.render.render(write_still=True)
            im=Image.open(scene.render.filepath).convert('RGBA');bg=Image.new('RGBA',im.size,'#333333');bg.alpha_composite(im);sheet.paste(bg.convert('RGB'),(index%4*320,index//4*384))
        sheet.save(O/'actual-eight.png')
        # Exact original camera crop, with no independent centering per image.
        box=(260,235,560,525);scene.render.resolution_x=600;scene.render.resolution_y=580
        center=((box[0]+box[2])/2,(box[1]+box[3])/2);target=Vector((center[0],-center[1]/S,0))
        camera.location=target+RAY*5000;camera.rotation_euler=(-RAY).to_track_quat('-Z','Y').to_euler();camera.data.ortho_scale=box[2]-box[0]
        scene.render.filepath=str(O/'native.png');bpy.ops.render.render(write_still=True)
        comparison=Image.new('RGB',(1200,580),'#333333');sourceim=Image.open(B.parent/'baseline/covered.png').convert('RGB').crop(box).resize((600,580),Image.Resampling.NEAREST);comparison.paste(sourceim,(0,0));im=Image.open(O/'native.png').convert('RGBA');bg=Image.new('RGBA',im.size,'#333333');bg.alpha_composite(im);comparison.paste(bg.convert('RGB'),(600,0));comparison.save(O/'native-comparison.png')
        assert all(sha(Path(p))==h for p,h in pins.items());assert sum(p.stat().st_size for p in O.rglob('*')if p.is_file())<32*1024**2
        print({'native_blocked':len(blocked),'junctions':joints})
    finally:guard.release()


if __name__=='__main__':main()
