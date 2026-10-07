"""Build a bounded private pair from exact native faces and guarded attached foliage.

Requires an explicit small-lane grant. No texture generation or publication.
"""
import json,sys,shutil
from pathlib import Path
import bpy,numpy as np
from PIL import Image
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from tree_geometry import material,one_sided,replace_mesh,RAY
from render_slots import acquire,release
from restart18_hidden_archer_support_guard import tube
BASE=OUT/'restart14-hidden-archer';OLD=BASE/'climbing-v17';DEST=BASE/'climbing-v18-compact';CAP=32*1024**2

def budget(*unused):
    used=sum(p.stat().st_size for p in DEST.rglob('*') if p.is_file()) if DEST.exists() else 0
    assert used<CAP and shutil.disk_usage(BASE).free>=10*1024**3+CAP-used,'Bounded construction reserve violated'

def main():
    budget();assert not DEST.exists();packet_path=OLD/'compact-fans-cpu-v4/report.json';packet=json.loads(packet_path.read_text());assert packet['status']=='CPU EXACT KNOWN-SOURCE CONSTRUCTION RECIPE';
    for path,digest in packet['inputs'].items():assert sha(Path(path))==digest
    DEST.mkdir();gray=DEST/'inferred-gray.png';Image.new('RGBA',(1,1),(158,158,158,255)).save(gray)
    for state in packet['states']:
        name=state['state'];previous=OLD/f'profile-05-{name}';record=json.loads((previous/'construction.json').read_text());oldhash=sha(previous/'model.blend');assert oldhash==record['model_sha256'];budget();bpy.ops.wm.open_mainfile(filepath=str(previous/'model.blend'));bpy.context.preferences.filepaths.save_version=0;objects=[o for o in bpy.context.scene.objects if o.type=='MESH'];assert len(objects)==1;obj=objects[0];old=obj.data;native_faces=record['source_opaque_centers']*2;mats=list(old.materials);inferred=material('Explicit inferred opaque local branch and leaf geometry',gray,False);inferred['texture_provenance']='Untextured inferred gray, no observed or generated RGB claim';one_sided(inferred);mats.append(inferred);slot=len(mats)-1;vertices=[];faces=[];uvs=[];slots=[];known=[]
        def add(poly,mat_slot,owned=False,coords=None):
            start=len(vertices);vertices.extend(np.asarray(poly).tolist());faces.append(tuple(range(start,start+len(poly))));uvs.extend(coords if coords is not None else [[.5,.5]]*len(poly));slots.append(mat_slot);known.append(owned)
        for face in old.polygons[:native_faces]:
            poly=[obj.matrix_world@old.vertices[old.loops[l].vertex_index].co for l in face.loop_indices];coords=[list(old.uv_layers['Foliage UV'].data[l].uv) for l in face.loop_indices];ownership=[old.color_attributes['Source ownership'].data[l].color[0] for l in face.loop_indices];assert len(set(ownership))==1;add(poly,face.material_index,bool(ownership[0]),coords)
        for segment in state['candidate_segments']+state['offmap_continuations']:
            for tri in tube(np.array(segment['start']),np.array(segment['end']),segment['radius']):add(tri,slot)
        for leaf in state['leaf_blades']:
            poly=np.array(leaf['polygon']);normal=np.cross(poly[1]-poly[0],poly[2]-poly[0]);poly=poly if normal@np.array(RAY)>0 else poly[::-1];add(poly,slot);add(poly[::-1]-np.array(RAY)*leaf['paired_back_ray_offset'],slot)
        shape=replace_mesh(obj,vertices,faces,uvs,mats,slots,known)
        for before,after in zip(old.polygons[:native_faces],obj.data.polygons[:native_faces]):
            assert before.material_index==after.material_index and len(before.vertices)==len(after.vertices)
            for bl,al in zip(before.loop_indices,after.loop_indices):
                assert tuple(old.vertices[old.loops[bl].vertex_index].co)==tuple(obj.data.vertices[obj.data.loops[al].vertex_index].co)
                assert tuple(old.uv_layers['Foliage UV'].data[bl].uv)==tuple(obj.data.uv_layers['Foliage UV'].data[al].uv)
                assert tuple(old.color_attributes['Source ownership'].data[bl].color)==tuple(obj.data.color_attributes['Source ownership'].data[al].color)
        folder=DEST/f'profile-05-{name}';folder.mkdir();budget();bpy.ops.wm.save_as_mainfile(filepath=str(folder/'model.blend'),compress=True);budget();new=dict(record);new.update(model_sha256=sha(folder/'model.blend'),shape=shape,stem_guard_sha256=sha(OLD/'compact-support-guard-v7/report.json'),leaf_guard_sha256=sha(packet_path),status='PRIVATE compact physically connected climbing candidate; visual/source/context review pending');write_json(folder/'construction.json',new);write_json(folder/'preservation.json',dict(previous_model_sha256=oldhash,model_sha256=new['model_sha256'],native_faces_byte_exact=native_faces,source_sha256=record['source_sha256'],old_inferred_geometry_removed=True,new_inferred_leaf_blades=len(state['leaf_blades']),new_support_segments=len(state['candidate_segments'])+len(state['offmap_continuations']),cpu_packet_sha256=sha(packet_path),native_empty_subpixel_inference=state['source_guard']['native_empty_conflicts'],limits=['Tiny source-separated native leaf islands use explicitly inferred subpixel twigs; inspect native-scale and contact views.','Gray inferred leaves/support are not texture complete.','Every local rock attachment is inferred; whole-neighbourhood morphology remains unapproved.']));assert sha(previous/'model.blend')==oldhash
    import restart14_hidden_archer_review_v12 as review
    review.small_budget=budget;review.main(DEST)
    import restart17_hidden_archer_contact as contact
    contact.ROUND=DEST;contact.DEST=DEST/'rock-bank-contact-v1';contact.budget=budget;contact.main()
if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
