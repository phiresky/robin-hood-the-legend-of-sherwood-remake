"""Check measured reconstruction constraints against the actual source frames."""
import json,unittest
import numpy as np
from PIL import Image
import qualify as q
from reconstruct import exact_runs
class SourceConstraints(unittest.TestCase):
 def test_all_sixteen_native_masks_round_trip(self):
  rows=json.loads((q.OUT/'qualification.json').read_text())['rows'];count=0
  for r in rows:
   for f in r['art']['states']['initial']['frames']+r['art']['states']['transition']['frames']:
    source=np.array(q.rgba(f));mask=np.zeros(source.shape[:2],dtype=bool);x0,y0=map(int,f['top_left'])
    for col in exact_runs(f):
     for lo,hi in col['runs']:mask[lo-y0:hi-y0,col['x']-x0]=True
    self.assertTrue(np.array_equal(mask,source[:,:,3]>0));raw=np.array(Image.open(q.ROOT/f['image']['path']).convert('RGBA'));self.assertTrue(np.array_equal(source[mask,:3],raw[mask,:3]));count+=1
  self.assertEqual(count,16)
 def test_physical_gap_is_not_texture_only(self):
  for r in json.loads((q.OUT/'qualification.json').read_text())['rows']:
   self.assertGreater(r['initial_to_terminal']['removed_pixels'],1900)
   self.assertGreater(r['initial_to_terminal']['added_pixels'],600)
   self.assertIsNone(r['art']['states']['final'])
   self.assertEqual(r['art']['states']['transition']['eligible_updates_to_termination'],6)
 def test_authoritative_frame_pins_and_seventeen_occurrences(self):
  rows=json.loads((q.OUT/'qualification.json').read_text())['rows'];self.assertEqual(sum(len(r['occurrences'])for r in rows),17)
  for r in rows:
   for f in r['art']['states']['initial']['frames']+r['art']['states']['transition']['frames']:
    self.assertEqual(q.pin(q.ROOT/f['image']['path']),f['image'])
if __name__=='__main__':unittest.main()
