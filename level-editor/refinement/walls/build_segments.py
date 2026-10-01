"""Extract repeatable wall strips, preserving source UVs/materials and recording provenance.

Outputs are independent derived assets. Original map models and placements are untouched.
The receipt records clipping, orientation, translation and any copy-only straightening.
"""
import copy
import argparse
import hashlib
import json
import math
from pathlib import Path
import struct
import sys
import numpy as np
from author_gameplay import strip_gameplay

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT/'refinement/blender'))
from lossy_assets import read_glb, accessor_array, node_matrix
sys.path.insert(0, str(ROOT/'refinement'))
from asset_index import write_asset_index

LIB = ROOT/'library/3d-assets'
STAGE = ROOT/'work/wall-presets/staging/3d-assets'

def sha(path): return hashlib.sha256(Path(path).read_bytes()).hexdigest()

def primitives(doc, buffers, scene_name, hidden_nodes=()):
    scene = next(s for s in doc['scenes'] if s.get('name') == scene_name) if scene_name else doc['scenes'][doc.get('scene',0)]
    def walk(index, parent, hidden=False):
        node=doc['nodes'][index];extras=node.get('extras',{})
        hidden=hidden or extras.get('default_hidden',False) or node.get('name') in hidden_nodes
        world=parent if node.get('name')=='map' and 'mesh' not in node else parent@node_matrix(node)
        if not hidden and 'mesh' in node:
            for primitive in doc['meshes'][node['mesh']]['primitives']:
                if primitive.get('mode',4)!=4: continue
                attrs={key:accessor_array(doc,buffers,value,True) for key,value in primitive['attributes'].items()}
                p=attrs['POSITION'];attrs['POSITION']=(np.c_[p,np.ones(len(p))]@world.T)[:,:3]
                if 'NORMAL' in attrs: attrs['NORMAL']=attrs['NORMAL']@np.linalg.inv(world[:3,:3])
                attrs.pop('TANGENT',None)
                ids=accessor_array(doc,buffers,primitive['indices']) if 'indices' in primitive else np.arange(len(p))
                yield attrs,ids.reshape(-1,3),primitive.get('material'),node.get('name')
        for child in node.get('children',[]):yield from walk(child,world,hidden)
    for index in scene['nodes']:yield from walk(index,np.eye(4))

def clip(poly, axis, boundary, above):
    out=[]
    for a,b in zip(poly,poly[1:]+poly[:1]):
        av,bv=a['POSITION'][axis],b['POSITION'][axis];ia=av>=boundary if above else av<=boundary;ib=bv>=boundary if above else bv<=boundary
        if ia:out.append(a)
        if ia!=ib:
            t=(boundary-av)/(bv-av);out.append({k:a[k]+(b[k]-a[k])*t for k in a})
    return out

def clip_parts(parts, box):
    clipped=[]
    for attrs,triangles,material,name in parts:
        values={k:[] for k in attrs}
        for tri in triangles:
            poly=[{k:v[i].copy() for k,v in attrs.items()} for i in tri]
            for axis,limits in enumerate(box):
                if limits is not None:poly=clip(clip(poly,axis,limits[0],True),axis,limits[1],False)
            for i in range(1,len(poly)-1):
                for v in (poly[0],poly[i],poly[i+1]):
                    for k in values:values[k].append(v[k])
        if values['POSITION']:
            values={k:np.array(v) for k,v in values.items()}
            clipped.append((values,np.arange(len(values['POSITION'])).reshape(-1,3),material,name))
    return clipped

def source_geometry(recipe, entries):
    entry=entries[recipe['source']];path=LIB/entry['model'];doc,buffers,_=read_glb(path)
    descriptor=json.loads((LIB/entry['descriptor']).read_text())
    hidden_nodes={part['node'] for part in descriptor.get('parts',[]) if part.get('default_hidden')}
    parts=list(primitives(doc,buffers,entry.get('model_scene'),hidden_nodes))
    if recipe.get('include_nodes'):
        parts=[p for p in parts if any(name in (p[3] or '') for name in recipe['include_nodes'])]
    if recipe.get('clip_box'):
        parts=clip_parts(parts,recipe['clip_box'])
    if not parts:raise ValueError('No selected geometry: '+recipe['id'])
    points=np.concatenate([p[0]['POSITION'] for p in parts]);center=points[:,:2].mean(axis=0)
    _,axes=np.linalg.eigh((points[:,:2]-center).T@(points[:,:2]-center));direction=axes[:,-1]
    angle=recipe.get('angle',math.degrees(math.atan2(direction[1],direction[0])))
    if 'angle' not in recipe:
        if angle>90:angle-=180
        if angle<=-90:angle+=180
    a=math.radians(angle);rot=np.array([[math.cos(a),math.sin(a),0],[-math.sin(a),math.cos(a),0],[0,0,1]])
    for attrs,_,_,_ in parts:
        attrs['POSITION']=attrs['POSITION']@rot.T
        if 'NORMAL' in attrs:attrs['NORMAL']=attrs['NORMAL']@rot.T
    if 'cross_interval' in recipe:
        parts=clip_parts(parts,[None,recipe['cross_interval']])
        if not parts:raise ValueError('Cross-section crop removed all geometry: '+recipe['id'])
    return entry,path,doc,buffers,parts,angle

