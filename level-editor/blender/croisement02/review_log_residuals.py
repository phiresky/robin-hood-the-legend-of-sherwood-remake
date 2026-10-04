"""Report source-reviewed residual regions without equating mask pixels to wood."""
import argparse
import json
from pathlib import Path
import numpy as np
from PIL import Image
from scipy.ndimage import distance_transform_edt


def main(workspace):
    directory=workspace/'inspection/source-coverage'
    report=json.loads((directory/'report.json').read_text())
    expected=np.asarray(Image.open(directory/'expected.png'))>127
    actual=np.asarray(Image.open(directory/'render.png'))[:,:,3]>127
    missing=expected&~actual
    yy,xx=np.indices(missing.shape);xx+=report['source_crop'][0];yy+=report['source_crop'][1]
    categories=[
        ('foreground_plant_boundary_ambiguous',missing&(xx>=140)&(xx<=195)&(yy>=1098),'Native mask crosses yellow/green foreground foliage and adjacent timber. Do not infer wood for these pixels without separating plants.'),
        ('upper_twig_region',missing&(xx>=190)&(yy<1050),'Contains real fine wood, source raster edges, and ambiguous crossings. Residual remains explicit and is not classified automatically as plant or antialiasing.'),
    ]
    assigned=np.zeros_like(missing)
    colors=[(255,220,0),(220,50,255),(255,50,50)]
    out=np.asarray(Image.open(directory/'source.png').convert('RGB')).copy();records=[]
    for index,(name,mask,note) in enumerate(categories):
        mask&=~assigned;assigned|=mask;out[mask]=colors[index]
        records.append(dict(category=name,pixels=int(mask.sum()),source_review=note))
    remaining=missing&~assigned;out[remaining]=colors[-1]
    distance=distance_transform_edt(~actual)
    records.append(dict(category='timber_silhouette_residual',pixels=int(remaining.sum()),within_two_render_pixels=int(np.count_nonzero(remaining&(distance<=2))),beyond_two_render_pixels=int(np.count_nonzero(remaining&(distance>2))),source_review='Wood outline strips and end pixels. These are real source silhouette differences, not declared nonwood or automatically dismissed as antialiasing.'))
    result=dict(model_sha256=report['model_sha256'],missing_pixels=int(missing.sum()),categories=records,status='Source-reviewed accounting; residual wood and ambiguous pixels remain, not 100 percent source completion.',legend=dict(yellow=records[0]['category'],magenta=records[1]['category'],red=records[2]['category']))
    (directory/'residual-review.json').write_text(json.dumps(result,indent=2)+'\n')
    Image.fromarray(out).resize((out.shape[1]*4,out.shape[0]*4),Image.Resampling.NEAREST).save(directory/'residual-review.png')
    print(json.dumps(result,indent=2))

if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('workspace',type=Path);main(p.parse_args().workspace)
