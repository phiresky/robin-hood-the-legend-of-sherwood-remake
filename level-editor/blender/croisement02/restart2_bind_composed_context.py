"""Bind an already reviewed native joint as an explicit gallery context link."""
import argparse,json,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT,reviewed_catalog
from evidence_io import sha,write_json
from restart2_composed_selections import selected_workspace

def main():
 p=argparse.ArgumentParser();p.add_argument('index',type=int);p.add_argument('native_joint',type=Path);a=p.parse_args();old=OUT/f'restart2-wood/composed-selections/tree-{a.index:02}-gallery-v2.json';new=old.with_name(old.name.replace('-v2','-v3'));record=json.loads(old.read_text());native=a.native_joint.resolve();w=Path(record['worker']);metadata=w/'inspection/composed-gallery.json'
 if new.exists() or metadata.exists():raise FileExistsError(new)
 if record['files'].get(str(native))!=sha(native):raise ValueError('Native joint not independently bound')
 write_json(metadata,dict(model_sha256=record['model_sha256'],native_joint=str(native),native_joint_sha256=sha(native),label='Exact native source and combined tree joint; independently reviewed, adjacent scope noted in receipt'))
 record['files'][str(metadata)]=sha(metadata);record['previous_receipt_sha256']=sha(old);write_json(new,record)
 if selected_workspace(OUT,a.index,reviewed_catalog())!=w:raise ValueError('Invalid gallery context selection')
 print(new)
if __name__=='__main__':main()
