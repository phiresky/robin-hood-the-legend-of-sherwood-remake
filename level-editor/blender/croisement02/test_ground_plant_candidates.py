"""Exercise failures that could expose stale or unsupported new geometry."""
import json
import tempfile
import unittest
from pathlib import Path
from ground_plant_candidates import validate_record,sha

class SelectionGuards(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.addCleanup(self.temp.cleanup)
        self.worker=Path(self.temp.name);(self.worker/'inspection/source-coverage').mkdir(parents=True)
        (self.worker/'model.blend').write_bytes(b'unit-test model binding')
        self.model=sha(self.worker/'model.blend');self.group={'id':'croisement02-ground-plant-111','native_ground_plant_mask':111,'parts':[{'node':'foliage-ground-plant-111'}]}
        self.write('saved-model-audit.json',{'status':'PASS','model_sha256':self.model,'objects':[{'source_node':'foliage-ground-plant-111'}]})
        self.write('visual-review.json',{'model_sha256':self.model,'ready_for_geometry_review':True})
        self.write('source-coverage/report.json',{'model_sha256':self.model,'intersection_over_union':.99})
        self.geometry={'geometry_version':'native-rooted-ground-plants-v4','minimum_z':.05,'ground_z':0}
        self.write('refinement.json',{'crown':self.geometry})
        self.record={'group':self.group,'native_mask':111,'domain':440,'user_approval':None,'workspace':str(self.worker),'model_sha256':self.model,'files':{str(self.worker/'model.blend'):self.model}}
    def write(self,name,value):
        (self.worker/'inspection'/name).write_text(json.dumps(value))
    def test_current_pending_candidate(self):self.assertEqual(validate_record(self.record,self.group),self.worker)
    def test_changed_model_rejected(self):
        (self.worker/'model.blend').write_bytes(b'changed')
        with self.assertRaises(ValueError):validate_record(self.record,self.group)
    def test_inherited_approval_rejected(self):
        self.record['user_approval']='approved'
        with self.assertRaises(ValueError):validate_record(self.record,self.group)
    def test_buried_root_rejected(self):
        self.geometry['minimum_z']=-1;self.write('refinement.json',{'crown':self.geometry})
        with self.assertRaises(ValueError):validate_record(self.record,self.group)
    def test_scope_reassignment_rejected(self):
        self.record['domain']=441
        with self.assertRaises(ValueError):validate_record(self.record,self.group)
if __name__=='__main__':unittest.main()
