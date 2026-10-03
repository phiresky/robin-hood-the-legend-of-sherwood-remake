"""Correct observed leaf face orientation and regenerate the source-only packet."""
import argparse
import json
import sys
from pathlib import Path
import bpy
import bmesh
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent));sys.path.insert(0,str(ROOT/'level-editor/refinement'));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT
from tree_geometry import RAY,one_sided
from bark_materials import fill
from refinement_workspace import validate,modified
from render_slots import acquire,release
from evidence_io import sha


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--masks',nargs='*',type=int);parser.add_argument('--from-version');parser.add_argument('--render',action='store_true')
    args=parser.parse_args(sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else [])
    completed=0
    for workspace in sorted((OUT/'forest-v4-round-1/assets').iterdir()):
        path=workspace/'inspection/refinement.json'
        if not path.exists():continue
        record=json.loads(path.read_text())
        if args.masks is not None and record['mask'] not in args.masks:continue
        version=record['crown'].get('geometry_version')
        if args.from_version and version!=args.from_version:continue
        if version=='native-leaf-clusters-v5' and record.get('leaf_fallback_ownership')=='corrected':continue
        if version not in ('native-leaf-clusters-v3','native-leaf-clusters-v4','native-leaf-clusters-v5'):continue
        acquire();bpy.ops.wm.open_mainfile(filepath=str(workspace/'model.blend'));bpy.context.preferences.filepaths.save_version=0;validate(workspace)
        objects=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o.get('asset_group')==workspace.name]
        crown=next(o for o in objects if o.get('projection_component')=='crown');bm=bmesh.new();bm.from_mesh(crown.data);bm.normal_update()
        normal_matrix=crown.matrix_world.to_3x3().inverted().transposed()
        ownership=bm.loops.layers.float_color.get('Source ownership')
        fallback=bm.faces.layers.int.get('reprojection_fallback_material')
        back_slots=[i for i,m in enumerate(crown.data.materials) if m.get('foliage_card_sides')=='paired-one-sided' and not m.get('foliage_observed')]
        repaired=0
        if version=='native-leaf-clusters-v5':
            for face in bm.faces:
                if crown.data.materials[face.material_index].get('foliage_observed') and all(loop[ownership][0]<.5 for loop in face.loops):
                    if len(back_slots)!=1:raise ValueError('Ambiguous inferred rear material')
                    face.material_index=back_slots[0]
                    if fallback is not None:face[fallback]=back_slots[0]
                    repaired+=1
            crown['foliage_backfaces_version']='v5'
        observed=[f for f in bm.faces if crown.data.materials[f.material_index].get('foliage_observed')]
        wrong=[f for f in observed if (normal_matrix@f.normal).normalized().dot(RAY)<.05]
        bmesh.ops.reverse_faces(bm,faces=wrong);bm.normal_update()
        if any((normal_matrix@f.normal).normalized().dot(RAY)<.05 for f in observed):raise ValueError('Observed leaf faces must face source camera')
        if crown.get('foliage_backfaces_version')!='v5':
            # Give source-facing and inferred rear faces separate ownership and
            # culling; a two-sided source face must not claim observed rear RGB.
            front=crown.data.materials[0];one_sided(front)
            back=crown.data.materials[1].copy();back.name=crown.name+' inferred leaf backs';one_sided(back)
            crown.data.materials.append(back);slot=len(crown.data.materials)-1
            uv=bm.loops.layers.uv.get('Foliage UV');ownership=bm.loops.layers.float_color.get('Source ownership')
            offset=crown.matrix_world.to_3x3().inverted()@(-RAY*.02)
            for face in list(observed):
                old=list(reversed(list(face.loops)))
                vertices=[bm.verts.new(loop.vert.co+offset) for loop in old]
                rear=bm.faces.new(vertices);rear.material_index=slot
                if fallback is not None:rear[fallback]=slot
                for loop,source in zip(rear.loops,old):
                    loop[uv].uv=source[uv].uv
                    if ownership:loop[ownership]=(0.,1.,1.,1.)
            bm.normal_update();bm.to_mesh(crown.data);bm.free();crown.data.update()
            crown['foliage_backfaces_version']='v5'
        else:
            bm.to_mesh(crown.data);bm.free();crown.data.update()
        validate(workspace);modified(workspace);record['bark']=fill(workspace,objects,record['mask'])
        validate(workspace);bpy.ops.wm.save_as_mainfile(filepath=str(workspace/'model.blend'))
        record['crown']['geometry_version']='native-leaf-clusters-v5';record['crown']['vertices']=len(crown.data.vertices);record['crown']['faces']=len(crown.data.polygons);record['crown']['observed_face_orientation']='PASS'
        record['leaf_fallback_ownership']='corrected'
        record['model_sha256']=sha(workspace/'model.blend');path.write_text(json.dumps(record,indent=2)+'\n')
        from audit_candidates import audit
        audit(workspace)
        print('FINALIZED',workspace.name,'reversed',len(wrong),flush=True)
        if args.render:
            from render_tree import render_workspace
            render_workspace(workspace,256,release_slot=False)
        completed+=1
        if completed%2==0:release()
    release()

if __name__=='__main__':main()
