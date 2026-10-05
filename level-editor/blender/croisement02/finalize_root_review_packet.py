"""Reuse exact reviewed full-bounds renders in standard root-completion packets."""
import argparse
import json
from pathlib import Path
import shutil
import sys
from PIL import Image,ImageDraw
sys.path[:0]=[str(Path(__file__).parent),str(Path(__file__).resolve().parents[3]/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json


def main(kind):
    logging=kind=='logging';asset='croisement02-logging-clearing-log' if logging else 'croisement02-southwest-stumps'
    trial=OUT/'restart2-vegetation'/('logging-convex-v7' if logging else 'southwest-convex-v3')
    worker=OUT/'restart2-vegetation'/(kind+'-root-package-v1')/'assets'/asset
    digest=sha(worker/'model.blend');assert digest==sha(trial/'model.blend')
    evidence=trial/'full-review/evidence.json';data=json.loads(evidence.read_text());assert data['model_sha256']==digest
    for name,expected in data['images'].items():assert sha(trial/'full-review'/name)==expected,name
    actual=worker/'inspection/actual-materials';shutil.copytree(trial/'full-review/actual',actual)
    write_json(actual/'evidence.json',dict(model_sha256=digest,sheet_sha256=sha(actual/'sheet.png'),source_evidence=str(evidence),source_evidence_sha256=sha(evidence),framing='Independent full-bounds eight views, source first; exact reviewed images reused without rerendering',render_config=dict(engine='CYCLES',samples=8,transparent_max_bounces=64)))
    compare=worker/'inspection/source-comparison';compare.mkdir()
    pictures=[Image.open(trial/'full-review'/name).convert('RGBA') for name in ['native-source.png','source.png','source-overlay.png']]
    w,h=pictures[0].size;canvas=Image.new('RGB',(w*3,h+25),'#333333');draw=ImageDraw.Draw(canvas)
    for i,(picture,label) in enumerate(zip(pictures,['Native source','Saved geometry','Native overlay'])):
        canvas.paste(picture,(i*w,25),picture);draw.text((i*w+5,5),label,fill='white')
    canvas.save(compare/'comparison.png')
    write_json(compare/'report.json',dict(model_sha256=digest,comparison_sha256=sha(compare/'comparison.png'),source_evidence=str(evidence),source_evidence_sha256=sha(evidence),scope='Native root mass crop; whole-asset framing supplied by actual eight views'))
    print(worker)


if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('kind',choices=['logging','southwest']);main(parser.parse_args().kind)
