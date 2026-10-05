"""Classify rendered sign body occlusion into solid contact and extra foliage."""
import sys,json
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json

def main():
    base=OUT/'restart2-fence/shrub57-sign-bend-v6';full=base/'joint-proof';bank=base/'bank-only-proof';dest=base/'contact-classification';dest.mkdir(exist_ok=False)
    full_report=json.loads((full/'report.json').read_text());bank_report=json.loads((bank/'report.json').read_text());assert full_report['model_sha256']==bank_report['model_sha256'];rows=[];sheet=Image.new('RGB',(1152,2496),(60,60,60))
    def read(path):return np.array(Image.open(path).convert('RGB'))[1::3,1::3,0]>127
    for phase in range(32):
        alone=read(full/f'pose-{phase:02}-alone.png');assert np.array_equal(alone,read(bank/f'pose-{phase:02}-alone.png'))
        full_hidden=alone&~read(full/f'pose-{phase:02}-joint.png');bank_hidden=alone&~read(bank/f'pose-{phase:02}-joint.png');extra=full_hidden&~bank_hidden
        record=dict(phase=phase,solid_contact_pixels=int(bank_hidden.sum()),extra_foliage_pixels=int(extra.sum()),extra_source_pixels=[[int(x+27),int(y+222)] for y,x in zip(*np.nonzero(extra))],solid_contact_source_pixels=[[int(x+27),int(y+222)] for y,x in zip(*np.nonzero(bank_hidden))]);rows.append(record)
        pic=np.array(Image.open(full/f'pose-{phase:02}-actual.png').convert('RGB'));up=lambda m:np.repeat(np.repeat(m,3,axis=0),3,axis=1);pic[up(bank_hidden)]=[40,150,255];pic[up(extra)]=[255,40,60];sheet.paste(Image.fromarray(pic),(phase%4*288,phase//4*312));ImageDraw.Draw(sheet).text((phase%4*288+3,phase//4*312+290),f'Pose{phase}: contact {bank_hidden.sum()}, leaves {extra.sum()}',fill='white')
    sheet.save(dest/'all32-contact.png');write_json(dest/'report.json',dict(model_sha256=full_report['model_sha256'],full_context_sha256=sha(full/'report.json'),bank_context_sha256=sha(bank/'report.json'),rows=rows,extra_foliage_total=sum(r['extra_foliage_pixels'] for r in rows),semantics='Exact saved object-index masks. Blue is bank/rock occlusion; red is additional complete-context occlusion. Tiny solid contact can be legitimate planted-post geometry and is reported separately.'));print(dest)
if __name__=='__main__':main()
