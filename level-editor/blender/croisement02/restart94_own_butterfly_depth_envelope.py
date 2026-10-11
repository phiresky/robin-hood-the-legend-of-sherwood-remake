"""Construct conservative per-piece depth exclusions for an own-source flight candidate."""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1');os.environ.setdefault('OMP_NUM_THREADS','1')
import argparse,json,struct
from functools import partial
from pathlib import Path
import restart94_piece_envelope_resume as envelope

def main():
 p=argparse.ArgumentParser();p.add_argument('--candidate',type=Path,required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args();assert not a.output.exists()
 envelope.reader.main=partial(envelope.reader.main,expected_map_sha256=envelope.reader.sha(envelope.reader.LIB/'scenes/croisement02.rhlos-map.json'))
 map_path=envelope.reader.LIB/'scenes/croisement02.rhlos-map.json';mapdoc=json.loads(map_path.read_text());models={}
 for row in [*mapdoc['assetSources'],*mapdoc['sceneAssets']]:
  model=envelope.reader.LIB/row['model'];models[str(model)]=envelope.reader.sha(model)
  with model.open('rb')as stream:header=stream.read(20);length=struct.unpack_from('<I',header,12)[0];doc=json.loads(stream.read(length))
  for buffer in doc['buffers']:
   if 'uri'in buffer:
    resource=model.parent/buffer['uri'];models[str(resource)]=envelope.reader.sha(resource)
 envelope.CACHE=a.output.with_suffix('.checkpoint.json');envelope.CHECKPOINT_PINS=dict(map_sha256=envelope.reader.sha(map_path),fit_sha256=envelope.reader.sha(a.candidate/'fit.json'),geometry_sha256=envelope.reader.sha(a.candidate/'geometry.py'),recipe_sha256=envelope.reader.sha(Path(envelope.__file__)),models=models)
 envelope.pinned.FIT=a.candidate/'fit.json';envelope.pinned.FIT_SHA=envelope.reader.sha(envelope.pinned.FIT);envelope.pinned.HELPER=a.candidate/'geometry.py';envelope.pinned.HELPER_SHA=envelope.reader.sha(envelope.pinned.HELPER);envelope.OUT=a.output;envelope.main()
if __name__=='__main__':main()
