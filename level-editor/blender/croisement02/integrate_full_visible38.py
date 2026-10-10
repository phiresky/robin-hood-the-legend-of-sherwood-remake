"""Build an unapproved whole-wood Tree38 hypothesis from a pinned CPU study.

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
from mathutils import Matrix

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(Path(__file__).parent), str(ROOT/'level-editor/refinement')]
from render_slots import acquire, release
from integrate_wood_field import protected_state, mesh_report
BASE = ROOT/'level-editor/work/croisement02-refinement'
MODEL = BASE/'root-stem-round-2/assets/croisement02-tree-38/model.blend'
MODEL_SHA = 'b245e87e24341e78415567d31327f353d575bb988da597ae34b8a06178fab054'


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
    if report['tree']!=38 or sha(MODEL)!=MODEL_SHA:raise ValueError('Wrong or changed source model')
    source=Path(report['source_mask'])
    if sha(source)!=report['source_sha256']:raise ValueError('Observed source mask changed')
    if report['observed_source_ground']['missing_body_centers']!=0:raise ValueError('Observed source samples remain uncovered')
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
        wood=[o for o in objects if o.type=='MESH' and o.get('asset_group')=='croisement02-tree-38' and o.get('projection_component')!='crown']
        if len(wood)!=1 or wood[0].get('source_node')!='building-094':raise ValueError('Whole-wood source ownership changed')
        primary=wood[0]
        others=[o for o in objects if o.type=='MESH' and o!=primary]
        protected=protected_state(others)
        bm=bmesh.new();points=[bm.verts.new(point)for point in vertices]
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
        data=bpy.data.meshes.new('Private full visible Tree38 wood');bm.to_mesh(data);bm.free()
        neutral=bpy.data.materials.new('Unprojected field wood38 full visible');neutral.diffuse_color=(.4,.4,.4,1.)
        data.materials.append(neutral)
        primary.data=data;primary.parent=None;primary.matrix_world=Matrix.Identity(4)
        if protected!=protected_state(others):raise ValueError('Protected crown or another asset changed')
        args.output.mkdir(parents=True)
        packet={'status':'PRIVATE full visible wood construction; not approved','records':[{'tree':38,'input_model':str(MODEL),'input_model_sha256':MODEL_SHA,'source_node':'building-094','source_mask':str(source),'source_mask_sha256':sha(source),'source_box':report['source_box'],'unresolved_source_coordinates':[],'construction_scope':'Replace complete private wood094; preserve source ownership, other assets and crown','cpu_mesh':str(args.mesh.resolve()),'cpu_mesh_sha256':mesh_sha,'cpu_report':str(report_path.resolve()),'cpu_report_sha256':sha(report_path)}]}
        packet_path=args.output/'recipe.json';packet_path.write_text(json.dumps(packet,indent=2)+'\n')
        destination=args.output/'model.blend';bpy.ops.wm.save_as_mainfile(filepath=str(destination),compress=True)
        if sha(MODEL)!=MODEL_SHA or sha(args.mesh)!=mesh_sha:raise ValueError('Input changed during integration')
        evidence={'status':'PRIVATE HOLD: full wood hypothesis, source projection and context pending','model_sha256':sha(destination),'input_model':str(MODEL),'input_model_sha256':MODEL_SHA,'packet':str(packet_path.resolve()),'packet_sha256':sha(packet_path),'mesh':topology,'connected_components':components,'protected':protected,'source_mask_unchanged':True,'retained_upper_geometry_changed':True,'inference':'Complete visible wood depth rebuilt; observed source pixels remain the constraint. No previous geometry approval is carried.','gallery_ready':False}
        (args.output/'evidence.json').write_text(json.dumps(evidence,indent=2)+'\n')
        print(json.dumps({k:v for k,v in evidence.items()if k!='protected'},indent=2))
    finally:release()


if __name__=='__main__':main()
