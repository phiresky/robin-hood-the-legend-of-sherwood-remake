"""Read the private glTF material wiring without changing source artifacts."""
import sys,json
from pathlib import Path
import bpy
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire,release
def main():
 out=ROOT/'level-editor/work/croisement03-refinement/restart2/tree13-exact-export-v1';acquire()
 try:
  bpy.ops.wm.read_factory_settings(use_empty=True);bpy.ops.import_scene.gltf(filepath=str(out/'model.glb'));rows=[]
  for m in bpy.data.materials:
   rows.append(dict(name=m.name,nodes=[dict(name=n.name,type=n.type,uv=getattr(n,'uv_map',None),image=n.image.name if n.type=='TEX_IMAGE' and n.image else None) for n in m.node_tree.nodes],links=[dict(a=l.from_node.name,output=l.from_socket.name,b=l.to_node.name,input=l.to_socket.name) for l in m.node_tree.links]))
  (out/'import-probe.json').write_text(json.dumps(rows,indent=2)+'\n')
 finally:release()
if __name__=='__main__':main()
