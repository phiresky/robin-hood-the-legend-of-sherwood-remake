"""Inspect source draw-domain overlap with the foreground native canopy loop."""
import json, sys
from pathlib import Path
import numpy as np
from PIL import Image, ImageDraw
HERE=Path(__file__).resolve().parent;sys.path.insert(0,str(HERE));import qualify as q

def main():
    dest=q.OUT/'profile02-native-loop-composition-v1';assert not dest.exists()
    level_path=q.ROOT/'datadirs/fullgame_gog_hackable/Data/Levels/Croisement02.rhp.json'
    level=json.loads(level_path.read_text());animation=level['animations'][5]
    assert animation['sprite']['profile_name']=='Croisement02 - Arbre06' and animation['active']
    manifest_path=q.ROOT/'datadirs/fullgame_gog_hackable/Data/Animations/Day/Treecr02.rhs.d/manifest.json'
    profile=next(p for p in json.loads(manifest_path.read_text())['profiles'] if p['name']==animation['sprite']['profile_name'])
    looprow=next(r for r in profile['rows'] if r['action_id']==0 and r['direction']==0)
    bindings=json.loads(q.BIND.read_text());owners=[r for r in bindings['rows'] if r['profile']=='Croisement02 - hidden archer02']
    patch_min=[min(p[1] for p in r['patch']['element_fx']['display_polyline']) for r in owners]
    loop_min=min(p[1] for p in animation['display_polyline']);assert patch_min==[20,20,20] and loop_min==524
    qualification=json.loads((q.OUT/'qualification.json').read_text());row=qualification['rows'][1]
    prior=q.OUT/'joined02-v5-holdout-coverage-256';prior_report=json.loads((prior/'report.json').read_text());crop=prior_report['crop'];w,h=crop[2]-crop[0],crop[3]-crop[1]
    bg=Image.open(q.ROOT/qualification['background']['path']).convert('RGBA').crop(crop)
    loops=[];loop_pins=[]
    for f in looprow['frames']:
        path=manifest_path.parent/profile['name']/looprow['path']/f['file'];pin=q.pin(path);im=q.rgba({'image':pin})
        xy=[int(animation['sprite']['position_x']+f['offset_x']),int(animation['sprite']['position_y']+f['offset_y'])]
        layer=Image.new('RGBA',(w,h));layer.alpha_composite(im,(xy[0]-crop[0],xy[1]-crop[1]));loops.append(layer);loop_pins.append(dict(image=pin,top_left=xy))
    sheet=Image.new('RGB',(w*3,(h+24)*4),(24,24,24));draw=ImageDraw.Draw(sheet);records=[]
    frames=row['art']['states']['initial']['frames']+row['art']['states']['transition']['frames']
    for j,(phase,f) in enumerate(zip(['initial','transition-0','transition-1','applied'],frames)):
        patch=Image.new('RGBA',(w,h));patch.alpha_composite(q.rgba(f),(int(f['top_left'][0])-crop[0],int(f['top_left'][1])-crop[1]));mask=np.array(patch)[:,:,3]>0
        native=Image.alpha_composite(bg,patch)
        images=[native,Image.alpha_composite(native,loops[0]),Image.alpha_composite(native,loops[7])]
        for i,(label,im) in enumerate(zip(['Patch over backplate only','Then foreground loop frame0','Then foreground loop frame7'],images)):
            sheet.paste(im.convert('RGB'),(i*w,j*(h+24)+24));draw.text((i*w+3,j*(h+24)+4),phase+' / '+label,fill='white')
        covers=[np.array(im)[:,:,3]>0 for im in loops]
        record=dict(phase=phase,source=f['image'],body_pixels=int(mask.sum()),loop_covered_per_frame=[int((mask&c).sum())for c in covers])
        if phase in ['initial','applied']:
            actual=np.array(Image.open(prior/f'{phase}-joined.png').convert('RGBA'));lost=mask&(actual[:,:,3]<128)
            record.update(previous_geometry_lost=int(lost.sum()),lost_under_loop_per_frame=[int((lost&c).sum())for c in covers],lost_always_under_loop=int((lost&np.logical_and.reduce(covers)).sum()),lost_never_under_loop=int((lost&~np.logical_or.reduce(covers)).sum()))
        records.append(record)
    dest.mkdir();sheet.save(dest/'source-composition.png')
    (dest/'report.json').write_text(json.dumps(dict(status='NATIVE_SOURCE_COMPOSITION_REQUIRES_CANONICAL_OWNER_LINK',
        level=q.pin(level_path),bindings=q.pin(q.BIND),loop_manifest=q.pin(manifest_path),loop_frames=loop_pins,
        loop_animation_index=5,loop_profile=profile['name'],loop_min_display_y=loop_min,patch_min_display_y=patch_min,
        records=records,source_comparison=q.pin(dest/'source-composition.png'),recipe=q.pin(Path(__file__)),limits=[
        'Native draw ordering is lower display-polyline minimumY first. Source animation5 is foreground to the three qualified archer02 occurrences.',
        'Independent authored frame combinations, not a synthesized timeline or second clock.',
        'Decoded opaque-body overlap only; keyed shadow contribution is excluded.',
        'Canonical Tree04-to-loop ownership link remains to be confirmed by the delivery owner.',
        'No source, geometry, alpha, receiver domain or canonical asset changed.']),indent=2)+'\n')
    print(json.dumps(records),flush=True)

if __name__=='__main__':main()
