"""Expose a separately inspected ground receiver without granting approval."""
import argparse
import json
import sys
from pathlib import Path
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement/blender')]
from evidence_io import sha,write_json
from build_review_gallery import build


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('workspace',type=Path);args=parser.parse_args()
    worker=args.workspace.resolve();model=sha(worker/'model.blend')
    review=json.loads((worker/'inspection/visual-review.json').read_text())
    audit=json.loads((worker/'inspection/saved-model-audit.json').read_text())
    if review['model_sha256']!=model or not review['ready_for_geometry_review']:raise ValueError('Current visual review required')
    if audit['model_sha256']!=model or audit['status']!='PASS':raise ValueError('Saved-model audit required')
    for name,expected in review['images'].items():
        if sha(worker/name)!=expected:raise ValueError('Reviewed image changed')
    partition=json.loads((worker/'inspection/ground-bank-partition.json').read_text())
    if partition['model_sha256']!=model or partition['status']!='PASS':raise ValueError('Current bank partition audit required')
    notes=['Ground and bank source domains overlap0pixels; observed ground overlapping bank first-hit geometry also0pixels. Native ramp3 foot separates raised bank from the dirt apron.', 'Unchanged native flat ground receiver; raised bank0–4 and rock/root-bank units are separate geometry.',
           '771341 observed ground pixels are preserved byte-for-byte;723163 hidden ground pixels remain neutral. No texture generation has run.',
           'Ownership colors: cyan observed, magenta hidden ground, dark gray separately owned relief. Gray material is unfilled ground, not a hole in geometry.',
           'Frozen78-group catalog records ownership only; context geometry uses native proxies plus strict bank. Final assembled-scene first-hit is still required.',
           'Native grass111–123 (6237pixels), log102/103, rootbank370, and fences430/431 remain excluded foreground; incomplete models cannot transfer their artwork to ground.',
           'All3353 mission frames and9 native patch records are hash-bound. The barrier terminal152×152 artwork has a separate ground-state assignment.',
           'This decision covers receiver geometry and proposed source domain only; it does not approve hidden texture, state integration, or publication.']
    item=dict(id='croisement02-ground-receiver',name='Flat Ground Receiver and Source Ownership',status='ready-for-user',technical_eligible=True,
              model=str(worker/'model.blend'),validation=str(worker/'validation.json'),review=str(worker/'inspection/visual-review.json'),stored_material_audit=str(worker/'inspection/saved-model-audit.json'),
              solid=str(worker/'modified/solid.png'),textured=str(worker/'modified/textured.png'),context=str(worker/'modified/context.png'),
              stored_material_textured=str(worker/'inspection/actual-oblique-0.png'),source_comparison=str(worker/'inspection/actual-source.png'),source_comparison_label='Actual saved ground material from original map camera; neutral areas await fill',
              source_comparison_secondary=str(worker/'inspection/domains-oblique-1.png'),source_comparison_secondary_label='Ownership on actual receiver: cyan observed, magenta hidden, dark separate relief',
              projection_errors=str(worker/'reference/domain-review.png'),projection_errors_label='Original map with proposed ground and pending foreground domains',
              artwork_references=[dict(id='bank-partition',label='Bank versus ground: orange bank, cyan ground, white native ramp3 foot',path=str(worker/'inspection/northeast-ground-bank-ownership.png'),sha256=sha(worker/'inspection/northeast-ground-bank-ownership.png')),dict(id='barrier-terminal',label='Barrier state: covered source versus terminal ground artwork',path=str(worker/'reference/barrier-state-source-comparison.png'),sha256=sha(worker/'reference/barrier-state-source-comparison.png'))],notes=notes)
    index=worker/'review-candidates.json';write_json(index,dict(map='Croisement02 ground receiver',items=[item],without_packets=[]));build(index,worker/'gallery',map_name='Croisement02 ground receiver')
    print(worker/'gallery/index.html')


if __name__=='__main__':main()
