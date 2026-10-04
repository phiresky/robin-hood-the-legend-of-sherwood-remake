"""Reopen rock endpoints and measure exact pair intersections without convex assumptions."""
import sys,json,itertools
from pathlib import Path
import bpy,bmesh
sys.path.insert(0,str(Path(__file__).parent))
from catalog import OUT
from log_trap_state_candidate import sha


def main():
 base=Path(sys.argv[sys.argv.index('--candidate')+1]).resolve();manifest=json.loads((base/'manifest.json').read_text());model=base/'worker.blend';assert sha(model)==manifest['model_sha256']
 bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.context.scene;rows=[];solids=[]
 for state in ['covered','applied']:
  objects=[o for o in scene.objects if o.get('state_endpoint')==state]
  for obj in objects:
   bm=bmesh.new();bm.from_mesh(obj.data);solids.append(dict(object=obj.name,state=state,closed=all(e.is_manifold for e in bm.edges),volume=bm.calc_volume(signed=True)));bm.free()
  for first,second in itertools.combinations(objects,2):
   copy=first.copy();copy.data=first.data.copy();scene.collection.objects.link(copy);bpy.context.view_layer.objects.active=copy
   mod=copy.modifiers.new('Independent volume intersection','BOOLEAN');mod.operation='INTERSECT';mod.solver='EXACT';mod.object=second;bpy.ops.object.modifier_apply(modifier=mod.name)
   bm=bmesh.new();bm.from_mesh(copy.data);volume=abs(bm.calc_volume(signed=True));bm.free();rows.append(dict(state=state,first=first.name,second=second.name,intersection_volume=volume,intersection_faces=len(copy.data.polygons)));data=copy.data;bpy.data.objects.remove(copy,do_unlink=True);bpy.data.meshes.remove(data)
 report=dict(status='PASS'if all(r['intersection_volume']<1e-4 for r in rows)and all(r['closed']and r['volume']>0 for r in solids)else 'HOLD',model_sha256=sha(model),solids=solids,pairs=rows,method='Reopened exact boolean intersection volumes; manifold and signed volume checks.',limitations=['Zero intersection volume does not establish stable physical balance or source body identity.'])
 (base/'pair-volume-audit.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report,indent=2))
if __name__=='__main__':main()
