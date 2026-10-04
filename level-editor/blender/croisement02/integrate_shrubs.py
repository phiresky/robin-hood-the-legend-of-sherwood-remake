"""Register only manually reviewed authored shrub candidates in the map catalog."""
import json
import sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT,reviewed_catalog
from catalog_schema import parse_catalog
from evidence_io import sha,write_json


def main():
    directory=OUT/'understory-candidates/clumps-v1'
    if sha(reviewed_catalog())!=sha(directory/'previous-catalog.json'):
        raise ValueError('Current ownership changed; merge the new shrub groups explicitly')
    catalog=json.loads((directory/'catalog.json').read_text())
    inv=json.loads((directory/'inventory/inventory.json').read_text())
    parse_catalog(catalog,{r['source_node'] for r in inv['objects']}-{'ground'})
    records=[]
    for index in (55,58,59):
        worker=OUT/f'understory-round-1/assets/croisement02-shrub-{index:02}'
        model_hash=sha(worker/'model.blend')
        review=json.loads((worker/'inspection/visual-review.json').read_text())
        audit=json.loads((worker/'inspection/saved-model-audit.json').read_text())
        coverage=json.loads((worker/'inspection/source-coverage/report.json').read_text())
        bounds=json.loads((worker/'inspection/actual-materials/opacity-bounds.json').read_text())
        if not (review.get('ready_for_geometry_review') and review['model_sha256']==model_hash
                and audit['status']=='PASS' and audit['model_sha256']==model_hash
                and coverage['model_sha256']==model_hash and coverage['intersection_over_union']>=.95
                and bounds['model_sha256']==model_hash and min(r['depth_width_ratio'] for r in bounds['crowns'])>=1.
                and review['sheet_sha256']==sha(worker/'inspection/actual-materials/sheet.png')):
            raise ValueError('Shrub review/geometry checks incomplete: '+worker.name)
        record=dict(model_sha256=model_hash,catalog_sha256=sha(directory/'catalog.json'),
                    status='reviewed geometry candidate; no user approval implied')
        write_json(worker/'inspection/shrub-candidate.json',record);records.append(dict(asset_id=worker.name,**record))
    write_json(OUT/'ownership-revision/catalog.json',catalog)
    write_json(OUT/'ownership-revision/grouping-review.json',json.loads((directory/'grouping-review.json').read_text()))
    write_json(directory/'integration.json',dict(status='three reviewed shrub candidates integrated; user approval pending',groups=len(catalog['groups']),native_parts=150,authored_parts=5,
        inventory=str(directory/'inventory/inventory.json'),source_masks=str(directory/'source-masks.json'),records=records))
    print('Integrated three shrub candidates:',len(catalog['groups']),'groups')

if __name__=='__main__':main()
