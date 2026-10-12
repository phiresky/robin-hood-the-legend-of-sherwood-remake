"""Inspect approved phase material contracts before lossless GLB conversion."""
import sys,json,hashlib
from pathlib import Path
import bpy
ROOT=Path(__file__).resolve().parents[4];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire,release
BASE=ROOT/'level-editor/work/active/croisement02/mission-hidden-archer-physical-oct12-v1/approved03-04-textures-v1/private-phase-integration-v1'
def main():
 delta=json.loads((BASE/'phase-integration-delta.json').read_text());rows=[]
 for profile in delta['profiles']:
  for model in profile['models']:
   p=Path(model['path']);assert hashlib.sha256(p.read_bytes()).hexdigest()==model['sha256'];bpy.ops.wm.open_mainfile(filepath=str(p));objects=[]
   for obj in bpy.context.scene.objects:
    if obj.type!='MESH' or obj.get('source_phase')!=model['phase']:continue
    mats=[]
    for mat in obj.data.materials:
     if mat is None:raise ValueError('Missing material')
     mats.append({'name':mat.name,'custom':dict(mat.items()),'nodes':[{'type':n.type,'name':n.name,'image':n.image.name if n.type=='TEX_IMAGE' and n.image else None,'packed':bool(n.image.packed_file) if n.type=='TEX_IMAGE' and n.image else None,'uv':n.uv_map if n.type=='UVMAP' else None}for n in mat.node_tree.nodes],'links':[(l.from_node.name,l.from_socket.name,l.to_node.name,l.to_socket.name)for l in mat.node_tree.links]})
    objects.append({'name':obj.name,'vertices':len(obj.data.vertices),'polygons':len(obj.data.polygons),'materials':mats,'uv_layers':[u.name for u in obj.data.uv_layers],'color_attributes':[a.name for a in obj.data.color_attributes]})
   assert objects;rows.append({'profile':profile['card_id'],'phase':model['phase'],'model':model,'objects':objects})
 (BASE/'material-contract-inspection.json').write_text(json.dumps(rows,indent=2,default=str)+'\n')
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
