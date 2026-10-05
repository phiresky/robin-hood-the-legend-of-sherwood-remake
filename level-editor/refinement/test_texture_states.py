import json
from pathlib import Path
import tempfile
import unittest

from PIL import Image

from build_texture_gallery import collect
from texture_decisions import bind, record, sha


class TextureStateTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.experiments = self.root / 'experiments'
        self.experiments.mkdir()
        self.output = self.root / 'review'
        self.primary = self.make_experiment('initial', 'ready-for-user')
        self.secondary = self.make_experiment('applied', 'supplemental')
        self.update(self.primary / 'texture-review.json', texture_states=[{
            'id': 'applied', 'name': 'Applied endpoint', 'experiment': str(self.secondary)}])

    def update(self, path, **fields):
        data = json.loads(path.read_text())
        data.update(fields)
        path.write_text(json.dumps(data))

    def make_experiment(self, name, status):
        experiment = self.experiments / name
        bake = experiment / 'bake'
        generation = experiment / 'generation'
        (bake / 'actual').mkdir(parents=True)
        generation.mkdir()
        for path in [experiment / 'solid.png', experiment / 'input.png', bake / 'actual/textured.png',
                     generation / 'generated-raw.png', generation / 'generated-preserved.png']:
            Image.new('RGBA', (16, 16), (80, 120, 160, 255)).save(path)
        (bake / 'worker.blend').write_bytes(name.encode())
        (experiment / 'approval.json').write_text(json.dumps({
            'asset_id': 'leicester-bridge', 'geometry_revision': 'geometry-pair',
            'input_sha256': sha(experiment / 'input.png')}))
        (bake / 'validation.json').write_text(json.dumps({
            'geometry_verified': True, 'generated_sha256': sha(generation / 'generated-preserved.png')}))
        (generation / 'generation.json').write_text(json.dumps({'changedProtected': 0}))
        (experiment / 'texture-review.json').write_text(json.dumps({
            'status': status, 'bake': 'bake', 'generation': 'generation',
            'all_eight_actual_views_inspected': True,
            'actual_sheet_sha256': sha(bake / 'actual/textured.png'),
            'baked_model_sha256': sha(bake / 'worker.blend')}))
        return experiment

    def build(self):
        result = collect(self.experiments, self.output, 'Leicester')
        displayed = json.loads((self.output / 'gallery/evidence.json').read_text())['items']
        return result, displayed

    def test_single_card_binds_both_endpoints_and_archives_both_models(self):
        result, items = self.build()
        self.assertEqual(result['candidates'], 1)
        self.assertEqual(len(items), 1)
        item = items[0]
        self.assertEqual(len(item['images']), 10)
        self.assertEqual(len(item['reports']), 4)
        decisions = self.output / 'decisions.json'
        feedback = 'leicester-bridge: approved [review ' + item['review_revision'][:16] + ']'
        record(self.output / 'gallery', decisions, feedback)
        decision = json.loads(decisions.read_text())['decisions'][0]
        self.assertEqual((Path(decision['archive']) / 'texture_state_applied_model.blend').read_bytes(), b'applied')
        item['user_approval'] = 'pending'
        bind(item, [decision])
        self.assertEqual(item['user_approval'], 'approved')
        (self.secondary / 'bake/worker.blend').write_bytes(b'changed applied model')
        with self.assertRaisesRegex(ValueError, 'state evidence changed'):
            bind(item, [decision])

    def test_cached_appearance_labels_do_not_change_evidence_fields(self):
        from build_texture_gallery import candidate
        from texture_decisions import IMAGE_FIELDS
        original, _, _ = candidate(self.primary, 'Leicester')
        self.update(self.primary / 'generation/generation.json', provider='cached-appearance-reuse')
        reused, _, _ = candidate(self.primary, 'Leicester')
        self.assertIn('Raw Sunburst output', original['source_trace_label'])
        self.assertIn('no new generation', reused['source_trace_label'])
        self.assertIn('transferred', reused['textured_label'])
        for field in IMAGE_FIELDS:
            self.assertEqual(original[field], reused[field])
        self.assertEqual(original['validation'], reused['validation'])
        self.update(self.secondary / 'generation/generation.json', provider='cached-appearance-reuse')
        _, items = self.build()
        markup = (self.output / 'gallery/index.html').read_text()
        self.assertIn('Applied endpoint: Cached donor appearance', markup)


    def test_other_pair_revision_rejected(self):
        self.update(self.secondary / 'approval.json', geometry_revision='different-pair')
        with self.assertRaisesRegex(ValueError, 'identity/revision'):
            self.build()

    def test_duplicate_asset_does_not_silently_replace_card(self):
        self.make_experiment('duplicate', 'ready-for-user')
        with self.assertRaisesRegex(ValueError, 'Multiple ready texture candidates'):
            self.build()

    def replacement_record(self, replacement):
        old = self.primary / 'texture-review.json'
        new = replacement / 'texture-review.json'
        return dict(asset_id='leicester-bridge', review=str(old), review_sha256=sha(old),
                    replacement=str(new), replacement_sha256=sha(new))

    def test_explicit_supersession_keeps_approval_archive_and_new_revision_pending(self):
        _, items = self.build()
        decisions = self.output / 'decisions.json'
        record(self.output / 'gallery', decisions,
               'leicester-bridge: approved [review ' + items[0]['review_revision'][:16] + ']')
        original_decisions = decisions.read_bytes()
        original_review = (self.primary / 'texture-review.json').read_bytes()
        replacement = self.make_experiment('replacement', 'ready-for-user')
        result = collect(self.experiments, self.output, 'Leicester',
                         supersessions=[self.replacement_record(replacement)])
        self.assertEqual(result['candidates'], 1)
        self.assertEqual(result['approved'], 0)
        self.assertEqual(decisions.read_bytes(), original_decisions)
        self.assertEqual((self.primary / 'texture-review.json').read_bytes(), original_review)
        decision = json.loads(decisions.read_text())['decisions'][0]
        self.assertEqual((Path(decision['archive']) / 'model.blend').read_bytes(), b'initial')

    def test_supersession_rejects_changed_review(self):
        replacement = self.make_experiment('replacement', 'ready-for-user')
        replacement_record = self.replacement_record(replacement)
        self.update(replacement / 'texture-review.json', notes=['changed'])
        with self.assertRaisesRegex(ValueError, 'supersession evidence changed'):
            collect(self.experiments, self.output, 'Leicester', supersessions=[replacement_record])

    def test_supersession_rejects_different_asset(self):
        replacement = self.make_experiment('replacement', 'ready-for-user')
        self.update(replacement / 'approval.json', asset_id='different-object')
        with self.assertRaisesRegex(ValueError, 'preserve asset identity'):
            collect(self.experiments, self.output, 'Leicester',
                    supersessions=[self.replacement_record(replacement)])

    def test_pending_bake_does_not_block_completed_candidates(self):
        pending = self.experiments / 'pending'
        pending.mkdir()
        (pending / 'texture-review.json').write_text(json.dumps({'status': 'bake-pending'}))
        result, items = self.build()
        self.assertEqual(result['candidates'], 1)

    def test_changed_displayed_secondary_image_rejected(self):
        _, items = self.build()
        item = items[0]
        copied = self.output / 'gallery' / item['images']['texture_state_applied_textured']['file']
        copied.write_bytes(b'wrong screenshot')
        with self.assertRaisesRegex(ValueError, 'Displayed texture evidence changed'):
            record(self.output / 'gallery', self.output / 'decisions.json',
                   'leicester-bridge: approved [review ' + item['review_revision'][:16] + ']')

    def test_material_state_requires_preservation_and_matching_bake(self):
        qa = self.primary / 'revealed-qa.json'
        qa.write_text(json.dumps({'materials_preserved': True,
                                 'baked_model_sha256': sha(self.primary / 'bake/worker.blend')}))
        state = {'id': 'revealed', 'name': 'Revealed interior',
                 'textured': str(self.primary / 'bake/actual/textured.png'), 'validation': str(qa),
                 'actual_sheet_sha256': sha(self.primary / 'bake/actual/textured.png'),
                 'validation_sha256': sha(qa)}
        self.update(self.primary / 'texture-review.json', material_states=[state])
        _, items = self.build()
        self.assertIn('texture_state_revealed_textured', items[0]['images'])
        self.update(qa, baked_model_sha256='wrong-bake')
        state['validation_sha256'] = sha(qa)
        self.update(self.primary / 'texture-review.json', material_states=[state])
        with self.assertRaisesRegex(ValueError, 'bind baked model'):
            self.build()


if __name__ == '__main__':
    unittest.main()
