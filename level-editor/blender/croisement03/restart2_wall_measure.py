"""Record the southeast wall's native volume corners before subdivision."""
import sys,json
from pathlib import Path
import bpy
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire,release
OUT=ROOT/'level-editor/work/croisement03-refinement'
def main():
    acquire();bpy.ops.wm.open_mainfile(filepath=str(OUT/'croisement03-grouped.blend'));rows=[]
    for o in bpy.data.collections['Croisement03 Working'].all_objects:
        if o.type=='MESH' and o.get('asset_group')=='croisement03-southeast-stone-wall':
            rows.append(dict(node=o['source_node'],vertices=[list(o.matrix_world@v.co) for v in o.data.vertices],faces=[list(p.vertices) for p in o.data.polygons]))
    (OUT/'restart2/southeast-wall-source/native-volume.json').write_text(json.dumps(rows,indent=2)+'\n');print([(r['node'],len(r['vertices']),len(r['faces'])) for r in rows]);release()
if __name__=='__main__':main()
