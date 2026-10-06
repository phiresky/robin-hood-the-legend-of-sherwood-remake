"""Private small foliage clusters preserving native visible RGB and wood geometry."""
import hashlib
import json
import math
from pathlib import Path
import random
import shutil
import sys
import bpy
import bmesh
import numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from mathutils.geometry import barycentric_transform

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(ROOT / 'level-editor/refinement'), str(ROOT / 'level-editor/refinement/blender')]
from render_slots import acquire, release
from refinement_workspace import _geometry
from workspace_components import appearance_state

E = ROOT / 'level-editor/work/croisement03-refinement/restart2/texture-batch-v7/croisement03-tree-25/experiment'
SIN, COS = math.sin(math.radians(35)), math.cos(math.radians(35))
RAY = Vector((0,-COS,SIN))


def sha(p):
    return hashlib.sha256(p.read_bytes()).hexdigest()


def make_image(name, rgba, source_image):
    height,width,_ = rgba.shape
    image = bpy.data.images.new(name,width=width,height=height,alpha=True)
    image.colorspace_settings.name = source_image.colorspace_settings.name
    image.pixels.foreach_set(np.flipud(rgba).copy().ravel())
    image.pack()
    return image


def make_material(source, name, image, known, double=False):
    material = source.copy(); material.name = name
    node = next(n for n in material.node_tree.nodes if n.type=='TEX_IMAGE' and n.image)
    node.image = image
    material['foliage_observed'] = known
    material['foliage_card_sides'] = 'double-sided' if double else 'paired-one-sided'
    material['texture_review_status'] = 'private-cluster-geometry-own-native-RGB-control'
    material.use_backface_culling = not double
    if double:
        shader = next(n for n in material.node_tree.nodes if n.type=='BSDF_PRINCIPLED')
        output = next(n for n in material.node_tree.nodes if n.type=='OUTPUT_MATERIAL')
        for link in list(output.inputs['Surface'].links): material.node_tree.links.remove(link)
        material.node_tree.links.new(shader.outputs['BSDF'],output.inputs['Surface'])
    return material


