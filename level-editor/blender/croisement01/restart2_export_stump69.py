"""Export approved continuous stump as body/cap parts with identical visible surfaces."""
import copy,json,sys,hashlib
from pathlib import Path
import bpy,bmesh
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire
from texture_staging import validate_texture_handoff,verify_baked_geometry
from export_editor import export_asset_library
from review_evidence import sha
R=ROOT/'level-editor/work/croisement01-refinement/restart2';asset='croisement01-central-ivy-stump';case=R/'approved-stump69-wood-fill-v1'/asset

def signatures(objects):
    result=[]
    for obj in objects:
        mesh=obj.data
        for face in mesh.polygons:
            corners=[]
            for li in face.loop_indices:
                v=mesh.vertices[mesh.loops[li].vertex_index].co
                corners.append((tuple(v),tuple((layer.name,tuple(layer.data[li].uv)) for layer in mesh.uv_layers),tuple((layer.name,tuple(layer.data[li].color)) for layer in mesh.color_attributes if layer.domain=='CORNER')))
            start=min(range(len(corners)),key=lambda i:corners[i]);corners=corners[start:]+corners[:start]
            result.append(json.dumps([corners,mesh.materials[face.material_index].name,face.use_smooth],sort_keys=True))
    return sorted(result)

def keep_faces(mesh,indices):
    bm=bmesh.new();bm.from_mesh(mesh);bm.faces.ensure_lookup_table()
    bmesh.ops.delete(bm,geom=[f for f in bm.faces if f.index not in indices],context='FACES')
    bmesh.ops.delete(bm,geom=[v for v in bm.verts if not v.link_faces],context='VERTS')
    bm.to_mesh(mesh);bm.free();mesh.update()

def main():
    # No export can pass this guard before the exact texture decision exists.
    h=validate_texture_handoff(case/'review-manifest.json',asset,case/'texture-handoff-v1/decisions.json',case/'decisions.json')
    acquire();guard=verify_baked_geometry(h)
    cfg=json.loads((case/'approved-workspace/workspace.json').read_text());working=bpy.data.collections[cfg['collection_name']]
    body=next(o for o in working.all_objects if o.type=='MESH' and o.get('asset_group')==asset)
    probe=json.loads((R/'stump69-cap-partition-probe-v1.json').read_text());assert sha(case/'baked-v1-luminance/worker.blend')==probe['model_sha256']
    original=signatures([body]);matrix=body.matrix_world.copy();top=probe['planned_cap_face'];count=len(body.data.polygons);assert count==probe['polygons']
    cap=body.copy();cap.data=body.data.copy();cap.name='Central Ivy Stump native cap reference058';working.objects.link(cap)
    for previous in list(working.all_objects):
        if previous!=cap and previous!=body and previous.get('source_node')=='building-058':bpy.data.objects.remove(previous,do_unlink=True)
    cap['source_node']='building-058';cap['source_obstacle']=58;cap['part_name']='Cut cap';body['part_name']='Continuous stump shaft'
    keep_faces(cap.data,{top});keep_faces(body.data,set(range(count))-{top})
    bpy.context.view_layer.update();assert body.matrix_world==matrix and cap.matrix_world==matrix
    assert signatures([body,cap])==original
    catalog_path=R/'stump69-wood-v1/assets'/asset/'reference/grouping.json';catalog=json.loads(catalog_path.read_text());groups=[]
    for group in catalog['groups']:
        parts=[p for p in group['parts'] if p.get('obstacle')!=58]
        if group['id']==asset:parts.append(dict(obstacle=58,name='Cut cap'))
        if parts:groups.append(dict(group,parts=parts))
    catalog['groups']=groups;catalog['canonical_owners']={f"building-{p['obstacle']:03}" if 'obstacle' in p else p['node']:g['id'] for g in groups for p in g['parts']}
    out=R/'stump69-integration-v2';out.mkdir(exist_ok=False)
    (out/'export-catalog.json').write_text(json.dumps(catalog,indent=2)+'\n')
    report=export_asset_library('Croisement01',out/'assets',ROOT/'level-editor/work/croisement01-refinement/baseline/Croisement01.rhp.json',asset_ids=[asset],catalog=catalog)
    proof=dict(scope='Private exact approved surface export partition; no live publication',geometry=guard,export=report,approved_user_decision_sha256=sha(case/'user-texture-decision.json'),source_cap_probe_sha256=sha(R/'stump69-cap-partition-probe-v1.json'),source_faces=count,body_faces=len(body.data.polygons),cap_faces=len(cap.data.polygons),union_of_local_surface_positions_uv_corner_ownership_materials_and_smoothing_exact=True,object_world_transforms_unchanged=True,surface_fingerprint_sha256=hashlib.sha256(json.dumps(original).encode()).hexdigest(),native_gameplay_parts_preserved=['building-057','building-058'])
    (out/'export-proof.json').write_text(json.dumps(proof,indent=2)+'\n');print(out)
if __name__=='__main__':main()
