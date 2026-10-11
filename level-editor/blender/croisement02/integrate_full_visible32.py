"""Build an unapproved whole-wood Tree32 hypothesis from a pinned CPU study.

Source ownership and every other asset stay frozen. This deliberately replaces
unsupported upper-limb depth instead of grafting a round stem to a flat limb.
Saved source, terrain, full-tree context and visual review remain required.
"""
import argparse
import hashlib
import json
import shutil
import sys
from pathlib import Path
import bpy
import bmesh
import numpy as np
from mathutils import Matrix,Vector

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(Path(__file__).parent), str(ROOT/'level-editor/refinement')]
from render_slots import acquire, release
from integrate_wood_field import protected_state, mesh_report
BASE = ROOT/'level-editor/work/croisement02-refinement'
MODEL = BASE/'root-stem-round-3/assets/croisement02-tree-32/model.blend'
MODEL_SHA = '00ab00761141d7c4c564331745ae680afef0fd20edd40e110c9301ec986f185b'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--mesh',type=Path,required=True)
    parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args(sys.argv[sys.argv.index('--')+1:])
    if args.output.exists():raise FileExistsError(args.output)
    report_path=args.mesh.parent/'report.json'
    report=json.loads(report_path.read_text())
    if report['tree']!=32 or sha(MODEL)!=MODEL_SHA:raise ValueError('Wrong or changed source model')
    source=Path(report['source_mask'])
    if sha(source)!=report['source_sha256']:raise ValueError('Observed source mask changed')
    if report['observed_source_ground']['missing_body_centers']!=0:raise ValueError('Observed source samples remain uncovered')
    from physical_tree32_reviewed_domain_cpu import reviewed_source,BASE as DOMAIN
    _,reviewed_path=reviewed_source(32,report['source_box'])
    if source.resolve()!=reviewed_path.resolve():raise ValueError('Expected reviewed Tree32 source domain')
    domain_proposal=json.loads((DOMAIN/'proposal.json').read_text())
    mesh_sha=sha(args.mesh)
    arrays=np.load(args.mesh,allow_pickle=False)
    vertices,faces=arrays['vertices'],arrays['faces']
    acquire(slots=2)
    try:
        memory=int(next(line.split()[1]for line in Path('/proc/meminfo').read_text().splitlines()if line.startswith('MemAvailable:')))*1024
        if memory<6*1024**3 or shutil.disk_usage(BASE).free<10*1024**3:raise RuntimeError('Resource guard stopped worker before scene load')
        bpy.ops.wm.open_mainfile(filepath=str(MODEL));bpy.context.view_layer.update()
        bpy.context.preferences.filepaths.save_version=0
        objects=list(bpy.data.collections['Croisement02 Working'].all_objects)
        wood=[o for o in objects if o.type=='MESH' and o.get('asset_group')=='croisement02-tree-32' and o.get('projection_component')!='crown']
        if not wood or any(o.get('source_node') not in ('building-080','building-081','building-082') for o in wood):raise ValueError('Whole-wood source ownership changed')
        primary=next(o for o in wood if o.get('source_node')=='building-080')
        others=[o for o in objects if o.type=='MESH' and o not in wood]
        protected=protected_state(others)
        # Keep map placement in the object transform. Blender's polygon-area
        # arithmetic loses small valid triangles when all coordinates include
        # the large map offset; this exact float32 rebase changes no world point.
        anchor=np.array([1090.,-1298.,0.],dtype=np.float32)
        world=vertices.astype(np.float32);local=world-anchor
        if not np.array_equal(local+anchor,world):raise ValueError('Local rebase changes world coordinates')
        bm=bmesh.new();points=[bm.verts.new(point)for point in local]
        for indices in faces:
            face=bm.faces.new([points[int(i)]for i in indices]);face.smooth=True
        bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
        topology=mesh_report(bm)
        if topology['nonmanifold_edges'] or topology['degenerate_faces']:raise ValueError(topology)
        unseen=set(bm.verts);components=0
        while unseen:
            components+=1;queue=[unseen.pop()]
            while queue:
                vertex=queue.pop()
                for edge in vertex.link_edges:
                    other=edge.other_vert(vertex)
                    if other in unseen:unseen.remove(other);queue.append(other)
        if components!=1:raise ValueError(f'Disconnected wood volumes: {components}')
        data=bpy.data.meshes.new('Private full visible Tree32 wood');bm.to_mesh(data);bm.free()
        neutral=bpy.data.materials.new('Unprojected field wood32 full visible');neutral.diffuse_color=(.4,.4,.4,1.)
        data.materials.append(neutral)
        primary.data=data;primary.parent=None;primary.matrix_world=Matrix.Translation(Vector(anchor))
        for old_wood in wood:
            if old_wood!=primary:bpy.data.objects.remove(old_wood,do_unlink=True)
        bpy.context.view_layer.update()
        if not np.array_equal(np.asarray([primary.matrix_world@v.co for v in data.vertices]),world):raise ValueError('Object placement changes CPU world coordinates')
        if protected!=protected_state(others):raise ValueError('Protected crown or another asset changed')
        args.output.mkdir(parents=True)
        packet={'status':'PRIVATE full visible wood construction; not approved','records':[{'tree':32,'input_model':str(MODEL),'input_model_sha256':MODEL_SHA,'source_node':'building-080','source_mask':str(source),'source_mask_sha256':sha(source),'source_box':report['source_box'],'unresolved_source_coordinates':domain_proposal['changed_pixels'],'construction_scope':'Replace complete private wood080–082; preserve source ownership, other assets and crown','cpu_mesh':str(args.mesh.resolve()),'cpu_mesh_sha256':mesh_sha,'cpu_report':str(report_path.resolve()),'cpu_report_sha256':sha(report_path)}]}
        packet_path=args.output/'recipe.json';packet_path.write_text(json.dumps(packet,indent=2)+'\n')
        destination=args.output/'model.blend';bpy.ops.wm.save_as_mainfile(filepath=str(destination),compress=True)
        if sha(MODEL)!=MODEL_SHA or sha(args.mesh)!=mesh_sha:raise ValueError('Input changed during integration')
        evidence={'status':'PRIVATE HOLD: full wood hypothesis, source projection and context pending','model_sha256':sha(destination),'input_model':str(MODEL),'input_model_sha256':MODEL_SHA,'packet':str(packet_path.resolve()),'packet_sha256':sha(packet_path),'mesh':topology,'connected_components':components,'protected':protected,'source_mask_unchanged':True,'local_mesh_origin':anchor.tolist(),'world_float32_coordinates_unchanged_by_rebase':True,'retained_upper_geometry_changed':True,'inference':'Complete visible wood depth rebuilt; observed source pixels remain the constraint. No previous geometry approval is carried.','gallery_ready':False}
        (args.output/'evidence.json').write_text(json.dumps(evidence,indent=2)+'\n')
        print(json.dumps({k:v for k,v in evidence.items()if k!='protected'},indent=2))
    finally:release()


if __name__=='__main__':main()
