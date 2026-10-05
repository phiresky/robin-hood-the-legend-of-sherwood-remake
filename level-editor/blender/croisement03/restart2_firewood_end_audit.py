"""Measure two manually located pale end patches independently of the full mask."""
import argparse,json,hashlib
from pathlib import Path
import numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'level-editor/work/croisement03-refinement'
def main():
    parser=argparse.ArgumentParser();parser.add_argument('workspace',type=Path);args=parser.parse_args();w=args.workspace.resolve()
    report=json.loads((w/'inspection/source-comparison/report.json').read_text());crop=report['crop']
    render=np.asarray(Image.open(w/'inspection/source-comparison/render.png').convert('RGBA'))
    source=np.asarray(Image.open(OUT/'baseline/covered.png').convert('RGBA'));rows=[]
    for name,box in [('near lower pale end',(431,782,440,787)),('far lower pale end',(442,781,448,785))]:
        x,y,x1,y1=box;s=source[y:y1,x:x1,:3].astype(int)
        observed=(s[:,:,0]>135)&(s[:,:,1]>110)&(s[:,:,2]>65)&(s[:,:,0]>s[:,:,1])
        alpha=render[y-crop[1]:y1-crop[1],x-crop[0]:x1-crop[0],3]>127
        coords=lambda region:[[int(x+px),int(y+py)] for py,px in zip(*np.nonzero(region))]
        rows.append(dict(name=name,manual_region=list(box),observed_pale_pixels=coords(observed),missing_pixels=coords(observed&~alpha)))
    result=dict(model_sha256=hashlib.sha256((w/'model.blend').read_bytes()).hexdigest(),source_sha256=hashlib.sha256((OUT/'baseline/covered.png').read_bytes()).hexdigest(),patches=rows,scope='Pale texels in two manually observed billet ends. This is not whole-mask ownership, texture correctness, or proof of hidden log count.')
    (w/'inspection/pale-end-coverage.json').write_text(json.dumps(result,indent=2)+'\n');print([(r['name'],len(r['observed_pale_pixels']),len(r['missing_pixels'])) for r in rows])
if __name__=='__main__':main()
