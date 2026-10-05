"""Preserve separate native net body/debris phases and patch handoff timing."""
import hashlib
import json
from pathlib import Path
import numpy as np
from PIL import Image
from catalog import OUT
from restart2_native_appearance_glb import build


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def handoff_tick(frames):
    """The final counter reaching its delay terminates the patch transition."""
    index = count = tick = 0
    while True:
        tick += 1
        count += 1
        if count > frames[index]['delay']:
            count = 0
            index += 1
        if index >= len(frames):
            index = 0
        if index == len(frames)-1 and (count == frames[index]['delay'] or frames[index]['delay'] == 0):
            return tick


def main():
    source = OUT/'net-state-source-review-v1/manifest.json'
    data = json.loads(source.read_text())
    bindings = []
    for family in ['piege01', 'piege03']:
        instances = [r for r in data['net_instances'] if r['instance']['target']['profile_name'] == f'Croisement02 - {family}h']
        representative = instances[0]
        for variant in 'eig':
            patch = next(p for p in representative['patches'] if p['name'].endswith(family+variant))
            for instance in instances:
                other = next(p for p in instance['patches'] if p['name'] == patch['name'])
                for state in ['transition', 'final']:
                    expected, actual = patch['states'][state]['frames'], other['states'][state]['frames']
                    assert len(expected) == len(actual)
                    for first, second in zip(expected, actual):
                        assert all(first[key] == second[key] for key in ['bbox', 'delay', 'sound_id'])
                        assert sha(OUT/'source-states'/first['image']) == sha(OUT/'source-states'/second['image'])
            frames = [f for state in ['transition', 'final'] for f in patch['states'][state]['frames']]
            left = min(f['bbox'][0] for f in frames)
            top = min(f['bbox'][1] for f in frames)
            right = max(f['bbox'][0]+f['bbox'][2] for f in frames)
            bottom = max(f['bbox'][1]+f['bbox'][3] for f in frames)
            transition = patch['states']['transition']['frames']
            boundary = handoff_tick(transition)
            assert transition[-1]['delay'] > 0
            assert boundary == sum(f['delay']+1 for f in transition)-1
            for state in ['transition', 'final']:
                name = f'net-{family}-{variant}-{state}'
                dest = OUT/'restart2-state'/f'{name}-source-v1'
                dest.mkdir(exist_ok=False)
                tick = 0
                records = []
                for index, frame in enumerate(patch['states'][state]['frames']):
                    path = OUT/'source-states'/frame['image']
                    image = Image.open(path).convert('RGBA')
                    pixels = np.asarray(image)
                    assert not np.any(np.all(pixels[:,:,:3] == [0,0,255], axis=2) & (pixels[:,:,3]>0)), 'Shadow operator requires separate export'
                    canvas = Image.new('RGBA', (right-left,bottom-top))
                    canvas.alpha_composite(image, (frame['bbox'][0]-left,frame['bbox'][1]-top))
                    filename = f'{index:03}.png'
                    canvas.save(dest/filename)
                    duration = frame['delay']+1
                    if state == 'transition' and index == len(transition)-1:
                        duration -= 1
                    records.append(dict(first_tick=tick,last_tick=tick+duration-1,image=filename,rgba_sha256=hashlib.sha256(canvas.tobytes()).hexdigest(),source_png_sha256=sha(path),source_bbox=frame['bbox'],sound_id=frame['sound_id']))
                    tick += duration
                if state == 'transition':
                    assert tick == boundary
                manifest = dict(status='Exact source appearance; physical net motion and scene composition remain separate',records=records,tick_rate=25,terminal_tick=tick-1,cycle_ticks=tick if state=='final' else None,transition_to_final_tick=boundary,screen_bounds=[left,top,right,bottom],source_manifest_sha256=sha(source),patch_id=patch['id'],family=family,variant=variant,state=state,role='Loose leaf/debris effect' if variant=='g' else 'Alternative net body; choose e or i, never both',limitations=['Clip presentation is planar native appearance, not recovered solid movement.','Transition must switch to final clip at the recorded boundary; standalone transition clamp is only a source review.','Covered target action, actor capture, native ordering and receiver metadata remain separate.'])
                (dest/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
                build(name,source=dest,loop=state=='final')
                bindings.append(dict(id=name,source_manifest_sha256=sha(dest/'manifest.json'),transition_to_final_tick=boundary,frame_count=len(records),cycle_ticks=tick if state=='final' else None,mission_patches=[dict(mission=r['instance']['mission'],patch_id=next(p['id'] for p in r['patches'] if p['name']==patch['name'])) for r in instances]))
    (OUT/'restart2-state/net-export-bindings-v1.json').write_text(json.dumps(dict(status='12 source clips exported; browser and integrated handoff proof pending',clips=bindings,source_manifest_sha256=sha(source),timing='Per-frame delay+1; transition final frame emits completion at counter==delay, and patch switches to final immediately. Final idle ignores completion and loops through the complete native duration.'),indent=2)+'\n')


if __name__ == '__main__':
    main()
