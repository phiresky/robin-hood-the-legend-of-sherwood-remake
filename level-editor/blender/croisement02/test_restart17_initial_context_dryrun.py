"""Regression guards for the proposed eight-reference update, with no live writes."""
import copy,json,tempfile,unittest
from pathlib import Path
from restart17_initial_context_dryrun import validate_contract_delta,validate_index_delta,validate_candidate_record,CANDIDATE,ROOT,safe

class ProposalGuards(unittest.TestCase):
    def fixture(self):
        before={'version':1,'entries':[{'id':f'entry{i}','mission':'fixture','contract':{'path':f'old/{i}.json','sha256':str(i)},'approved_geometry':'unchanged'}for i in range(41)]}
        records=[{'id':f'entry{i}','baseline_path':f'old/{i}.json','baseline_sha256':str(i),'destination':f'new/{i}.json','sha256':f'new{i}'}for i in range(8)]
        after=copy.deepcopy(before)
        for i in range(8):after['entries'][i]['contract']={'path':f'new/{i}.json','sha256':f'new{i}'}
        return before,after,records
    def test_exact_eight_references_preserve_forty_one(self):
        validate_index_delta(*self.fixture())
    def test_unrelated_entry_and_asset_metadata_changes_rejected(self):
        for index in [0,40]:
            before,after,records=self.fixture();after['entries'][index]['approved_geometry']='changed'
            with self.assertRaisesRegex(ValueError,'outside'):validate_index_delta(before,after,records)
    def test_extra_missing_and_duplicate_entry_rejected(self):
        for mutation in ['extra','missing','duplicate']:
            before,after,records=self.fixture()
            if mutation=='extra':after['entries'].append(copy.deepcopy(after['entries'][0]))
            elif mutation=='missing':after['entries'].pop()
            else:after['entries'][40]=copy.deepcopy(after['entries'][39])
            with self.assertRaises(ValueError):validate_index_delta(before,after,records)
    def test_wrong_old_reference_rejected(self):
        before,after,records=self.fixture();records[0]['baseline_sha256']='stale'
        with self.assertRaisesRegex(ValueError,'baseline'):validate_index_delta(before,after,records)
    def contract(self):
        before={'native':{'elements':[]},'families':[{'id':'old','physical':{'initial':['approved-model']}}]}
        after=copy.deepcopy(before);after['native']['patch_states']=[{'id':'context','initial':['source']}]
        return before,after
    def test_context_addition_only(self):
        before,after=self.contract();validate_contract_delta(before,after,['context'])
    def test_changed_physical_binding_rejected(self):
        before,after=self.contract();after['families'][0]['physical']['initial']=['other-model']
        with self.assertRaisesRegex(ValueError,'physical'):validate_contract_delta(before,after,['context'])
    def test_missing_or_duplicate_context_rejected(self):
        before,after=self.contract()
        with self.assertRaises(ValueError):validate_contract_delta(before,after,['missing'])
        with self.assertRaises(ValueError):validate_contract_delta(before,after,['context','context'])
    def test_reviewed_candidate_cannot_be_replaced_by_rehashing_proposal(self):
        reviewed={'id':'entry0','mission':'fixture','path':'contract.json','sha256':'exact','baseline_sha256':'old','added_context_ids':['context']}
        row={'id':'entry0','mission':'fixture','source':str((CANDIDATE/'library/contract.json').relative_to(ROOT)),'destination':'mission-states/croisement02/contracts/initial-context/exact.json','sha256':'exact','baseline_path':'contract.json','baseline_sha256':'old','added_context_ids':['context']}
        validate_candidate_record(row,reviewed)
        for field,value in [('sha256','replacement'),('destination','another.json'),('added_context_ids',['different']),('source','other.json')]:
            changed=copy.deepcopy(row);changed[field]=value
            with self.assertRaises(ValueError):validate_candidate_record(changed,reviewed)
    def test_escaping_paths_and_symlinks_rejected(self):
        with tempfile.TemporaryDirectory(prefix='initial-context-guard-')as directory:
            base=Path(directory);(base/'inside').mkdir();(base/'link').symlink_to(base/'inside',target_is_directory=True)
            for path in ['../escape','/tmp/escape','link/asset.json']:
                with self.assertRaises(ValueError):safe(base,path)
            self.assertEqual(safe(base,'inside/asset.json'),base/'inside/asset.json')
if __name__=='__main__':unittest.main()
