"""Prepare Tree18's inferred rear crown from its own source branches and leaf samples."""
import hashlib
import json
import math
import shutil
from pathlib import Path
import numpy as np
from PIL import Image, ImageDraw
R=Path(__file__).resolve().parents[3]
B=R/'level-editor/work/croisement03-refinement/restart2'
S,C=math.sin(math.radians(35)),math.cos(math.radians(35))
RAY=np.array([0.,-C,S]); VERTICAL=np.array([0.,-S,-C])

def sha(p): return hashlib.sha256(p.read_bytes()).hexdigest()

def main():
    out=B/'tree18-regional-crown-cpu-v1'; assert not out.exists()
    regional=B/'tree18-regional-plants-source-v1/receipt.json'
    donorpath=B/'tree18-native-leaf-donors-v1/receipt.json'
    woodpath=B/'tree18-root-connectivity-cpu-v1/geometry.json'
    region=json.loads(regional.read_text()); donor=json.loads(donorpath.read_text()); wood=json.loads(woodpath.read_text())
    pins={str(p):sha(p) for p in (regional,donorpath,woodpath)}
    for packet in (region,donor):
        for name,digest in packet['pins'].items(): assert sha(Path(name))==digest,name
    atlas=donorpath.parent/'exact-rgba-atlas.png'; assert sha(atlas)==donor['output_pins'][atlas.name]
    assert Image.open(atlas).size==(192,16)
    crown=next(r for r in region['regions'] if r['id']=='gold-tree18-crown-envelope')
    mask=Image.new('L',(1408,960)); ImageDraw.Draw(mask).polygon([tuple(p) for p in crown['outline']],fill=255); domain=np.array(mask)>0
    wood_y=float(np.median([v[1] for p in wood['parts'] for v in p['vertices']]))
    # Camera-depth spreads are hypotheses anchored to the wood scaffold, never
    # to the editor animation plane. Supports follow distinct source sprays.
    supports=[(335,225,75,45,220,190),(415,218,78,46,265,220),(490,255,70,55,195,190),
              (535,300,48,50,150,160),(285,290,58,65,160,180),(375,295,62,60,165,180),
              (457,333,61,44,140,170),(315,395,35,65,55,150)]
    rng=np.random.default_rng(18122026); vertices=[]; faces=[]; uvs=[]; clusters=[]; rows=[]
    for index,(x,y,rx,ry,rd,count) in enumerate(supports):
        center=np.array([x,wood_y,(-wood_y*S-y)/C]); rows.append({'native_center':[x,y],'native_radii':[rx,ry],'inferred_ray_radius':rd,'world_center':center.tolist()})
        accepted=0
        for attempt in range(count*40):
            if accepted==count:break
            unit=rng.normal(size=3); unit/=np.linalg.norm(unit); unit*=rng.random()**(1/3)
            angle=math.atan2(unit[1],unit[0]); modulation=1+.1*math.sin(5*angle+index)+.07*math.cos(9*angle-index)
            position=center+np.array([rx*unit[0],0,0])*modulation+VERTICAL*ry*unit[1]*modulation+RAY*rd*unit[2]
            nx,ny=float(position[0]),float(-position[1]*S-position[2]*C)
            if not(0<=nx<1408 and 0<=ny<960 and domain[int(ny),int(nx)]):continue
            axis=rng.normal(size=3);axis/=np.linalg.norm(axis);other=np.cross(axis,[0,0,1] if abs(axis[2])<.9 else [1,0,0]);other/=np.linalg.norm(other);third=np.cross(axis,other)
            size=float(rng.uniform(3.2,6.0)); sample=int(rng.integers(12)); coords=[[sample/12,1],[(sample+1)/12,1],[(sample+1)/12,0],[sample/12,0]]
            for u,v in ((axis,other),(axis,third),(other,third)):
                points=[position+(u*su+v*sv)*size for su,sv in ((-1,-1),(1,-1),(1,1),(-1,1))]
                if np.dot(np.cross(points[1]-points[0],points[2]-points[0]),RAY)>0:points=points[::-1]
                start=len(vertices);vertices.extend(p.tolist() for p in points);uvs.extend(coords);faces.extend([[start,start+1,start+2],[start,start+2,start+3]])
            clusters.append({'support':index,'center':position.tolist(),'source_projection':[nx,ny],'donor':sample,'half_size':size});accepted+=1
        assert accepted==count
    points=np.array(vertices); triangles=np.array(faces); normals=np.cross(points[triangles[:,1]]-points[triangles[:,0]],points[triangles[:,2]]-points[triangles[:,0]])
    width,depth=float(np.ptp(points[:,0])),float(np.ptp(points[:,1]));assert depth>=width,(width,depth);assert np.max(normals@RAY)<=1e-7
    out.mkdir(); shutil.copyfile(atlas,out/'own-native-gold-atlas.png')
    report={'status':'PRIVATE_REGIONAL_REAR_CROWN_GEOMETRY_PROPOSAL_NOT_SAVED_REVIEWED','pins':pins,'vertices':vertices,'faces':faces,'uvs':uvs,'supports':rows,'clusters':clusters,'world_bounds':[points.min(0).tolist(),points.max(0).tolist()],'width':width,'depth':depth,'wood_scaffold_depth_center_y':wood_y,'atlas_sha256':sha(out/'own-native-gold-atlas.png'),'observed_faces':0,'inferred_rear_faces':len(faces),'required_material':'Exact own-native gold atlas, nearest sampling, one-sided inferred rear foliage; no observed native replacement.',
            'required_joined_checks':['Saved bank/wood/terrain contact and eight native/oblique views; enclosing plant masses separate from pixel ownership.','Exact all14 native alpha and confirmed stone/bark rays, one observed owner.','256/512 transparent-bounce convergence before density or shape changes.','Preserve all22406 native union pixels including2225 outside hand envelope.'],
            'limits':['No observed static foliage assigned. Source envelope guides cluster centers only, not opaque textures or native-alpha clipping.','All hidden depth, branch support spread and physical attachment remain hypotheses pending joined saved review.','Green ivy/right shrub/root collar require separate geometry/materials; gold donors must not be reused for green cover.','No other tree assets used as shape references; geometry derives from this tree source/wood scaffold.']}
    (out/'geometry.json').write_text(json.dumps(report,separators=(',',':'))+'\n')
    source=Image.open(B.parent/'baseline/covered.png').convert('RGB'); draw=ImageDraw.Draw(source)
    for i,row in enumerate(rows):
        x,y=row['native_center'];rx,ry=row['native_radii'];draw.ellipse((x-rx,y-ry,x+rx,y+ry),outline='#00ffff',width=1);draw.text((x,y),str(i+1),fill='white')
    source.crop((190,150,620,475)).resize((860,650),Image.Resampling.NEAREST).save(out/'support-source-preview.png')
    print(json.dumps({'clusters':len(clusters),'faces':len(faces),'width':width,'depth':depth,'world_bounds':report['world_bounds']}))

if __name__=='__main__':main()
