"""Restore inferred off-map twig visibility without altering native source surfaces."""
import json,sys,shutil,math
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from tree_geometry import SIN,COS,material,one_sided,replace_mesh
from render_slots import acquire,release
BASE=OUT/'restart14-hidden-archer';OLD=BASE/'climbing-v17';DEST=BASE/'climbing-v18';CAP=32*1024**2

def budget():
    used=sum(p.stat().st_size for p in DEST.rglob('*') if p.is_file()) if DEST.exists() else 0
    assert used<CAP and shutil.disk_usage(BASE).free>=8*1024**3+CAP-used,'Small lane reserve violated'

def clip(vertices,outside):
    def y(v):return -v[0][1]*SIN-v[0][2]*COS
    result=[]
    for a,b in zip(vertices,vertices[1:]+vertices[:1]):
        ya,yb=y(a),y(b);ina=ya<=0 if outside else ya>=0;inb=yb<=0 if outside else yb>=0
        if ina:result.append(a)
        if ina!=inb:
            t=ya/(ya-yb);result.append((a[0]+(b[0]-a[0])*t,a[1]+(b[1]-a[1])*t))
    return result

def area(poly):
    return sum(np.linalg.norm(np.cross(poly[i][0]-poly[0][0],poly[i+1][0]-poly[0][0]))/2 for i in range(1,len(poly)-1))

def main():
    budget();assert not DEST.exists();proofpath=OLD/'readonly-role-support-v2/report.json';proof=json.loads(proofpath.read_text());DEST.mkdir();gray=DEST/'inferred-gray.png';Image.new('RGBA',(1,1),(158,158,158,255)).save(gray)
    for state in ['initial','applied']:
        budget();previous=OLD/f'profile-05-{state}';record=json.loads((previous/'construction.json').read_text());audit=next(r for r in proof['records'] if r['state']==state);oldhash=sha(previous/'model.blend');assert oldhash==record['model_sha256']==audit['model_sha256'];assert record['source_top_left'][1]==0
        bpy.ops.wm.open_mainfile(filepath=str(previous/'model.blend'));bpy.context.preferences.filepaths.save_version=0;objects=[o for o in bpy.context.scene.objects if o.type=='MESH'];assert len(objects)==1;obj=objects[0];old=obj.data;native_faces=2*record['source_opaque_centers'];stem_end=native_faces+10*audit['planned_support_graph']['stem_segments'];mats=list(old.materials);inferred=material('Explicit inferred opaque off-map supporting wood',gray,False);one_sided(inferred);mats.append(inferred)
        vertices=[];faces=[];uvs=[];slots=[];known=[];changes=[];native_signature=[]
        for face in old.polygons:
            points=[(np.array(obj.matrix_world@old.vertices[old.loops[l].vertex_index].co),np.array(old.uv_layers['Foliage UV'].data[l].uv)) for l in face.loop_indices]
            ownership=[old.color_attributes['Source ownership'].data[l].color[0] for l in face.loop_indices];assert max(ownership)-min(ownership)<1e-7
            is_stem=native_faces<=face.index<stem_end
            if is_stem:assert face.material_index in (1,2) and max(ownership)==0
            sy=[-p[0][1]*SIN-p[0][2]*COS for p in points]
            pieces=[(points,face.material_index)]
            if is_stem and min(sy)<-1e-6:
                inside=clip(points,False);outside=clip(points,True);pieces=[(p,slot) for p,slot in [(inside,face.material_index),(outside,len(mats)-1)] if len(p)>=3 and area(p)>1e-10]
                error=abs(sum(area(p) for p,_ in pieces)-area(points));assert error<1e-4
                changes.append(dict(original_polygon=face.index,original_material=face.material_index,source_y_range=[min(sy),max(sy)],area_preservation_error=error,pieces=len(pieces)))
            for polygon,slot in pieces:
                start=len(vertices);vertices.extend(p.tolist() for p,u in polygon);uvs.extend(u.tolist() for p,u in polygon);faces.append(tuple(range(start,start+len(polygon))));slots.append(slot);known.append(bool(ownership[0]))
        shape=replace_mesh(obj,vertices,faces,uvs,mats,slots,known)
        # Every observed leaf polygon is retained before the stem-only region.
        for before,after in zip(old.polygons[:native_faces],obj.data.polygons[:native_faces]):
            assert before.material_index==after.material_index and len(before.vertices)==len(after.vertices)
            for bl,al in zip(before.loop_indices,after.loop_indices):
                bv=old.vertices[old.loops[bl].vertex_index].co;av=obj.data.vertices[obj.data.loops[al].vertex_index].co
                assert (bv-av).length<1e-6
                assert (old.uv_layers['Foliage UV'].data[bl].uv-obj.data.uv_layers['Foliage UV'].data[al].uv).length<1e-7
        folder=DEST/f'profile-05-{state}';folder.mkdir();budget();bpy.ops.wm.save_as_mainfile(filepath=str(folder/'model.blend'),compress=True);budget();newhash=sha(folder/'model.blend');new=dict(record);new.update(model_sha256=newhash,shape=shape,status='Private inferred off-map support-opacity correction; geometry/native/context review pending');write_json(folder/'construction.json',new)
        write_json(folder/'preservation.json',dict(previous_model=str(previous/'model.blend'),previous_model_sha256=oldhash,model_sha256=newhash,changed_stem_polygons=changes,observed_paired_leaf_faces_unchanged=native_faces,source_sha256=record['source_sha256'],source_image_unchanged=True,scope='Split existing inferred stem surfaces at native top boundary; only off-map pieces receive opaque inferred gray. In-map surface material/UV and every observed leaf face unchanged.',clearance_inherited_from_unchanged_surface_footprint=True,role_audit_sha256=sha(proofpath)))
        assert sha(previous/'model.blend')==oldhash
    from restart14_hidden_archer_review_v12 import main as review
    review(DEST)
if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
