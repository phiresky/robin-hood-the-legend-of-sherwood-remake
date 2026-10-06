"""Regression checks for links retained across review-gallery revisions."""
import hashlib
import errno
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('shared_gallery', Path(__file__).with_name('build_review_gallery.py'))
gallery = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gallery)


class StableGalleryLinks(unittest.TestCase):
    def test_archive_copy_fallback_and_existing_archive_integrity(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source, manifest, output = root / 'source.png', root / 'manifest.json', root / 'gallery'
            source.write_bytes(b'image')
            manifest.write_text(json.dumps(dict(items=[dict(id='asset', name='Asset',
                status='ready-for-user', solid=str(source), textured=str(source))])))
            gallery.build(manifest, output)
            evidence = (output / 'evidence.json').read_bytes()
            with patch.object(gallery.os, 'link', side_effect=OSError(errno.EXDEV, 'Other filesystem')):
                gallery.build(manifest, output)
            archive = output / 'history' / hashlib.sha256(evidence).hexdigest()[:16]
            entry = json.loads(evidence)['items'][0]['images']['solid']
            self.assertEqual(gallery._sha(archive / entry['file']), entry['sha256'])
            self.assertFalse(os.path.samefile(output / entry['file'], archive / entry['file']))
            self.assertFalse(list((output / 'history').glob('.archive-*')))
            (archive / entry['file']).unlink()
            with self.assertRaises(FileNotFoundError):
                gallery.build(manifest, output)
            self.assertEqual((output / 'evidence.json').read_bytes(), evidence)

    def test_history_keeps_only_referenced_resources_and_shares_immutable_bytes(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source, report, reference = (root / name for name in ('source.png', 'report.json', 'reference.png'))
            source.write_bytes(b'image one')
            report.write_text('{"version": 1}')
            reference.write_bytes(b'original artwork')
            manifest, output = root / 'manifest.json', root / 'gallery'
            item = dict(id='asset', name='Asset', status='ready-for-user', solid=str(source),
                        textured=str(source), validation=str(report), artwork_references=[dict(
                            id='original', label='Original', path=str(reference),
                            sha256=hashlib.sha256(reference.read_bytes()).hexdigest())])
            manifest.write_text(json.dumps(dict(items=[item])))

            def build():
                gallery.build(manifest, output)
                return (output / 'evidence.json').read_bytes(), (output / 'index.html').read_bytes()

            first, first_page = build()
            # Simulate accumulated legacy resources, including a large unreferenced report.
            (output / 'images/obsolete.png').write_bytes(b'obsolete')
            (output / 'reports/obsolete.glb').write_bytes(b'obsolete model export')
            build()
            archive = output / 'history' / hashlib.sha256(first).hexdigest()[:16]
            record = json.loads(first)['items'][0]
            expected = {entry['file'] for kind in ('images', 'reports', 'reference_images')
                        for entry in record[kind].values()}
            self.assertEqual({str(p.relative_to(archive)) for p in archive.rglob('*') if p.is_file()},
                             expected | {'index.html', 'evidence.json'})
            for relative in expected:
                self.assertTrue(os.path.samefile(output / relative, archive / relative))
            for _ in range(3):
                again, _ = build()
                self.assertEqual(again, first)
            self.assertEqual(len(list((output / 'history').iterdir())), 1)

            source.write_bytes(b'image two')
            report.write_text('{"version": 2}')
            second, second_page = build()
            self.assertEqual((archive / 'evidence.json').read_bytes(), first)
            self.assertEqual((archive / 'index.html').read_bytes(), first_page)
            self.assertNotEqual(first, second)
            build()
            second_archive = output / 'history' / hashlib.sha256(second).hexdigest()[:16]
            self.assertEqual((second_archive / 'index.html').read_bytes(), second_page)
            second_record = json.loads(second)['items'][0]
            current = {entry['file'] for kind in ('images', 'reports', 'reference_images')
                       for entry in second_record[kind].values()}
            self.assertEqual({str(p.relative_to(second_archive)) for p in second_archive.rglob('*') if p.is_file()},
                             current | {'index.html', 'evidence.json'})
            for relative in expected - current:
                self.assertTrue((output / relative).is_file(), 'Old live URLs must remain valid')
                self.assertFalse((second_archive / relative).exists())
            for kind in ('images', 'reports', 'reference_images'):
                for entry in record[kind].values():
                    self.assertEqual(gallery._sha(archive / entry['file']), entry['sha256'])

    def test_current_replacement_does_not_write_through_existing_hardlinks(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source, current, frozen = (root / name for name in ('source', 'current', 'frozen'))
            source.write_bytes(b'new bytes')
            current.write_bytes(b'old bytes')
            os.link(current, frozen)
            gallery._atomic_copy(source, current, gallery._sha(source))
            self.assertEqual(frozen.read_bytes(), b'old bytes')
            self.assertEqual(current.read_bytes(), b'new bytes')
            self.assertFalse(os.path.samefile(current, frozen))
            inode = current.stat().st_ino
            gallery._atomic_copy(source, current, gallery._sha(source))
            self.assertEqual(current.stat().st_ino, inode, 'Verified identical resources are not rewritten')
            os.unlink(frozen)
            os.link(current, frozen)
            gallery._atomic_text(current, 'new document')
            self.assertEqual(frozen.read_bytes(), b'new bytes')
            self.assertEqual(current.read_text(), 'new document')

    def test_missing_or_changed_frozen_resource_fails_before_rebuilding(self):
        for kind in ('images', 'reports', 'reference_images'):
            for changed in (False, True):
                with self.subTest(kind=kind, changed=changed), tempfile.TemporaryDirectory() as temporary:
                    root = Path(temporary)
                    source = root / 'source.png'
                    source.write_bytes(b'original')
                    manifest, output = root / 'manifest.json', root / 'gallery'
                    manifest.write_text(json.dumps(dict(items=[dict(id='asset', name='Asset',
                        status='ready-for-user', solid=str(source), textured=str(source),
                        validation=str(source), artwork_references=[dict(id='original', label='Original',
                            path=str(source), sha256=gallery._sha(source))])])))
                    gallery.build(manifest, output)
                    evidence = (output / 'evidence.json').read_bytes()
                    page = (output / 'index.html').read_bytes()
                    entry = next(iter(json.loads(evidence)['items'][0][kind].values()))
                    resource = output / entry['file']
                    if changed:
                        resource.write_bytes(b'corrupted')
                    else:
                        resource.unlink()
                    with self.assertRaises(ValueError if changed else FileNotFoundError):
                        gallery.build(manifest, output)
                    self.assertEqual((output / 'evidence.json').read_bytes(), evidence)
                    self.assertEqual((output / 'index.html').read_bytes(), page)
                    self.assertFalse((output / 'history').exists())

    def test_grouping_review_has_scoped_buttons_and_stable_revisions(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);source=root/'source.png';source.write_bytes(b'image')
            model=root/'model.glb';model.write_bytes(b'geometry')
            manifest=root/'manifest.json';output=root/'gallery'
            item={'id':'wall','name':'Courtyard wall','status':'ready-for-user',
                  'solid':str(source),'east_solid':str(source),'context':str(source),'model':str(model)}
            manifest.write_text(json.dumps({'map':'York','review_kind':'grouping','items':[item]}))
            gallery.build(manifest,output,pending_only=True)
            page=(output/'index.html').read_text()
            self.assertIn('Approve grouping</button>',page)
            self.assertIn('Request changes</button>',page)
            self.assertIn('does not approve geometry completion',page)
            self.assertIn('id="asset-search"',page)
            self.assertNotIn('Gray means no accepted original texture',page)
            first=json.loads((output/'evidence.json').read_text())['items'][0]
            self.assertEqual(set(first['images']),{'solid','east_solid','context'})
            gallery.build(manifest,output,pending_only=True)
            second=json.loads((output/'evidence.json').read_text())['items'][0]
            self.assertEqual(first['review_revision'],second['review_revision'])
            model.write_bytes(b'changed geometry')
            gallery.build(manifest,output,pending_only=True)
            third=json.loads((output/'evidence.json').read_text())['items'][0]
            self.assertNotEqual(first['review_revision'],third['review_revision'])

    def test_removal_and_new_image_keep_previous_links_valid(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / 'source.png'
            source.write_bytes(b'original image fixture')
            manifest = root / 'manifest.json'
            output = root / 'gallery'
            items = [{'id': name, 'name': name, 'status': 'ready-for-user',
                      'solid': str(source), 'textured': str(source)} for name in ('first', 'second')]

            def build():
                manifest.write_text(json.dumps({'map': 'Test map', 'items': items}))
                gallery.build(manifest, output, pending_only=True)
                return json.loads((output / 'evidence.json').read_text())['items']

            first_records = build()
            retained = first_records[1]['images']['solid']
            items.pop(0)
            remaining = build()[0]
            self.assertEqual(remaining['review_revision'], first_records[1]['review_revision'])
            self.assertEqual(remaining['images']['solid']['file'], retained['file'])
            self.assertIn('id="second"', (output / 'index.html').read_text())
            self.assertIn('href="#second"', (output / 'index.html').read_text())
            source.write_bytes(b'revised image fixture')
            revised_record = build()[0]
            self.assertNotEqual(revised_record['review_revision'], remaining['review_revision'])
            revised = revised_record['images']['solid']
            self.assertNotEqual(revised['file'], retained['file'])
            for record in (retained, revised):
                self.assertEqual(hashlib.sha256((output / record['file']).read_bytes()).hexdigest(),
                                 record['sha256'])
            self.assertTrue(list((output / 'history').glob('*/index.html')))

    def test_texture_review_labels_do_not_change_geometry_default(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            manifest, output = root / 'manifest.json', root / 'gallery'
            for kind in (None, 'texture'):
                output = root / ('gallery-' + str(kind))
                data = {'map': 'Test', 'items': []}
                if kind:
                    data['review_kind'] = kind
                manifest.write_text(json.dumps(data))
                gallery.build(manifest, output)
                page = (output / 'index.html').read_text()
                if kind:
                    self.assertIn('Generated textures baked onto the approved geometry', page)
                    self.assertIn('Baked textures</option>', page)
                    self.assertNotIn('Geometry candidates, not generated textures', page)
                else:
                    self.assertIn('Geometry candidates, not generated textures', page)
                    self.assertIn('Original textures + gray</option>', page)

    def test_model_revision_and_blocked_approval(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / 'source.png'
            source.write_bytes(b'image')
            model = root / 'model.blend'
            model.write_bytes(b'geometry one')
            manifest = root / 'manifest.json'
            manifest.write_text(json.dumps({'items': [{'id': 'asset', 'name': 'Asset',
                'status': 'fix-needed', 'model': str(model), 'solid': str(source), 'textured': str(source)}]}))
            output = root / 'gallery'
            gallery.build(manifest, output)
            first = json.loads((output / 'evidence.json').read_text())['items'][0]['review_revision']
            self.assertIn('<option value="approved" disabled>', (output / 'index.html').read_text())
            model.write_bytes(b'geometry two')
            gallery.build(manifest, output)
            second = json.loads((output / 'evidence.json').read_text())['items'][0]['review_revision']
            self.assertNotEqual(first, second)

    def test_animation_views_share_one_decision_and_bind_revision(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source, state = root / 'source.png', root / 'state.png'
            source.write_bytes(b'base image')
            state.write_bytes(b'initial state')
            manifest = root / 'manifest.json'
            manifest.write_text(json.dumps({'items': [{'id': 'gate', 'name': 'Gate',
                'status': 'ready-for-user', 'solid': str(source), 'textured': str(source),
                'animation_reviews': [{'id': 'initial', 'name': 'Initial', 'solid': str(state),
                    'textured': str(state), 'context': str(source)}]}]}))
            output = root / 'gallery'
            gallery.build(manifest, output)
            page = (output / 'index.html').read_text()
            self.assertEqual(page.count('<article '), 1)
            self.assertEqual(page.count('class="decision"'), 1)
            self.assertIn('<details class="animation-state">', page)
            first = json.loads((output / 'evidence.json').read_text())['items'][0]
            self.assertIn('animation_initial_textured', first['images'])
            state.write_bytes(b'changed state')
            gallery.build(manifest, output)
            second = json.loads((output / 'evidence.json').read_text())['items'][0]
            self.assertNotEqual(first['review_revision'], second['review_revision'])


if __name__ == '__main__':
    unittest.main()
