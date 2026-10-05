"""Private source-constrained broken cart panel with a physical framed aperture."""
import json
import math
import sys
from pathlib import Path
import bpy
import bmesh
import numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree

HERE = Path(__file__).resolve().parent
sys.path[:0] = [str(HERE), str(HERE.parents[1] / 'refinement'),
                str(HERE.parents[1] / 'refinement/blender')]
from catalog import OUT
from tree_geometry import RAY, SIN, COS
from log_trap_state_candidate import point
from scenery_geometry import Mesh
from approved_texture_stage import geometry, appearance
from evidence_io import sha, write_json, record_recipe
from render_slots import acquire, release


def main():
    source = OUT / 'restart2-state/south-cart-wreck-solid-v3'
    dest = OUT / 'restart3-south-cart/broken-panel-v2'
    assert not dest.exists()
    model_hash = sha(source / 'worker.blend')
    assert model_hash == '192fbfa9e2f9806aae828f50ac25247e9fa0f5150384e9d14fff2868fb84362c'
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(source / 'worker.blend'))
        scene = bpy.context.scene
        original = {o.name: (geometry(o), appearance(o)) for o in scene.objects if o.type == 'MESH'}
        roof = scene.objects['Tipped barrel canopy shell']
        paint, gray = roof.data.materials[:2]
        survey_path = OUT / 'restart3-south-cart/surface-survey-v1.json'
        survey = json.loads(survey_path.read_text())
        supports = [(72,28),(96,14),(129,19),(138,72)]
        support_hits = [next(r['hits'][0] for r in survey['rows'] if tuple(r['pixel']) == p) for p in supports]
        design = np.array([[x,y,1] for x,y in supports], dtype=float)
        depths = np.array([h['ray_depth'] for h in support_hits])
        plane = np.linalg.lstsq(design, depths, rcond=None)[0]

        def located(pixel, offset=0):
            x,y = pixel
            p = point(953+x,844+y,0)
            depth = float(np.dot([x,y,1],plane)) + offset
            return p + RAY*(depth-p.dot(RAY))

        def mesh(name, vertices, faces):
            data = bpy.data.meshes.new(name)
            data.from_pydata(vertices,[],faces); data.update()
            bm=bmesh.new();bm.from_mesh(data)
            bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
            bmesh.ops.triangulate(bm,faces=list(bm.faces))
            assert all(e.is_manifold for e in bm.edges),name
            assert bm.calc_volume(signed=True)>0,name
            bm.to_mesh(data);bm.free()
            obj=bpy.data.objects.new(name,data);scene.collection.objects.link(obj)
            data.materials.append(paint);data.materials.append(gray)
            obj['state']='south cart terminal private broken-panel hypothesis'
            return obj

        def prism(name, contour, front=1.5, back=-1.5):
            n=len(contour)
            vertices=[located(p,d) for d in [front,back] for p in contour]
            faces=[tuple(range(n)),tuple(reversed(range(n,n*2)))]
            faces.extend((i,(i+1)%n,(i+1)%n+n,i+n) for i in range(n))
            return mesh(name,vertices,faces)

        inner=[(94,31),(103,37),(96,45),(85,39)]
        outer=[(95,28),(106,36),(97,49),(81,40)]
        contour=[(72,28),(95,13),(128,19),(143,40),(134,45),(137,49),
                 (128,55),(124,72),(108,69),(102,64),(95,61),(77,45)]
        panel=prism('Broken cabin panel',contour)
        cutter=prism('Temporary window cutting volume',outer,front=8,back=-8)
        bpy.context.view_layer.objects.active=panel
        modifier=panel.modifiers.new('Physical aperture','BOOLEAN')
        modifier.operation='DIFFERENCE';modifier.solver='EXACT';modifier.object=cutter
        bpy.ops.object.modifier_apply(modifier=modifier.name)
        bpy.data.objects.remove(cutter,do_unlink=True)
        vertices=[located(p,d) for d in [2.,-2.] for ring in [outer,inner] for p in ring]
        faces=[]
        for i in range(4):
            j=(i+1)%4
            faces.extend([(i,j,4+j,4+i),(8+i,12+i,12+j,8+j),
                          (i,8+i,8+j,j),(4+i,4+j,12+j,12+i)])
        mesh('Physical window frame',vertices,faces)
        for index,(a,b) in enumerate([((95,31),(96,45)),((85,39),(103,37))]):
            m=Mesh();m.tube(located(a,1.9),located(b,1.9),.65,n=8)
            mesh(f'Window crossbar {index}',m.vertices,m.faces)
        # Jagged, finite boards are separate from the bounded source image.
        prism('Broken projecting upper board',[(56,0),(72,1),(77,5),(91,14),(85,19),
              (78,32),(67,22),(69,18),(58,16)],front=.5,back=-2.5)
        prism('Split outer board',[(72,37),(80,44),(71,66),(74,72),(66,93),
              (64,87),(60,91),(60,75),(54,64),(62,58)],front=2,back=-1)
        # Four short joints tie the displaced broken panel to the fitted structure.
        for index,(pixel,hit) in enumerate(zip(supports,support_hits)):
            a=Vector(hit['world']);b=located(pixel,-1.5)
            if (a-b).length>.2:
                m=Mesh();m.tube(a,b,1.4,n=8)
                mesh(f'Broken panel support joint {index}',m.vertices,m.faces)
        added=[o for o in scene.objects if o.type=='MESH' and o.name not in original]
        for obj in added:
            bm=bmesh.new();bm.from_mesh(obj.data)
            bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
            bmesh.ops.triangulate(bm,faces=list(bm.faces))
            cuts=min(10,max(0,math.ceil(max(e.calc_length() for e in bm.edges)/3)-1))
            if cuts:bmesh.ops.subdivide_edges(bm,edges=list(bm.edges),cuts=cuts,use_grid_fill=True)
            assert all(e.is_manifold for e in bm.edges),obj.name
            assert bm.calc_volume(signed=True)>0,obj.name
            bm.to_mesh(obj.data);bm.free();obj.data.update()
            uv=obj.data.uv_layers.new(name='Native target projection')
            for face in obj.data.polygons:
                for loop in face.loop_indices:
                    p=obj.data.vertices[obj.data.loops[loop].vertex_index].co
                    uv.data[loop].uv=((p.x-945)/220,1-(-p.y*SIN-p.z*COS-820)/180)
        vertices=[];faces=[]
        for obj in scene.objects:
            if obj.type!='MESH':continue
            start=len(vertices);vertices.extend(obj.matrix_world@v.co for v in obj.data.vertices)
            faces.extend(tuple(start+i for i in p.vertices) for p in obj.data.polygons)
        bvh=BVHTree.FromPolygons(vertices,faces)
        for obj in added:
            for face in obj.data.polygons:
                hit=bvh.ray_cast(face.center+RAY*2000,-RAY,4000)
                face.material_index=0 if face.normal.dot(RAY)>.05 and hit[0] is not None and (hit[0]-face.center).length<.05 else 1
        assert original=={o.name:(geometry(o),appearance(o)) for o in scene.objects if o.name in original}
        dest.mkdir(parents=True)
        bpy.context.preferences.filepaths.save_version=0
        bpy.ops.wm.save_as_mainfile(filepath=str(dest/'worker.blend'),compress=True)
        center=Vector(tuple((min(p[i] for p in vertices)+max(p[i] for p in vertices))/2 for i in range(3)))
        camera=scene.camera;camera.data.ortho_scale=270
        scene.render.resolution_x=scene.render.resolution_y=384
        views=[]
        for index in range(8):
            az=math.radians(index*45)
            direction=Vector((math.sin(az)*COS,-math.cos(az)*COS,SIN))
            camera.location=center+direction*3000
            camera.rotation_euler=(center-camera.location).to_track_quat('-Z','Y').to_euler()
            views.append(dict(index=index,direction=list(direction),native=index==0))
            for mode in ['actual','solid']:
                scene.view_layers[0].material_override=gray if mode=='solid' else None
                scene.render.filepath=str(dest/f'view-{index}-{mode}.png')
                bpy.ops.render.render(write_still=True)
        from PIL import Image
        for mode in ['actual','solid']:
            sheet=Image.new('RGBA',(384*4,384*2))
            for index in range(8):sheet.paste(Image.open(dest/f'view-{index}-{mode}.png'),((index%4)*384,(index//4)*384))
            sheet.save(dest/f'{mode}.png')
        recipe=record_recipe(dest,Path(__file__))
        write_json(dest/'manifest.json',dict(status='Private physical panel trial; self/root review pending',
            model_sha256=sha(dest/'worker.blend'),source_model_sha256=model_hash,
            survey_sha256=sha(survey_path),original_mesh_geometry_appearance_exact=True,
            window_inner=inner,window_outer=outer,panel_contour=contour,
            inferred_depth_plane=plane.tolist(),support_residuals=(design@plane-depths).tolist(),
            new_components=[o.name for o in added],views=views,recipe=recipe,
            limitations=['Panel fold/depth and jagged broken edges are hypotheses from native artwork.',
                        'Exact whole-object contact and load support need fresh reopened audit.',
                        'No texture generation, catalog publication or user approval.']))
    finally:
        release()


if __name__=='__main__':main()
