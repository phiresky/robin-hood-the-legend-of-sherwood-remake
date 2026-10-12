import copy,importlib.util,json,tempfile,unittest
from pathlib import Path
p=Path(__file__).with_name('transaction.py');s=importlib.util.spec_from_file_location('candidate',p);m=importlib.util.module_from_spec(s);s.loader.exec_module(m)
class Preservation(unittest.TestCase):
 def setUp(self):
  self.a=json.loads((m.L/'scenes/croisement01.rhlos-map.json').read_text());self.b=json.loads((m.S/'croisement01.rhlos-map.json').read_text())
 def test_exact_stage(self):self.assertEqual(m.preserve(self.a,self.b)['added_placements'],1)
 def test_placement_drift(self):
  self.b['placements'][0]['name']='unrelated mutation'
  with self.assertRaises(RuntimeError):m.preserve(self.a,self.b)
 def test_metadata_drift(self):
  self.b['sceneMetadata']['unexpected']=True
  with self.assertRaises(RuntimeError):m.preserve(self.a,self.b)
 def test_unrelated_ref_drift(self):
  next(x for x in self.b['assetSources'] if x['id'] not in m.IDS)['model_sha256']='wrong'
  with self.assertRaises(RuntimeError):m.preserve(self.a,self.b)
class Recovery(unittest.TestCase):
 def test_failure_rolls_back_existing_and_new_payload(self):
  with tempfile.TemporaryDirectory(dir=m.D) as tmp:
   d=Path(tmp);a=d/'existing';b=d/'new';x=d/'candidate-a';y=d/'candidate-b';a.write_text('old');x.write_text('new-a');y.write_text('new-b')
   rows=[dict(target=str(a),source=str(x),old_sha256=m.core.sha(a),new_sha256=m.core.sha(x)),dict(target=str(b),source=str(y),old_sha256=None,new_sha256=m.core.sha(y))]
   with self.assertRaises(RuntimeError):m.core.commit(rows,d/'journal',lambda:None,fail_after=2)
   self.assertEqual(a.read_text(),'old');self.assertFalse(b.exists())
 def test_rollback_refuses_external_drift(self):
  with tempfile.TemporaryDirectory(dir=m.D) as tmp:
   d=Path(tmp);a=d/'existing';x=d/'candidate';a.write_text('old');x.write_text('new')
   rows=[dict(target=str(a),source=str(x),old_sha256=m.core.sha(a),new_sha256=m.core.sha(x))]
   journal=m.core.commit(rows,d/'journal',lambda:None);a.write_text('external')
   with self.assertRaises(RuntimeError):m.core.restore(rows,d/'journal',journal)
   self.assertEqual(a.read_text(),'external')
if __name__=='__main__':unittest.main()
