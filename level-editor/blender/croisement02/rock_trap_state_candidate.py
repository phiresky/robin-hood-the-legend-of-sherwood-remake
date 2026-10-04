"""Isolated full-volume rock endpoint hypotheses with bound terrain support."""
import json,sys,math,hashlib
from pathlib import Path
import bpy,bmesh
import numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from PIL import Image
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(Path(__file__).parent)]
from catalog import OUT
from render_slots import acquire,release
from tree_geometry import SIN,COS,RAY
from log_trap_state_candidate import material,point

def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def observed_outer_profile(alpha, rows, index):
    """Fit only the outer boundary; transparent interior holes never cut a rock."""
    cx,cy,rx,rz=rows[index];ry=math.sqrt((rx*SIN)**2+(rz*COS)**2);ratios=[]
    for angle in np.arange(32)*math.tau/32:
        dx,dy=math.cos(angle),math.sin(angle);expected=1/math.sqrt((dx/rx)**2+(dy/ry)**2);samples=[]
        for distance in np.arange(expected*.65,expected*1.3,.25):
            x=int(round(cx+dx*distance));y=int(round(cy+dy*distance))
            if not(0<=x<alpha.shape[1]and 0<=y<alpha.shape[0]):continue
            scores=[((x-a)/r)**2+((y-b)/math.sqrt((r*SIN)**2+(z*COS)**2))**2 for a,b,r,z in rows]
            if min(range(len(rows)),key=scores.__getitem__)!=index:continue
            samples.append((distance,bool(alpha[y,x])))
        occupied=[d for d,a in samples if a]
        last=max(occupied)if occupied else None
        outside=[d for d,a in samples if last is not None and d>last and not a]
        ratios.append(max(.75,min(1.15,(last+.5)/expected))if last is not None and len(outside)>=3 else 1.)
    return ratios

