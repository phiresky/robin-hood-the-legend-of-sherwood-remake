"""Drop unused atlas dependencies for explicitly geometry-only opaque wood review."""
import json,sys
from pathlib import Path
HERE=Path(__file__).resolve().parent;sys.path.insert(0,str(HERE))
import qualify as q

def main():
    dest=q.OUT/'support04-wood-context-v2-colors';assert not dest.exists();dest.mkdir()
    report=json.loads((q.OUT/'support04-wood-context-v1/report.json').read_text())
    for row in report['records']:
        original=q.ROOT/row['view']['path'];assert q.pin(original)==row['view'];doc=json.loads(original.read_text())
        used={p['material']for n in doc['nodes']if 'mesh'in n for p in doc['meshes'][n['mesh']]['primitives']}
        assert all(doc['materials'][i].get('alphaMode','OPAQUE')=='OPAQUE'for i in used)
        doc['materials']=[dict(name=m.get('name','')+' diagnostic wood',doubleSided=m.get('doubleSided',False),pbrMetallicRoughness=dict(baseColorFactor=[.25,.12,.04,1],metallicFactor=0,roughnessFactor=1))for m in doc['materials']]
        for key in ['images','textures','samplers']:doc.pop(key,None)
        path=dest/original.name;path.write_text(json.dumps(doc,separators=(',',':'))+'\n');row['original_material_view']=row['view'];row['view']=q.pin(path)
    report['status']='GEOMETRY_ONLY_WOOD_JUNCTION_CONTEXT';report['limits']=['Exact wood topology, transforms, UV and source buffer positions retained.','All imported wood materials were OPAQUE; diagnostic constant brown preserves their culling flags and opacity.','No original wood appearance evaluation in this view.','Foliage/collision/source appearance evidence is separate.']
    (dest/'report.json').write_text(json.dumps(report,indent=2)+'\n')
if __name__=='__main__':main()