def main():
    assert shutil.disk_usage(ROOT).free > 25*1024**3
    out = E / 'cluster-geometry-v1'; assert not out.exists()
    src = E / 'rearpatch-geometry-v3/worker.blend'
    assert sha(src)=='ab07fa1187526bef5ea32e537f6c8b14245258b3c50fc27c18aeca57afdcc9bf'
    refs = json.loads((E / 'foliage-detail-retry-v1/auxiliary-references.json').read_text())['references']
    assert {r['asset_id'] for r in refs} == {'leicester-southeast-cottage-tree','leicester-moat-bank-tree'}
    for r in refs: assert sha(Path(r['parent_image'])) == r['parent_sha256']
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(src)); bpy.context.preferences.filepaths.save_version=0
        obj = next(o for o in bpy.data.objects if o.type=='MESH' and o.get('asset_group')=='croisement03-tree-25')
        mesh = obj.data; mesh.calc_loop_triangles()
        outside = {o.name:_geometry(o,protect_appearance=True) for o in bpy.data.objects if o.type=='MESH' and o!=obj}
        materials_before = appearance_state(obj)['materials']
        vertices = [obj.matrix_world@v.co for v in mesh.vertices]; tris = list(mesh.loop_triangles)
        old_bounds = [[min(p[i] for p in vertices),max(p[i] for p in vertices)] for i in range(3)]
        tree = BVHTree.FromPolygons(vertices,[list(t.vertices) for t in tris],all_triangles=True)
        atlases = {}
        for slot,material in enumerate(mesh.materials):
            if not material or not material.get('foliage_physical_opacity'): continue
            node = next(n for n in material.node_tree.nodes if n.type=='TEX_IMAGE' and n.image)
            pixels = np.empty(len(node.image.pixels),np.float32); node.image.pixels.foreach_get(pixels)
            atlases[slot] = dict(rgba=pixels.reshape(node.image.size[1],node.image.size[0],4),
                uv=mesh.uv_layers[node.inputs['Vector'].links[0].from_node.uv_map],
                sided=material.get('foliage_card_sides')=='paired-one-sided', image=node.image)
        # Recover the actual visible native foliage at one source-art pixel per
        # sample. Wood-owned pixels remain holes in the foliage atlas.
        bbox=(1092,500,1460,800); x0,y0,x1,y1=bbox; width,height=x1-x0,y1-y0
        rgba=np.zeros((height,width,4),np.float32); points=np.full((height,width,3),np.nan,np.float32)
        observed=np.zeros((height,width),bool); limits=0
        ownership=mesh.color_attributes['Source ownership']
        for yy in range(height):
            sy=y0+yy+.5
            for xx in range(width):
                sx=x0+xx+.5; origin=Vector((sx,-sy/SIN,0))+RAY*10000
                for step in range(256):
                    p,normal,tid,distance=tree.ray_cast(origin,-RAY)
                    if p is None: break
                    tri=tris[tid]; face=mesh.polygons[tri.polygon_index]; slot=face.material_index
                    if slot not in atlases: break
                    data=atlases[slot]
                    if not (data['sided'] and normal.dot(-RAY)>=0):
                        mapped=barycentric_transform(p,*[vertices[i] for i in tri.vertices],
                            *[Vector((*data['uv'].data[i].uv,0)) for i in tri.loops])
                        a=data['rgba']; tx=min(a.shape[1]-1,int((mapped.x%1)*a.shape[1])); ty=min(a.shape[0]-1,int((mapped.y%1)*a.shape[0]))
                        if a[ty,tx,3]>=.5:
                            rgba[yy,xx]=a[ty,tx]; rgba[yy,xx,3]=1
                            points[yy,xx]=p
                            observed[yy,xx]=sx<1408 and min(ownership.data[i].color[0] for i in tri.loops)>.5
                            break
                    origin=p-RAY*.002
                else: limits+=1
            if yy%60==0: print('Native foliage row',yy,flush=True)
        assert limits==0
        known=rgba.copy(); known[~observed,3]=0
        inferred=rgba.copy(); inferred[observed,3]=0
        source_image=atlases[1]['image']
        known_image=make_image('Tree25 exact visible native foliage',known,source_image)
        inferred_image=make_image('Tree25 unobserved frontend foliage',inferred,source_image)
        diagnostic_image=make_image('Tree25 own-native foliage diagnostic',rgba,source_image)
        base=mesh.materials[1]
        new_materials=[make_material(base,'Tree25 small clusters / known source',known_image,True),
            make_material(base,'Tree25 small clusters / inferred front',inferred_image,False,True),
            make_material(base,'Tree25 small clusters / inferred volume',diagnostic_image,False,True)]
        bm=bmesh.new(); bm.from_mesh(mesh); uv=bm.loops.layers.uv['Foliage UV']
        all_uv=list(bm.loops.layers.uv.values()); flags=bm.loops.layers.float_color['Source ownership']; fallback=bm.faces.layers.int['reprojection_fallback_material']
        def signature(f):
            return (f.material_index,tuple((tuple(l.vert.co),tuple(tuple(l[u].uv) for u in all_uv),tuple(l[flags])) for l in f.loops))
        leaves=[f for f in bm.faces if f.material_index in atlases]
        wood=[signature(f) for f in bm.faces if f.material_index not in atlases]
        bmesh.ops.delete(bm,geom=leaves,context='FACES')
        assert [signature(f) for f in bm.faces]==wood
        first_slot=len(mesh.materials)
        for material in new_materials: mesh.materials.append(material)
        inverse=obj.matrix_world.inverted(); rng=random.Random(2503); records=[]
        def patch(box,plane,slot,known_face):
            ax,ay,bx,by=box; cx,cy=(ax+bx)/2,(ay+by)/2
            slope_x,slope_y,center_y=plane
            coords=[]; tex=[]
            for px,py in ((ax,ay),(bx,ay),(bx,by),(ax,by)):
                wy=center_y+slope_x*(px-cx)+slope_y*(py-cy)
                coords.append(Vector((px,wy,(-py-wy*SIN)/COS)))
                tex.append(((px-x0)/width,1-(py-y0)/height))
            vs=[bm.verts.new(inverse@p) for p in coords]
            for indices in ((0,3,2),(0,2,1)):
                face=bm.faces.new([vs[i] for i in indices]); face.material_index=slot; face[fallback]=slot
                for loop,index in zip(face.loops,indices):
                    for layer in all_uv: loop[layer].uv=tex[index]
                    loop[flags]=(1 if known_face else 0,1,1,1)
        size=8
        for yy in range(0,height,size):
            for xx in range(0,width,size):
                sy=slice(yy,min(height,yy+size)); sx=slice(xx,min(width,xx+size))
                visible=rgba[sy,sx,3]>.5
                if not visible.any(): continue
                sample=points[sy,sx][visible]; box=(x0+xx,y0+yy,min(x1,x0+xx+size),min(y1,y0+yy+size))
                # A small front cluster stays at or just in front of its old
                # visible leaf samples, so it cannot fall behind the wood.
                front_y=float(sample[:,1].min())-.02
                if observed[sy,sx].any(): patch(box,(0,0,front_y),first_slot,True)
                if (visible&~observed[sy,sx]).any(): patch(box,(0,0,front_y),first_slot+1,False)
                patch(box,(0,0,front_y+.04),first_slot+2,False)
                count=0
                if visible.sum()>=3 and rng.random()<.60:
                    cx,cy=(box[0]+box[2])/2,(box[1]+box[3])/2
                    rear_limit=min(old_bounds[1][1],(-cy-65*COS)/SIN)
                    available=rear_limit-front_y
                    if available>12:
                        a,b=rng.uniform(-.8,.8),rng.uniform(-.4,.4)
                        extent=abs(a)*(box[2]-box[0])/2+abs(b)*(box[3]-box[1])/2
                        center=min(rear_limit-extent,front_y+available*rng.uniform(.18,.98))
                        center=max(center,front_y+extent+.1)
                        patch(box,(a,b,center),first_slot+2,False); count=1
                records.append(dict(box=box,visible_pixels=int(visible.sum()),observed_pixels=int(observed[sy,sx].sum()),interior_clusters=count))
        assert [signature(f) for f in bm.faces if f.material_index<first_slot]==wood
        bm.normal_update(); bm.to_mesh(mesh); bm.free(); mesh.update()
        assert appearance_state(obj)['materials'][:first_slot]==materials_before
        assert outside=={o.name:_geometry(o,protect_appearance=True) for o in bpy.data.objects if o.name in outside}
        new_points=[obj.matrix_world@v.co for v in mesh.vertices]
        bounds=[[min(p[i] for p in new_points),max(p[i] for p in new_points)] for i in range(3)]
        assert bounds[1][1]-bounds[1][0]>=bounds[0][1]-bounds[0][0],bounds
        out.mkdir(); bpy.ops.wm.save_as_mainfile(filepath=str(out/'worker.blend'),compress=True)
        np.savez_compressed(out/'native-samples.npz',rgba=rgba,observed=observed,points=points)
        report=dict(status='Private whole-canopy cluster candidate; actual/native/contact review pending',
            model_sha256=sha(out/'worker.blend'),source_model_sha256=sha(src),native_bbox=bbox,
            native_foliage_pixels=int((rgba[...,3]>.5).sum()),observed_pixels=int(observed.sum()),
            ray_depth_limits=limits,front_clusters=len(records),interior_clusters=sum(r['interior_clusters'] for r in records),
            original_foliage_faces_removed=len(leaves),wood_faces_preserved=len(wood),outside_objects_preserved=len(outside),
            all_preexisting_materials_images_unchanged=True,before_bounds=old_bounds,after_bounds=bounds,
            permitted_references=[dict(asset_id=r['asset_id'],image=r['parent_image'],sha256=r['parent_sha256']) for r in refs],
            clusters=records,limitations=['Visible source RGB sampled at native pixel centres; full rendered edge/coverage comparison still required.',
                'New physical leaf arrangement and unobserved density are inference and require fresh geometry approval.',
                'Own-source RGB on inferred surfaces is diagnostic, not approved final appearance.',
                'Native known-face identities/hidden source-layer overlap intentionally replaced; wood and wall contact remain protected.'])
        (out/'construction.json').write_text(json.dumps(report,indent=2)+'\n')
        print({k:v for k,v in report.items() if k not in ('clusters','permitted_references')})
    finally:
        release()


if __name__=='__main__': main()
