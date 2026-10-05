"""Separate broad native leaf assignments into narrow rooted blade hypotheses."""
import copy
import math
import numpy as np
from restart2_ground_blades_curve import build as build_curves

def build(obj,source,guide,ground):
    refined=copy.deepcopy(guide);root=np.asarray(guide['root_source_local'],float);groups={};records=[]
    for pixel in guide['observed_pixels']:
        leaf=pixel['leaf'];axis=np.asarray(guide['paths'][leaf][0],float)+.5-root;axis/=max(np.linalg.norm(axis),1e-8);side=np.asarray([-axis[1],axis[0]])
        band=math.floor(float(np.dot(np.asarray([pixel['x']+.5,pixel['y']+.5])-root,side))/2)
        groups.setdefault((leaf,band),[]).append(pixel)
    refined['paths']=[];refined['observed_pixels']=[]
    for index,((leaf,band),pixels) in enumerate(sorted(groups.items())):
        raw=np.asarray(guide['paths'][leaf],float);axis=raw[0]+.5-root;length=max(np.linalg.norm(axis),1e-8);axis/=length;side=np.asarray([-axis[1],axis[0]])
        progress=np.clip(np.linalg.norm(raw+.5-root,axis=1)/length,0,1)
        path=raw+side[None,:]*((band+.5)*2)*progress[:,None]
        refined['paths'].append(path.tolist());refined['observed_pixels'].extend(dict(p,leaf=index) for p in pixels)
        records.append(dict(leaf=index,parent_path=leaf,transverse_band=band,pixels=len(pixels)))
    result=build_curves(obj,source,refined,ground)
    result.update(geometry_version='narrow-rooted-native-blade-hypothesis-v1',source_partition=records,source_pixel_count=len(refined['observed_pixels']),partition_width_source_pixels=2,limitations=['Fine blade associations within overlapping native foliage are inferred; all source RGBA remains unchanged.','Requires actual oblique/source/contact review; splitting a source region does not itself establish plausible leaf geometry.'])
    return result