def main():
    source=OUT/'state-target-evidence/rock-trap';evidence=json.loads((source/'manifest.json').read_text());box=evidence['bbox'];left,top,right,bottom=box;dest=OUT/'rock-trap-state-candidate-v5';dest.mkdir(exist_ok=False)
    bank=OUT/'terrain-bank-candidate/assets/croisement02-north-woodland-bank';audit=json.loads((bank/'inspection/saved-model-audit.json').read_text());assert sha(bank/'model.blend')==audit['model_sha256'];names=[r['object']for r in audit['objects'] if r['source_node']in [f'building-{i:03d}'for i in range(5)]]
    acquire()
    try:
        bpy.ops.wm.read_factory_settings(use_empty=True);scene=bpy.context.scene;scene.render.engine='CYCLES';scene.cycles.samples=16;scene.view_settings.view_transform='Standard';scene.render.film_transparent=True;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.render.resolution_x=512;scene.render.resolution_y=512;scene.render.resolution_percentage=100
        scene.world=bpy.data.worlds.new('World');scene.world.use_nodes=True;scene.world.node_tree.nodes['Background'].inputs[0].default_value=(.15,.15,.15,1)
        lightdata=bpy.data.lights.new('Sun','SUN');lightdata.energy=2;light=bpy.data.objects.new('Sun',lightdata);scene.collection.objects.link(light);light.rotation_euler=(.6,-.5,-.4)
        with bpy.data.libraries.load(str(bank/'model.blend'),link=False)as(src,dst):dst.objects=names
        banks=[o for o in dst.objects if o];[scene.collection.objects.link(o)for o in banks];bpy.context.view_layer.update();vertices=[];faces=[]
        for o in banks:
            start=len(vertices);vertices.extend(o.matrix_world@v.co for v in o.data.vertices);faces.extend(tuple(start+i for i in p.vertices)for p in o.data.polygons)
        bvh=BVHTree.FromPolygons(vertices,faces)
        gray=bpy.data.materials.new('Unobserved inferred rock');gray.diffuse_color=(.2,.2,.2,1);gray.use_nodes=True;gray.node_tree.nodes.get('Principled BSDF').inputs['Base Color'].default_value=(.2,.2,.2,1)
        surveys={'covered':json.loads((source/'covered-ellipsoid-fit.json').read_text())['survey'],'applied':[(32,70,19,17),(55,88,18,18),(69,71,13,14),(83,79,12,12),(67,110,6,5)]};states={};records=[]
        for state,rows in surveys.items():
            tick=-1 if state=='covered' else 104;mat=material(source/f'tick-{tick:03d}.png');objects=[];alpha=np.array(Image.open(source/f'tick-{tick:03d}.png'))[:,:,3]>0
            for i,(sx,sy,rx,rz)in enumerate(rows):
                profile=observed_outer_profile(alpha,rows,i)if state=='applied'else [1.]*32
                sx+=left;sy+=top;z=max(v.z for v in vertices)+rz+.5 if state=='covered' else rz+.5;support=0
                for _ in range(6):
                    center=point(sx,sy,z);hit=bvh.ray_cast(Vector((center.x,center.y,500)),Vector((0,0,-1)),1000);support=float(hit[0].z)if hit[0]is not None else 0;z=max(0,support)+rz+.5
                center=point(sx,sy,z);bpy.ops.mesh.primitive_ico_sphere_add(subdivisions=3,radius=1,location=center);o=bpy.context.object;o.name=f'{state} inferred complete boulder {i:02d}'
                for v in o.data.vertices:
                    p=v.co.copy();angle=math.atan2(p.y,p.x);variation=1+.05*math.sin(angle*3+i)+.025*math.cos(angle*5-i);v.co=(p.x*rx*variation,p.y*rx*variation,p.z*rz*(1+.03*math.sin(angle*4+i)))
                    if state=='applied':
                        q=v.co.copy();theta=math.atan2(-q.y*SIN-q.z*COS,q.x)%math.tau;u=theta/math.tau*32;k=int(u);ratio=profile[k]*(1-(u-k))+profile[(k+1)%32]*(u-k);along=RAY*q.dot(RAY);v.co=along+(q-along)*ratio
                o.data.update();bm=bmesh.new();bm.from_mesh(o.data);assert all(e.is_manifold for e in bm.edges);bm.free();o.data.materials.append(mat);o.data.materials.append(gray);uv=o.data.uv_layers.new(name='Native target projection')
                for face in o.data.polygons:
                    face.material_index=0 if face.normal.dot(RAY)>.05 else 1
                    for loop in face.loop_indices:
                        p=o.data.vertices[o.data.loops[loop].vertex_index].co+center;uv.data[loop].uv=((p.x-left)/(right-left),1-(-p.y*SIN-p.z*COS-top)/(bottom-top))
                o.data.update();o['state_endpoint']=state;o['geometry_status']='unapproved full inferred boulder';objects.append(o);records.append(dict(state=state,index=i,source_center=[sx,sy],inferred_radii=[rx,rx,rz],support_height=support,closed_manifold=True,outer_profile=profile))
            states[state]=objects
        camera_data=bpy.data.cameras.new('Review camera');camera_data.type='ORTHO';camera_data.clip_end=10000;camera_data.ortho_scale=max(right-left,bottom-top)*1.2;camera=bpy.data.objects.new('Review camera',camera_data);scene.collection.objects.link(camera);scene.camera=camera
        for state,objects in states.items():
            for k,others in states.items():
                for o in others:o.hide_render=k!=state
            for with_bank in (False,True):
                for o in banks:o.hide_render=not with_bank
                for view,direction in [('source',RAY),('oblique',Vector((-1,-1,.8)).normalized())]:
                    target=point((left+right)/2,(top+bottom)/2,0);camera.location=target+direction*3000;camera.rotation_euler=(target-camera.location).to_track_quat('-Z','Y').to_euler()
                    for mode in ('actual','solid'):
                        replacements=[]
                        if mode=='solid':
                            for o in objects:
                                for slot in o.material_slots:replacements.append((slot,slot.material));slot.material=gray
                        scene.render.filepath=str(dest/f'{state}-{view}-{mode}-bank{int(with_bank)}.png');bpy.ops.render.render(write_still=True)
                        for slot,original in replacements:slot.material=original
        for k,objects in states.items():
            for o in objects:o.hide_render=k!='covered'
        bpy.ops.wm.save_as_mainfile(filepath=str(dest/'worker.blend'))
        report=dict(status='Unapproved diagnostic: foreground foliage and detailed silhouette fitting still required',source_manifest_sha256=sha(source/'manifest.json'),bank_model_sha256=audit['model_sha256'],model_sha256=sha(dest/'worker.blend'),geometry=records,limitations=['Full closed boulders inferred from source endpoints; initial partial caps are not extruded into false sliced solids.','Source RGB projected only to source-facing faces; unobserved regions remain gray.','Bank support and actual bank occlusion included; shrub62 occlusion pending stable worker.','Endpoint boulders are independent hypotheses: temporal identity and motion are not proven.','No alpha clipping of physical geometry, no API, no canonical scene changes.'])
        (dest/'manifest.json').write_text(json.dumps(report,indent=2)+'\n')
    finally:release()
if __name__=='__main__':main()
