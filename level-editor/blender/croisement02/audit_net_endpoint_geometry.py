"""Reopen a net endpoint and check closed volumes and separate wood/bag depth."""
import sys,json
from pathlib import Path
import bpy,bmesh,numpy as np
sys.path.insert(0,str(Path(__file__).parent))
from catalog import OUT
from log_trap_state_candidate import sha
from settle_initial_rock_pile import collision_interval,points


def main():
 base=Path(sys.argv[sys.argv.index('--candidate')+1]).resolve();report=json.loads((base/'manifest.json').read_text());assert sha(base/'worker.blend')==report['model_sha256'];bpy.ops.wm.open_mainfile(filepath=str(base/'worker.blend'));bpy.context.view_layer.update();rows=[]
 for record in report['objects']:
  obj=bpy.data.objects[record['object']];p=points(obj);bm=bmesh.new();bm.from_mesh(obj.data);closed=all(e.is_manifold for e in bm.edges);volume=bm.calc_volume(signed=True);bm.free();normals=np.array([obj.matrix_world.to_3x3()@f.normal for f in obj.data.polygons]);anchors=np.array([obj.matrix_world@obj.data.vertices[f.vertices[0]].co for f in obj.data.polygons]);outside=(p@normals.T-(anchors*normals).sum(axis=1)).max();rows.append(dict(object=obj.name,closed=closed,volume=volume,convex_plane_max_outside=float(outside),minimum_z=float(p[:,2].min())))
 bag=bpy.data.objects['Occupied bag'];wood=bpy.data.objects['Wooden piece'];interval=collision_interval(bag,wood);intersects=interval is not None and interval['first']<-.001 and interval['last']>.001
 result=dict(status='PASS scoped closed-volume and bag/wood separation checks'if not intersects and all(r['closed']and r['volume']>0 and r['convex_plane_max_outside']<.001 for r in rows)else 'HOLD',model_sha256=report['model_sha256'],objects=rows,bag_wood_ray_intersection_interval=interval,bag_wood_intersect=intersects,limitations=['Cord-to-bag and cord-to-wood entry contacts are intentional connectors, not independent boulder bodies.','Upper cord attachment to actual map trees is not established.','This check does not establish phase animation, hidden texture or physical dynamics.'])
 (base/'geometry-audit.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result,indent=2))
if __name__=='__main__':main()