def feature_intervals(parts, height):
    """Project geometry above a reviewed height onto the longitudinal axis."""
    intervals=[]
    for attrs,triangles,_,_ in parts:
        for tri in triangles:
            polygon=clip([{'POSITION':attrs['POSITION'][i]} for i in tri],2,height,True)
            if polygon:
                xs=[v['POSITION'][0] for v in polygon]
                intervals.append((min(xs),max(xs)))
    merged=[]
    for start,end in sorted(intervals):
        if merged and start<=merged[-1][1]+1e-4:
            merged[-1][1]=max(merged[-1][1],end)
        else:merged.append([start,end])
    return merged

def repeat_interval(features, start, end):
    """Cut at gap midpoints, retaining whole features and a natural seam gap.

    At the join, the two half gaps sum to the mean of the source gaps. This
    preserves irregular masonry instead of forcing every merlon to one pitch.
    """
    gaps=[(a[1]+b[0])/2 for a,b in zip(features,features[1:]) if b[0]-a[1]>1e-3]
    gaps=[x for x in gaps if start<=x<=end]
    if len(gaps)<2:raise ValueError('Repeat window needs at least two complete feature gaps')
    start,end=gaps[0],gaps[-1]
    inside=[(a,b) for a,b in features if start<a and b<end]
    if not inside:raise ValueError('Repeat contains no complete features')
    return start,end,{'features':len(inside),
                    'seam_gap':end-inside[-1][1]+inside[0][0]-start,
                    'internal_gaps':[b[0]-a[1] for a,b in zip(inside,inside[1:])]}

