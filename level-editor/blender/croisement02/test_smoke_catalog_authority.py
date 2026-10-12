import unittest,tempfile,json,hashlib
from pathlib import Path
from audit_smoke_catalog_authority import audit_catalog

def catalog(rows):return json.dumps({'version':1,'assets':rows},separators=(',',':')).encode()
class AuthorityTests(unittest.TestCase):
 def setUp(self):
  self.tmp=tempfile.TemporaryDirectory();self.root=Path(self.tmp.name)
  self.owned={'id':'other-id','name':'x'*700,'source_map':'cRoIsEmEnT02','editor':{'value':0}}
  self.other={'id':'tree','name':'Tree','source_map':'York','descriptor':'asset.json','descriptor_sha256':'old','editor':{'id':'tree','source_map':'York','gameplay':{'x':0}}}
 def tearDown(self):self.tmp.cleanup()
 def changed(self):
  row=json.loads(json.dumps(self.other));row['editor']['gameplay']['x']=1;raw=json.dumps(row['editor']).encode();(self.root/'asset.json').write_bytes(raw);row['descriptor_sha256']=hashlib.sha256(raw).hexdigest();return row
 def test_descriptor_backed_unrelated_update(self):
  new=self.changed();r=audit_catalog(catalog([self.owned,self.other]),catalog([self.owned,new]),self.root);self.assertEqual(r['changes'][0]['id'],'tree');self.assertEqual(r['owned_rows'],['other-id'])
 def test_complete_casefold_source_ownership(self):
  altered=dict(self.owned,editor={'value':1})
  with self.assertRaisesRegex(ValueError,'Owned C02'):audit_catalog(catalog([self.owned]),catalog([altered]),self.root)
 def test_new_affiliation_protected(self):
  changed=dict(self.other,source_map='Croisement02')
  with self.assertRaisesRegex(ValueError,'Owned C02'):audit_catalog(catalog([self.other]),catalog([changed]),self.root)
 def test_descriptor_bytes_drift(self):
  new=self.changed();(self.root/'asset.json').write_text('{}')
  with self.assertRaisesRegex(ValueError,'Unbacked'):audit_catalog(catalog([self.other]),catalog([new]),self.root)
 def test_model_change_rejected(self):
  new=self.changed();new['model']='different.glb'
  with self.assertRaisesRegex(ValueError,'Unsupported'):audit_catalog(catalog([self.other]),catalog([new]),self.root)
if __name__=='__main__':unittest.main()
