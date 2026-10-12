"""Actual-bytecode and actual-artwork boundary checks for private coupling."""
import copy,unittest
import bindings as b
from preview import sample_patch
from verify import literal_patch_reads

class CouplingTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):cls.data=b.read(b.OUT/'bindings-v2.json')
    def test_all_arrow_callbacks_run_once_from_actual_bytecode(self):
        count=0
        for row in self.data['rows']:
            if not row['trigger']:continue
            script=b.read(b.ROOT/row['source_pins']['bytecode']['path']);c=next(c for c in script['classes'] if c['class_name']==row['trigger']['callback_class'])
            events,member=b.trace_callback(c);self.assertEqual(events,row['trigger']['events']);self.assertEqual(member,1)
            self.assertEqual(b.trace_callback(c,True),([],1));count+=1
        self.assertEqual(count,15)
    def test_native_argument_order_preserves_actor_and_location_identity(self):
        row=next(r for r in self.data['rows'] if r['mission']=='Tac19_FoB_EC' and r['patch_index']==4)
        moves=[e['args'] for e in row['trigger']['events'] if e['native']=='SetActorLocation']
        self.assertEqual(moves,[[{'kind':'actor','index':52},{'kind':'location','index':9}],[{'kind':'actor','index':53},{'kind':'location','index':10}]])
    def test_source_mismatched_move_and_unlock_handles_are_retained(self):
        row=next(r for r in self.data['rows'] if r['mission']=='Emb07_FoB_JMS' and r['patch_index']==7)
        self.assertEqual([m['actor']['handle'] for m in row['trigger']['actor_moves']],[42,43,44])
        self.assertEqual([m['handle'] for m in row['trigger']['ai_unlock_actors']],[45,46,47])
    def test_unsupported_callback_opcode_fails_closed(self):
        row=next(r for r in self.data['rows'] if r['trigger']);script=b.read(b.ROOT/row['source_pins']['bytecode']['path']);c=copy.deepcopy(next(c for c in script['classes'] if c['class_name']==row['trigger']['callback_class']))
        address=next(f for f in c['functions'] if f['name']=='ActivatedByArrow')['address'];c['quads'][address]['operation']=255
        with self.assertRaises(ValueError):b.trace_callback(c)
    def test_hidden_archer_terminal_background_and_reset(self):
        row=self.data['rows'][0];self.assertEqual(sample_patch(row,'apply',0)['draw']['counter'],65535)
        self.assertEqual(sample_patch(row,'apply',5)['phase'],'transition')
        terminal=sample_patch(row,'apply',6);self.assertEqual(terminal['phase'],'applied');self.assertFalse(terminal['fx_active']);self.assertEqual(terminal['background_frame']['index'],2)
        self.assertEqual(sample_patch(row,'reset')['background'],'baseline-restored');self.assertFalse(sample_patch(row,'reset')['reset_callback_latch'])
    def test_net_has_pc_crossing_not_invented_capture_callback(self):
        row=next(r for r in self.data['rows'] if r['mission']=='Tac19_FoB_EC' and r['patch_index']==0)
        self.assertIsNone(row['trigger']);self.assertEqual(row['automatic_trigger']['kind'],'pc-enters-apply-sector')
        self.assertEqual(row['automatic_trigger']['polygon'],[[1364,1088],[1357,1087],[1362,1083]])
        self.assertEqual(sample_patch(row,'apply',92)['phase'],'transition');final=sample_patch(row,'apply',93)
        self.assertTrue(final['fx_active']);self.assertIsNone(final['background_frame'])
    def test_dormant_sources_have_no_literal_bytecode_references(self):
        rows=[r for r in self.data['rows'] if r['trigger_qualification']=='NO_AUTHORED_LITERAL_TRIGGER_OR_TARGET_ASSOCIATION_FOUND'];self.assertEqual(len(rows),2)
        for row in rows:
            reads=literal_patch_reads(b.read(b.ROOT/row['source_pins']['bytecode']['path']))
            self.assertFalse(any(r['literal']==row['runtime_patch_index'] for r in reads));self.assertEqual(row['patch']['apply_sector']['points'],[])
    def test_patch_fx_origin_is_top_left_not_actor_center(self):
        frame=self.data['rows'][0]['art']['states']['initial']['frames'][0]
        self.assertEqual(frame['top_left'],[115,115])

if __name__=='__main__':unittest.main()
