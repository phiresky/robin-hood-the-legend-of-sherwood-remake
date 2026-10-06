"""Compare authored UV selection and material wiring on private derivatives."""
import sys,json
from pathlib import Path
import bpy
HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE))
from restart6_source_gap_audit import ROOT,SPECS
from render_slots import acquire,release
from evidence_io import write_json
def main():
 rows=[]
 for path in [SPECS[39][0],ROOT/'tree39-contour-v4/model.blend']:
  bpy.ops.wm.open_mainfile(filepath=str(path));objects=[]
  for o in bpy.context.scene.objects:
   if o.type!='MESH'or o.get('asset_group')!='croisement02-tree-39':continue
   objects.append(dict(name=o.name,modifiers=[dict(name=m.name,type=m.type,show_viewport=m.show_viewport,show_render=m.show_render)for m in o.modifiers],active_uv=o.data.uv_layers.active.name,uv=[dict(name=u.name,render=u.active_render)for u in o.data.uv_layers],used_materials=[dict(slot=i,name=m.name,nodes=[dict(name=n.name,type=n.type,uv=n.uv_map if n.type=='UVMAP'else None,image=n.image.name if n.type=='TEX_IMAGE'and n.image else None)for n in m.node_tree.nodes],links=[(l.from_node.name,l.from_socket.name,l.to_node.name,l.to_socket.name)for l in m.node_tree.links])for i,m in enumerate(o.data.materials)if i in {p.material_index for p in o.data.polygons}]))
  rows.append(dict(model=str(path),objects=objects))
 write_json(ROOT/'material-state-audit-v2.json',dict(models=rows))
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