def build(recipe, entries):
    entry,path,doc,buffers,parts,angle=source_geometry(recipe,entries)
    points=np.concatenate([p[0]['POSITION'] for p in parts]);lo=points.min(axis=0);hi=points.max(axis=0)
    start=lo[0]+(hi[0]-lo[0])*recipe.get('start',.2);end=lo[0]+(hi[0]-lo[0])*recipe.get('end',.8)
    if 'interval' in recipe:start,end=recipe['interval']
    if not lo[0]<=start<end<=hi[0]:raise ValueError('Repeat interval outside source geometry')
    repeat_check=None
    if 'feature_height' in recipe:
        features=feature_intervals(parts,recipe['feature_height'])
        start,end,repeat_check=repeat_interval(features,start,end)
        if recipe.get('level_feature_tops'):
            tops=[((a+b)/2,points[(points[:,0]>=a)&(points[:,0]<=b),2].max())
                  for a,b in features if start<a and b<end]
            if len(tops)<2:raise ValueError('Leveling requires two complete features')
            xs,zs=np.array(tops).T
            slope,intercept=np.polyfit(xs,zs,1)
            target=slope*(start+end)/2+intercept-lo[2]
            for attrs,_,_,_ in parts:
                p=attrs['POSITION']
                p[:,2]=lo[2]+(p[:,2]-lo[2])*target/(slope*p[:,0]+intercept-lo[2])
    output=[]
    for attrs,triangles,material,name in parts:
        values={k:[] for k in attrs}
        for tri in triangles:
            poly=[{k:v[i].copy() for k,v in attrs.items()} for i in tri]
            for a,b in zip(np.linspace(start,end,33)[:-1],np.linspace(start,end,33)[1:]):
                band=clip(clip(poly,0,a,True),0,b,False)
                for i in range(1,len(band)-1):
                    for v in (band[0],band[i],band[i+1]):
                        for k in values:values[k].append(v[k])
        if values['POSITION']:output.append(({k:np.array(v) for k,v in values.items()},material,name))
    if not output:raise ValueError('Empty wall segment: '+recipe['id'])
    if recipe.get('straighten'):
        vertices=np.concatenate([x[0]['POSITION'] for x in output])
        stations=np.linspace(start,end,33)
        spans=[]
        for station in stations:
            section=vertices[np.abs(vertices[:,0]-station)<1e-4]
            if not len(section):raise ValueError('Gap in selected strip: '+recipe['id'])
            spans.append((section[:,1].min(),section[:,1].max(),section[:,2].min(),section[:,2].max()))
        spans=np.array(spans);widths=spans[:,1]-spans[:,0];heights=spans[:,3]-spans[:,2]
        if np.min(widths)<.001:raise ValueError('Tapered strip needs narrower trim: '+recipe['id'])
        target_width=np.median(widths);target_height=np.median(heights)
        for attrs,_,_ in output:
            p=attrs['POSITION'];center=np.interp(p[:,0],stations,(spans[:,0]+spans[:,1])/2)
            width=np.interp(p[:,0],stations,widths)
            p[:,1]=(p[:,1]-center)*target_width/width
            if recipe.get('level_top'):
                bottom=np.interp(p[:,0],stations,spans[:,2]);height=np.interp(p[:,0],stations,heights)
                p[:,2]=(p[:,2]-bottom)*target_height/height
            # Geometry has changed only in this dedicated copy. Recompute its flat normals.
            triangles=p.reshape(-1,3,3);normals=np.cross(triangles[:,1]-triangles[:,0],triangles[:,2]-triangles[:,0])
            normals/=np.maximum(np.linalg.norm(normals,axis=1)[:,None],1e-12)
            attrs['NORMAL']=np.repeat(normals,3,axis=0)
    vertices=np.concatenate([x[0]['POSITION'] for x in output]);lo=vertices.min(axis=0);hi=vertices.max(axis=0)
    anchor=np.array([(start+end)/2,(lo[1]+hi[1])/2,lo[2]])
    for attrs,_,_ in output:attrs['POSITION']-=anchor
    if repeat_check:
        final_parts=[(attrs,np.arange(len(attrs['POSITION'])).reshape(-1,3),material,name)
                     for attrs,material,name in output]
        final_features=feature_intervals(final_parts,recipe['feature_height']-anchor[2])
        seam=(end-start)-final_features[-1][1]+final_features[0][0] if final_features else -1
        if (len(final_features)!=repeat_check['features'] or
                abs(seam-repeat_check['seam_gap'])>1e-3 or
                final_features[0][0]<=start-anchor[0]+1e-4 or
                final_features[-1][1]>=end-anchor[0]-1e-4):
            raise ValueError('Generated strip no longer preserves complete repeat features: '+recipe['id'])
    binary=bytearray();views=[];accessors=[]
    def blob(data):
        while len(binary)%4:binary.append(0)
        index=len(views);views.append({'buffer':0,'byteOffset':len(binary),'byteLength':len(data)});binary.extend(data);return index
    def accessor(values):
        values=np.asarray(values,dtype='<f4');index=len(accessors)
        kind={1:'SCALAR',2:'VEC2',3:'VEC3',4:'VEC4'}[values.shape[1] if values.ndim>1 else 1]
        accessors.append({'bufferView':blob(values.tobytes()),'componentType':5126,'count':len(values),'type':kind,
                          'min':values.min(axis=0).tolist(),'max':values.max(axis=0).tolist()})
        return index
    out={'asset':{'version':'2.0','generator':'Sherwood reviewed wall-strip extraction'},'scene':0,'scenes':[{'name':'default','nodes':[0]}],
         'nodes':[{'name':'map','rotation':[-math.sqrt(.5),0,0,math.sqrt(.5)],'children':[1]},
                  {'name':recipe['name'],'extras':{'asset_group':recipe['id']},'children':[2]},
                  {'name':'scenery-wall-strip','extras':{'scenery':True},'children':[]}],
         'meshes':[],'materials':copy.deepcopy(doc.get('materials',[])),'textures':copy.deepcopy(doc.get('textures',[])),
         'samplers':copy.deepcopy(doc.get('samplers',[])),'images':[]}
    for image in doc.get('images',[]):
        if 'uri' in image:
            data=(path.parent/image['uri']).resolve().read_bytes();suffix=Path(image['uri']).suffix.lower();mime={'.jpg':'image/jpeg','.jpeg':'image/jpeg','.png':'image/png','.webp':'image/webp','.avif':'image/avif'}[suffix]
        else:
            view=doc['bufferViews'][image['bufferView']];offset=view.get('byteOffset',0);data=buffers[view.get('buffer',0)][offset:offset+view['byteLength']];mime=image['mimeType']
        out['images'].append({'bufferView':blob(data),'mimeType':mime})
    for attrs,material,name in output:
        if 'NORMAL' in attrs:
            normals=attrs['NORMAL'];normals/=np.maximum(np.linalg.norm(normals,axis=1)[:,None],1e-12)
        primitive={'attributes':{k:accessor(v) for k,v in attrs.items()},'mode':4}
        if material is not None:primitive['material']=material
        out['meshes'].append({'primitives':[primitive]});index=len(out['nodes']);out['nodes'][2]['children'].append(index)
        out['nodes'].append({'name':name,'mesh':len(out['meshes'])-1})
    for key in ['extensionsUsed','extensionsRequired']:
        if key in doc:out[key]=[e for e in doc[key] if e not in ['KHR_mesh_quantization','EXT_meshopt_compression']]
    out.update(bufferViews=views,accessors=accessors,buffers=[{'byteLength':len(binary)}])
    json_bytes=json.dumps(out,separators=(',',':')).encode();json_bytes+=b' '*((-len(json_bytes))%4);binary+=b'\0'*((-len(binary))%4)
    glb=struct.pack('<4sII',b'glTF',2,12+8+len(json_bytes)+8+len(binary))+struct.pack('<II',len(json_bytes),0x4e4f534a)+json_bytes+struct.pack('<II',len(binary),0x004e4942)+binary
    folder=STAGE/recipe['id'];folder.mkdir(parents=True,exist_ok=True)
    model_path=folder/'model.glb'
    if not model_path.exists() or model_path.read_bytes()!=glb:
        # These staged derivatives bind the old model bytes. The publisher
        # regenerates them after the new source has passed visual review.
        for name in ('lossy.glb','lossy.glb.receipt.json','preview.glb','preview.glb.receipt.json'):
            (folder/name).unlink(missing_ok=True)
    model_path.write_bytes(glb)
    gameplay = strip_gameplay([triangle for attrs, _, _ in output for triangle in attrs['POSITION'].reshape(-1, 3, 3)],
                              'scenery-wall-strip', **recipe['collision'])
    gameplay['spline']['modelSha256'] = sha(model_path)
    descriptor={'version':1,'kind':'projection-mapped-asset','id':recipe['id'],'name':recipe['name'],'source_map':entry['source_map'],
                'asset_type':'Wall','tags':['spline-wall','derived-strip'],'model':'model.glb','model_scene':'default','resources':[],
                'parts':[{'node':'scenery-wall-strip','name':recipe['name'],'scenery':True}],
                'gameplay':gameplay,
                'bounds_local_scene':{'min':(lo-anchor).tolist(),'max':(hi-anchor).tolist()},
                'provenance':{'source':recipe['source'],'model_sha256':sha(path),'descriptor_sha256':sha(LIB/entry['descriptor']),
                              'recipe':recipe,'angle':angle,'anchor':anchor.tolist(),'interval':[start,end],
                              'repeat_check':repeat_check,
                              'method':('Dedicated copy: clipped original UVs/materials, straightened cross-sections'+('; leveled top' if recipe.get('level_top') else '') if recipe.get('straighten') else 'Triangle clipping with interpolated original attributes; rigid normalization only.')}}
    (folder/'asset.json').write_text(json.dumps(descriptor,indent=2)+'\n')
    return {**recipe,'source_map':entry['source_map'],'asset':recipe['id'],'axis':'x','sourceStraight':True,'sourceAngle':0,'sourceStart':0,'sourceEnd':1,
            'width':float(hi[1]-lo[1]),'repeatLength':float(end-start),'height':float(hi[2]-lo[2]),'source_angle':angle}

if __name__=='__main__':
    parser=argparse.ArgumentParser()
    parser.add_argument('--ids',nargs='+',help='Rebuild only these presets, preserving other staged receipts')
    args=parser.parse_args()
    recipes=json.loads((Path(__file__).parent/'recipes.json').read_text());entries={x['id']:x for x in json.loads((LIB/'index.json').read_text())['assets']}
    receipts=ROOT/'work/wall-presets/segments.json'
    previous={row['id']:row for row in json.loads(receipts.read_text())} if args.ids else {}
    if args.ids and set(args.ids)-{r['id'] for r in recipes}:raise ValueError('Unknown preset id')
    results=[]
    for recipe in recipes:
        if args.ids and recipe['id'] not in args.ids:
            results.append(previous[recipe['id']]);continue
        result=build(recipe,entries);results.append(result);print(recipe['id'],round(result['repeatLength']),round(result['width']),round(result['height']))
    write_asset_index(STAGE)
    scenes=STAGE.parent/'scenes';scenes.mkdir(exist_ok=True)
    (scenes/'index.json').write_text('[]\n')
    (ROOT/'work/wall-presets/segments.json').write_text(json.dumps(results,indent=2)+'\n')
