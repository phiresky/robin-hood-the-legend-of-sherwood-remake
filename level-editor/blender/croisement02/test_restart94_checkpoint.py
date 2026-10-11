"""A real envelope callback interrupted at its atomic checkpoint must resume identically."""
import json,tempfile
from pathlib import Path
import numpy as np
import restart94_piece_envelope_resume as envelope

def main():
 base=Path(__file__).resolve().parents[3]/'level-editor/work/croisement02-refinement';candidate=base/'restart83-butterfly04-contact-candidate-v1';fit=json.loads((candidate/'fit.json').read_text());envelope.pinned.FIT=candidate/'fit.json';envelope.pinned.FIT_SHA=envelope.reader.sha(envelope.pinned.FIT);envelope.pinned.HELPER=candidate/'geometry.py';envelope.pinned.HELPER_SHA=envelope.reader.sha(envelope.pinned.HELPER);envelope.N=1
 x,y=fit['rows'][0]['source']['alpha_centroid_display'];depth=envelope.reader.COS*y/envelope.reader.SIN+25/envelope.reader.SIN;camera=np.array([[x-12,y-12,depth],[x+12,y-12,depth],[x,y+12,depth]]);world=np.c_[camera[:,0],-envelope.reader.COS*camera[:,1]+envelope.reader.SIN*camera[:,2],envelope.reader.SIN*camera[:,1]+envelope.reader.COS*camera[:,2]]
 def reader(**kw):
  for number in range(2):kw['triangle_callback']({'id':f'fixture-receiver-{number}'},{'name':'Fixture'},0,0,(world+np.array([number,0,0]))[None],None,{'alphaMode':'OPAQUE'},lambda *args:1,lambda *args:(None,{},1))
  return kw['postprocess']([],[],envelope.reader.LIB/'scenes/croisement02.rhlos-map.json')
 envelope.reader.main=reader;envelope.CHECKPOINT_PINS={'case':'exact-resume-fixture','fit':envelope.pinned.FIT_SHA}
 with tempfile.TemporaryDirectory(prefix='canopy-depth-resume-',dir='/tmp')as temporary:
  root=Path(temporary);envelope.OUT=root/'baseline';envelope.CACHE=None;envelope.main();baseline=json.loads((envelope.OUT/'report.json').read_text())
  envelope.OUT=root/'resumed';envelope.CACHE=root/'checkpoint.json';replace=Path.replace;triggered=False
  class Interrupted(Exception):pass
  def interrupt(path,target):
   nonlocal triggered
   result=replace(path,target)
   if path==envelope.CACHE.with_suffix('.tmp') and not triggered:triggered=True;raise Interrupted()
   return result
  Path.replace=interrupt
  try:
   try:envelope.main()
   except Interrupted:pass
  finally:Path.replace=replace
  assert triggered and envelope.CACHE.exists() and not envelope.OUT.exists();envelope.main();resumed=json.loads((envelope.OUT/'report.json').read_text());assert baseline==resumed
 print('PASS interrupted envelope resumed exactly with no duplicated bands or counters')
if __name__=='__main__':main()
