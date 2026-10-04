"""Extract native log silhouettes and editable thin-branch centerlines."""
import json
from pathlib import Path
import numpy as np
from PIL import Image
from catalog import OUT


def main():
    from hashlib import sha256
    level=json.loads((OUT/'baseline/Croisement02.rhp.json').read_text());profiles=[]
    record=level['masks'][102];x0,y0=record['box_top_left'];mask=np.asarray(Image.open(OUT/'baseline/masks/000102.png').convert('L'))>0
    for x in range(0,mask.shape[1],3):
        yy=np.flatnonzero(mask[:,x])
        if not len(yy):continue
        profiles.append([x+x0+.5,float((yy.min()+yy.max())/2+y0+.5),float((yy.max()-yy.min()+1)/2)])
    for i,p in enumerate(profiles):
        a=profiles[max(0,i-1)];b=profiles[min(len(profiles)-1,i+1)];slope=(b[1]-a[1])/max(b[0]-a[0],1)
        p[2]=p[2]/np.sqrt(1+slope*slope)+.45
    # Independently traced wood centerlines: each secondary branch attaches once.
    # Do not skeletonize the occlusion silhouette: closing raster gaps invents loops.
    wood_paths=[
      [(213,1055),(212,1047),(209,1036),(205,1024),(202,1019)],
      [(209,1036),(203,1030),(198,1022)],
      [(212,1047),(205,1043),(198,1036)],
      [(209,1036),(209,1026),(208,1020)],
      [(214,1054),(219,1045),(225,1036),(229,1028),(230,1023)],
      [(219,1045),(214,1038),(214,1033)],
      [(225,1036),(220,1033),(219,1028)],
      [(220,1044),(224,1040),(225,1048)],
      [(232,1058),(246,1048),(251,1039),(258,1032),(266,1027)],
      [(251,1039),(245,1033),(241,1025)],
      [(258,1032),(260,1026),(265,1022)],
      [(246,1048),(257,1045),(266,1044)],
      [(253,1054),(270,1043),(282,1038),(288,1029),(289,1022)],
      [(270,1043),(267,1037),(268,1032)],
      [(282,1038),(285,1043),(283,1047)],
    ]
    paths=[[[float(x),float(y),.60 if j==len(path)-1 else .85] for j,(x,y) in enumerate(path)] for path in wood_paths]
    result=dict(source_sha256=sha256((OUT/'animation-references/composite-frame-0.png').read_bytes()).hexdigest(),mask_sha256={str(i):sha256((OUT/f'baseline/masks/{i:06}.png').read_bytes()).hexdigest() for i in (102,103)},foreground_log_profile=profiles,branches=paths,method='Native102 column silhouette with rounded hidden depth; native103 individually traced wood centerlines without inferred closed cycles. These are geometry construction targets, not independent depth proof.')
    output=OUT/'southwest-log-revision/native-traces.json';output.parent.mkdir(exist_ok=True);output.write_text(json.dumps(result,indent=2)+'\n');print(len(profiles),'profile rings;',len(paths),'branch strokes;',output)

if __name__=='__main__':main()
