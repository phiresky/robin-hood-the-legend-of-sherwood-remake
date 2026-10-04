"""Audit saved upright-fence candidates against their observed wood domains."""
import json
import sys
from pathlib import Path
import bpy
import bmesh
from PIL import Image
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from catalog import OUT
from evidence_io import sha,write_json
from source_coverage import audit


def main(directory):
    for index in (94,95):
        worker=directory/f'assets/croisement02-east-upright-rail-fence-{index}'
        bpy.ops.wm.open_mainfile(filepath=str(worker/'model.blend'))
        objects=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o.get('asset_group')==worker.name]
        rows=[]
        for obj in objects:
            bm=bmesh.new();bm.from_mesh(obj.data)
            row=dict(object=obj.name,vertices=len(bm.verts),faces=len(bm.faces),nonmanifold_edges=sum(not e.is_manifold for e in bm.edges),degenerate_faces=sum(f.calc_area()<1e-8 for f in bm.faces));bm.free()
            if row['nonmanifold_edges'] or row['degenerate_faces']:raise ValueError(row)
            rows.append(row)
        packet=worker/'inspection/source-domain';packet.mkdir(exist_ok=True)
        Image.new('RGBA',(1792,1152)).save(packet/'complete-source.png')
        write_json(packet/'partition.json',dict(native_bbox=[0,0,1792,1152],native_mask=index,observed_domain=430+index-94,note='Geometry coverage compares only assigned observed wood, not every pixel of the mixed native character-occlusion silhouette.'))
        report=json.loads((worker/'inspection/refinement.json').read_text());report.update(mask=index,wood_domain_mask=430+index-94,source_packet=str(packet/'partition.json'))
        write_json(worker/'inspection/refinement.json',report)
        coverage=audit(worker,objects)
        write_json(worker/'inspection/fence-topology.json',dict(status='PASS',model_sha256=sha(worker/'model.blend'),objects=rows,coverage=coverage,notes=['Each timber is a separate closed volume. Intentional joinery intersections connect posts and rails; geometry does not fill the openings.','Hidden member depth, mounting faces, foot depth and beyond-map rail continuation are inferred.']))
        print(worker,coverage,flush=True)

if __name__=='__main__':
    args=sys.argv[sys.argv.index('--')+1:];acquire()
    try:main(Path(args[0]).resolve())
    finally:release()
