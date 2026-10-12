import json,tempfile,unittest
from pathlib import Path
import rebase_round_wood_index as m
class Guards(unittest.TestCase):
 def catalog(self,extra=1):return json.dumps({'version':1,'assets':[{'id':i,'name':i,'source_map':'Croisement02'if i.startswith('croisement02')else'York','metadata':extra if i=='york-house'else 0}for i in ['croisement02-tree-32','croisement02-tree-38','york-house']]},separators=(',',':')).encode()
 def test_unrelated_current_rows_preserved(self):
  reviewed=m.candidate(self.catalog());current=self.catalog(2);new=m.candidate(current);self.assertEqual(m.guard_scope(reviewed,current,new),['york-house'])
  bad=new.replace(b'"metadata":2',b'"metadata":3')
  with self.assertRaisesRegex(ValueError,'Unrelated current row'):m.guard_scope(reviewed,current,bad)
 def test_c02_mutation_rejected(self):
  old=self.catalog();reviewed=m.candidate(old);changed=old.replace(b'"metadata":0',b'"metadata":7',1)
  with self.assertRaisesRegex(ValueError,'Reviewed C02 row'):m.guard_scope(reviewed,changed,m.candidate(changed))
 def test_drift_between_plan_and_apply(self):
  with tempfile.TemporaryDirectory()as d:
   p=Path(d)/'index.json';p.write_bytes(b'before');records=[{'target':str(p),'old_sha256':m.sha(p)}];m.guard_records(records);p.write_bytes(b'concurrent')
   with self.assertRaisesRegex(ValueError,'Target drift'):m.guard_records(records)
if __name__=='__main__':unittest.main()
