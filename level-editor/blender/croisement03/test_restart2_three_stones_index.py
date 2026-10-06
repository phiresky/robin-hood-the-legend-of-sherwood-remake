"""Verify a multi-group replacement cannot alter unrelated cached editor data."""
import copy,hashlib,tempfile,unittest
from pathlib import Path
from restart2_three_stones_index import merge_scope,check_prior
class ScopedIndexTests(unittest.TestCase):
 def test_multiple_retirements_preserve_unrelated_metadata(self):
  old={'version':1,'assets':[{'id':'keep','descriptor':'keep.json','editor':{'legacy':True}},{'id':'old-a'},{'id':'old-b'}]};fresh={'assets':[{'id':'keep','descriptor':'keep.json','editor':{}},{'id':'new','descriptor':'new.json'}]};merged=merge_scope(old,fresh,{'new'},{'old-a','old-b'});self.assertEqual(merged['assets'][0],old['assets'][0]);self.assertEqual([x['id'] for x in merged['assets']],['keep','new'])
  bad=copy.deepcopy(fresh);bad['assets'][0]['descriptor']='other.json'
  with self.assertRaises(AssertionError):merge_scope(old,bad,{'new'},{'old-a','old-b'})
  bad=copy.deepcopy(fresh);bad['assets'].append({'id':'old-a'})
  with self.assertRaises(AssertionError):merge_scope(old,bad,{'new'},{'old-a','old-b'})
 def test_stale_preflight_fails(self):
  with tempfile.TemporaryDirectory() as t:
   p=Path(t)/'index.json';p.write_bytes(b'old');digest=hashlib.sha256(p.read_bytes()).hexdigest();check_prior(p,digest);p.write_bytes(b'new')
   with self.assertRaises(AssertionError):check_prior(p,digest)
if __name__=='__main__':unittest.main()
