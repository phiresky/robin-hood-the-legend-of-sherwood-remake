"""Project generated appearance onto inferred faces, preserving native front materials."""
import hashlib,json,math,sys,time,shutil
from pathlib import Path
HERE=Path(__file__).resolve().parent;sys.path.insert(0,str(HERE))
import build_models as b
from operational_records import read_record
from render_slots import acquire,release
import numpy as np
from PIL import Image
from mathutils import Matrix,Vector
from mathutils.bvhtree import BVHTree
ROOT=b.q.ROOT;BASE=b.q.OUT/'approved03-04-textures-v1'
GEN='generation-short-no-mask-with-lighting-openrouter-with-auxiliary'
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def write(p,d):p.write_text(json.dumps(d,indent=2)+'\n')
def available():return int(next(x.split()[1] for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:')))*1024

def geometry(o):
    m=o.data
    return hashlib.sha256(repr(([tuple(v.co) for v in m.vertices],[tuple(p.vertices) for p in m.polygons],[list(r) for r in o.matrix_world],[[tuple(t.uv) for t in u.data] for u in m.uv_layers if u.name!='Generated inferred projection'])).encode()).hexdigest()
def image_hash(image):
    a=np.empty(len(image.pixels),np.float32);image.pixels.foreach_get(a);return hashlib.sha256(a.tobytes()).hexdigest()
def observed(o):
    if o.get('inferred_support'):return None
    m=o.data;mat=m.materials[0]
    return dict(faces=[p.index for p in m.polygons if p.material_index==0],material=mat.name,images={n.image.name:image_hash(n.image) for n in mat.node_tree.nodes if n.type=='TEX_IMAGE' and n.image},uv=[[tuple(u.data[i].uv) for p in m.polygons if p.material_index==0 for i in p.loop_indices] for u in m.uv_layers if u.name!='Generated inferred projection'])
def preflight(state,experiment=None):
    exp=Path(experiment) if experiment is not None else BASE/'experiments-v2'/state/'experiment'
    review=json.loads((exp/GEN/'generation-review.json').read_text()) if experiment is not None else read_record(exp/GEN/'generation-review.json')
    for p,h in review['pins'].items():assert sha(p)==h,p
    assert review['ready_for_bake'];return exp,review

def main(state,experiment=None,destination=None):
    exp,review=preflight(state,experiment);dest=Path(destination) if destination is not None else BASE/'baked-v1'/state;assert not dest.exists();dest.mkdir(parents=True)
    manifest=json.loads((exp/'views.json').read_text()) if experiment is not None else read_record(exp/'views.json');b.bpy.ops.wm.open_mainfile(filepath=str(exp/'approved-model.blend'));scene=b.bpy.context.scene
    allobjects=[o for o in scene.objects if o.type=='MESH'];targets=[scene.objects[n] for n in manifest['object_names']]
    before={o.name:geometry(o) for o in allobjects};known_before={o.name:observed(o) for o in allobjects}
    original_indices={o.name:[p.material_index for p in o.data.polygons] for o in allobjects}
    scope={n:set(f) for n,f in manifest['texture_receiver_face_indices'].items()}
    vertices=[];triangles=[];owners=[]
    for o in targets:
        o.data.calc_loop_triangles();offset=len(vertices);vertices.extend(o.matrix_world@v.co for v in o.data.vertices)
        triangles.extend(tuple(offset+i for i in t.vertices) for t in o.data.loop_triangles);owners.extend((o.name,t.polygon_index) for t in o.data.loop_triangles)
    tree=BVHTree.FromPolygons(vertices,triangles,all_triangles=True)
    mask=np.array(Image.open(exp/'mask.png').convert('RGBA'))[:,:,3]==0
    generated=b.bpy.data.images.load(str(exp/GEN/'generated-preserved.png'),check_existing=False);generated.pack()
    cameras=[]
    for view in manifest['views']:
        mat=Matrix(view['camera_matrix_world']);cameras.append((view,mat.inverted(),mat.col[2].to_3d().normalized()))
    report=[]
    for o in targets:
        mesh=o.data;uv=mesh.uv_layers.new(name='Generated inferred projection')
        # Leave every preexisting UV layer and native material slot untouched.
        mat=b.bpy.data.materials.new(o.name+' generated inferred appearance');mat.use_nodes=True;nodes=mat.node_tree.nodes;nodes.clear();out=nodes.new('ShaderNodeOutputMaterial');em=nodes.new('ShaderNodeEmission');tex=nodes.new('ShaderNodeTexImage');tex.image=generated;tex.interpolation='Closest';tex.extension='CLIP';uvnode=nodes.new('ShaderNodeUVMap');uvnode.uv_map=uv.name
        links=mat.node_tree.links;links.new(uvnode.outputs['UV'],tex.inputs['Vector']);links.new(tex.outputs['Color'],em.inputs['Color']);links.new(em.outputs[0],out.inputs[0]);mat['generated_source_sha256']=sha(exp/GEN/'generated-preserved.png');mat['texture_review_status']='candidate-review-pending';mesh.materials.append(mat);slot=len(mesh.materials)-1
        assigned=[];unseen=[];views_used={str(i):0 for i in range(8)}
        for face in mesh.polygons:
            if face.index not in scope[o.name]:continue
            points=[o.matrix_world@mesh.vertices[i].co for i in face.vertices];center=sum(points,Vector())/len(points);normal=(o.matrix_world.to_3x3().inverted().transposed()@face.normal).normalized()
            witnesses=[center]+[center*.4+p*.6 for p in points]
            chosen=None
            for score,view,inverse,direction in sorted([(normal.dot(d),v,m,d) for v,m,d in cameras],key=lambda r:-r[0]):
                if score<=.05:continue
                crop=view['crop'];witness=None
                for point in witnesses:
                    local=inverse@point;x=crop['left']+(.5+local.x/view['ortho_scale'])*256;y=crop['top']+(.5-local.y/view['ortho_scale'])*256
                    ix,iy=math.floor(x),math.floor(y)
                    if not(crop['left']<=ix<crop['left']+256 and crop['top']<=iy<crop['top']+256) or not mask[iy,ix]:continue
                    hit,_,triangle,_=tree.ray_cast(point+direction*4000,-direction)
                    if hit is None or owners[triangle]!=(o.name,face.index) or (hit-point).length>.01:continue
                    witness=[float(x),float(y)];break
                if witness is not None:chosen=(view,inverse,witness);break
            if chosen is None:unseen.append(face.index);continue
            view,inverse,witness=chosen;crop=view['crop']
            for i in face.loop_indices:
                p=inverse@(o.matrix_world@mesh.vertices[mesh.loops[i].vertex_index].co)
                uv.data[i].uv=((crop['left']+(.5+p.x/view['ortho_scale'])*256)/1024,1-(crop['top']+(.5-p.y/view['ortho_scale'])*256)/512)
            face.material_index=slot;assigned.append(face.index);views_used[str(view['index'])]+=1
        report.append(dict(object=o.name,inferred_faces=len(scope[o.name]),generated_faces=len(assigned),unseen_faces=len(unseen),unseen_face_indices=unseen,views=views_used))
        print(state,report[-1]['object'],'generated',len(assigned),'unseen',len(unseen),flush=True)
    assert before=={o.name:geometry(o) for o in allobjects}
    assert known_before=={o.name:observed(o) for o in allobjects}
    for o in allobjects:
        for p in o.data.polygons:
            if p.index not in scope.get(o.name,set()):assert p.material_index==original_indices[o.name][p.index]
    b.bpy.context.preferences.filepaths.save_version=0;b.bpy.ops.wm.save_as_mainfile(filepath=str(dest/'model.blend'),compress=True)
    b.bpy.ops.wm.open_mainfile(filepath=str(dest/'model.blend'));allobjects=[o for o in b.bpy.context.scene.objects if o.type=='MESH']
    assert before=={o.name:geometry(o) for o in allobjects};assert known_before=={o.name:observed(o) for o in allobjects}
    write(dest/'bake-validation.json',dict(status='SAVED_MODEL_SOURCE_GEOMETRY_PRESERVED_APPEARANCE_REVIEW_PENDING',model_sha256=sha(dest/'model.blend'),source_model_sha256=sha(exp/'approved-model.blend'),generation_review_sha256=sha(exp/GEN/'generation-review.json'),geometry=before,source_materials_and_original_uv_exact=True,physical_geometry_and_opacity_unchanged=True,objects=report,method='Each inferred polygon uses its best-facing fixed approved camera with an editable first-hit witness on that exact polygon. UV projection into locally source-preserved generated sheet; no native front slot/UV edits.',limits=['Unseen polygons retain their original gray material and are enumerated; inspect actual saved views before concluding fill coverage.','Generated appearance approval and native integration pending.']))
    print('ARCHER_TEXTURE_BAKE_COMPLETE',state,flush=True)

if __name__=='__main__':
    state=sys.argv[sys.argv.index('--')+1];preflight(state)
    while True:
        while available()<11*1024**3:print('Waiting 11 GiB for archer texture bake',flush=True);time.sleep(30)
        assert shutil.disk_usage(ROOT).free>=10*1024**3
        acquire()
        try:
            if available()<10*1024**3:continue
            main(state);break
        finally:release()
