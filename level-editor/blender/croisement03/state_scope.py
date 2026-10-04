"""Hash and summarize native state frames without substituting static appearances."""
import collections
import hashlib
import json
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'level-editor/work/croisement03-refinement'
def main():
    folder=OUT/'source-states';d=json.loads((folder/'layers.json').read_text());families=collections.defaultdict(list);total=0
    for patch in d['mission_patches']:
        states={}
        for name,value in patch['states'].items():
            frames=[]
            for frame in value['frames']:
                path=folder/frame['image']
                frames.append(dict(frame,sha256=hashlib.sha256(path.read_bytes()).hexdigest()))
                total+=1
            states[name]=dict(frame_count=len(frames),frames=frames)
        families[patch['name']].append(dict(id=patch['id'],mission=patch['mission'],runtime_patch_index=patch['runtime_patch_index'],sprite=patch['state']['element_fx']['sprite'],states=states,sight_before=patch['sight_before'],sight_after=patch['sight_after'],status='native evidence extracted; state model and timing review pending'))
    result=dict(map='Croisement03',mission_count=len({p['mission'] for p in d['mission_patches']}),mission_patch_count=len(d['mission_patches']),native_patch_count=len(d['patches']),frame_files=total,profile_families=dict(families),native_patches=d['patches'],semantics='Distinct positions and mission triggers are retained even when graphic profile names match. Extraction does not establish modeled runtime state coverage. Patch application changes state immediately when invoked; sequence animation and visual timing require separate command-order review.')
    (OUT/'state-scope.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps({k:result[k] for k in ['mission_count','mission_patch_count','native_patch_count','frame_files']}))
if __name__=='__main__':main()
