"""Recover native colors on newly visible inferred fronts without changing alpha."""
import sys,json,hashlib,math
from pathlib import Path
from collections import defaultdict
import bpy,numpy as np
from PIL import Image,ImageDraw
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from tree_geometry import SIN,COS,RAY
from render_slots import acquire,release
from sign_source_raster import raster
from restart2_sign_neighbors import camera_to,render

def geometry(obj):
    h=hashlib.sha256();mesh=obj.data
    for value in [np.array(obj.matrix_world),np.array([v.co[:] for v in mesh.vertices]),np.array([l.vertex_index for l in mesh.loops]),np.array([(p.loop_start,p.loop_total) for p in mesh.polygons])]:h.update(value.tobytes())
    for uv in mesh.uv_layers:h.update(uv.name.encode());h.update(np.array([v.uv[:] for v in uv.data]).tobytes())
    for c in mesh.color_attributes:h.update(c.name.encode());h.update(np.array([v.color[:] for v in c.data]).tobytes())
    return h.hexdigest()

def main():
    dest=OUT/'restart2-fence/shrub57-sign-bend-v7';dest.mkdir(exist_ok=False)
    original=OUT/'understory-round-9/assets/croisement02-shrub-57/model.blend';base=OUT/'restart2-fence/shrub57-sign-bend-v6/model.blend';box=[-42,200,116,390]
    bpy.ops.wm.open_mainfile(filepath=str(original));expected,known=raster(bpy.data.objects['West Rock Foliage 57'],box)
    bpy.ops.wm.open_mainfile(filepath=str(base));obj=bpy.data.objects['West Rock Foliage 57'];guard=geometry(obj);actual,roles,depth,faces=raster(obj,box,with_details=True)
    assert np.array_equal(expected[:,:,3],actual[:,:,3]);diff=np.any(abs(expected-actual)>1e-5,axis=2);assert np.all(known[diff]==1) and np.all(roles[diff]==2)
    mesh=obj.data;mesh.calc_loop_triangles();uv=mesh.uv_layers['Foliage UV'];world=np.array([tuple(obj.matrix_world@v.co) for v in mesh.vertices]);projected=np.column_stack((world[:,0],-SIN*world[:,1]-COS*world[:,2]));triangles=defaultdict(list)
    for t in mesh.loop_triangles:triangles[t.polygon_index].append(t)
    receipts=[]
    for face in sorted(set(faces[diff])):
        face=int(face);polygon=mesh.polygons[face];material=mesh.materials[polygon.material_index];texture=next(n for n in material.node_tree.nodes if n.type=='TEX_IMAGE' and n.image);image=texture.image;w,h=image.size;pixels=np.array(image.pixels[:],dtype=np.float32).reshape(h,w,4)
        points=[]
        for y,x in zip(*np.nonzero(faces==face)):
            source=np.array([x+box[0]+.5,y+box[1]+.5]);found=None
            for t in triangles[face]:
                a,b,c=projected[list(t.vertices)];basis=np.column_stack((b-a,c-a))
                if abs(np.linalg.det(basis))<1e-10:continue
                bc=np.linalg.solve(basis,source-a)
                if min(bc)>=-1e-6 and bc.sum()<=1+1e-6:found=np.array([1-bc.sum(),*bc])@np.array([uv.data[l].uv[:] for l in t.loops]);break
            assert found is not None
            points.append((int(x),int(y),found,expected[y,x] if diff[y,x] else actual[y,x]))
        for factor in [1,2,4,8]:
            ownership={};conflict=False
            for x,y,tuv,color in points:
                ix,iy=np.floor(tuv*[w*factor,h*factor]).astype(int);key=(int(ix%(w*factor)),int(iy%(h*factor)))
                if key in ownership and not np.allclose(ownership[key],color,atol=1e-5):conflict=True;break
                ownership[key]=color
            if not conflict:break
        else:raise ValueError('Native color samples collide in bounded UV atlas')
        expanded=np.repeat(np.repeat(pixels,factor,axis=0),factor,axis=1);changed=[]
        for x,y,tuv,color in points:
            if not diff[y,x]:continue
            ix,iy=np.floor(tuv*[w*factor,h*factor]).astype(int);ix=int(ix%(w*factor));iy=int(iy%(h*factor))
            expanded[iy,ix,:3]=color[:3];changed.append(dict(source_pixel=[x+box[0],y+box[1]],texture_pixel=[ix,iy],native_rgba=color.tolist()))
        assert np.array_equal(expanded[:,:,3],np.repeat(np.repeat(pixels[:,:,3],factor,axis=0),factor,axis=1))
        clone=bpy.data.images.new(f'Native first-hit recovery face {face}',width=w*factor,height=h*factor,alpha=True,float_buffer=False);clone.colorspace_settings.name=image.colorspace_settings.name;clone.pixels.foreach_set(expanded.ravel());clone.pack()
        replacement=material.copy();replacement.name=material.name+f' / native front recovery {face}'
        next(n for n in replacement.node_tree.nodes if n.type=='TEX_IMAGE' and n.image).image=clone
        replacement['native_front_reprojection']=True;replacement['native_front_reprojection_face']=face
        replacement['native_front_reprojection_pixels']=json.dumps(changed)
        replacement['native_front_reprojection_semantics']='Only recorded RGB texels are newly source-observed; remaining material texels retain inferred provenance. Alpha and shader structure unchanged.'
        mesh.materials.append(replacement);polygon.material_index=len(mesh.materials)-1
        receipts.append(dict(face=face,material=replacement.name,source_image=image.name,new_image=clone.name,factor=factor,alpha_function_unchanged=True,changed_native_pixels=changed))
    assert geometry(obj)==guard
    after,after_roles=raster(obj,box)
    assert np.array_equal(expected[:,:,3],after[:,:,3]);error=float(np.max(abs(expected-after)));assert error<1e-5,error
    bpy.ops.wm.save_as_mainfile(filepath=str(dest/'model.blend'));digest=sha(dest/'model.blend')
    bpy.ops.wm.open_mainfile(filepath=str(dest/'model.blend'));obj=bpy.data.objects['West Rock Foliage 57'];assert geometry(obj)==guard;after,_=raster(obj,box);assert np.max(abs(expected-after))<1e-5
    scene=bpy.context.scene;scene.render.use_compositing=False;scene.render.resolution_x=384;scene.render.resolution_y=384;camera=scene.camera;camera.data.ortho_scale=192
    camera_to(camera,Vector((38,-284/SIN,0)),RAY);render(scene,dest/'native-after.png')
    points=np.array([tuple(obj.matrix_world@v.co) for v in obj.data.vertices]);center=Vector((points.min(0)+points.max(0))/2);camera.data.ortho_scale=250;sheet=Image.new('RGB',(1536,816),(65,65,65))
    for i in range(8):
        angle=i*math.pi/4;camera_to(camera,center,Vector((math.sin(angle)*COS,-math.cos(angle)*COS,SIN)));pic=render(scene,dest/f'actual-{i}.png');sheet.paste(pic,(i%4*384,i//4*408),pic.getchannel('A'));ImageDraw.Draw(sheet).text((i%4*384+4,i//4*408+386),f'Actual saved materials {i}',fill='white')
    sheet.save(dest/'actual-eight.png')
    write_json(dest/'report.json',dict(status='Private native-color recovery; independent review and complete sign proof pending',model_sha256=digest,geometry_base_sha256=sha(base),original_source_sha256=sha(original),geometry_uv_ownership_sha256=guard,geometry_uv_ownership_unchanged=True,alpha_function_unchanged=True,source_crop=box,native_first_hit_rgba_max_error=error,patched_native_pixels=int(diff.sum()),patches=receipts,limitations=['Recorded RGB texels are source-observed additions on previously inferred fronts. Whole faces are not relabeled as observed.','Future texture edits must preserve the recorded native texels; automatic texture-fill integration is not claimed.','All geometry and physical alpha proofs may be bound through the exact preservation receipt; fresh actual-material review remains required.','No user approval inheritance, selector mutation or publication.']))
    print(dest)
if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
