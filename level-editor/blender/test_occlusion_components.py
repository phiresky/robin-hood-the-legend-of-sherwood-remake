"""Component selection must not apply a machinery mask to its entire building."""
import json
import tempfile
import unittest
from pathlib import Path

import numpy as np
from occlusion_constraints import SourceMaskConstraints


class ComponentConstraintsTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.bitmaps = {
            1: np.ones((2, 4), dtype=bool),
            2: np.array([[1, 1, 0, 0]] * 2, dtype=bool),
            3: np.array([[0, 0, 1, 1]] * 2, dtype=bool),
            4: np.array([[0, 0, 1, 0]] * 2, dtype=bool),
        }
        (self.root / 'inventory.json').write_text(json.dumps({'masks': [
            {'index': index, 'box_top_left': [0, 0], 'box_size': [4, 2],
             'png': f'{index}.png'} for index in self.bitmaps]}))

    def tearDown(self):
        self.temp.cleanup()

    def load(self, assignments, label='exterior'):
        path = self.root / 'constraints.json'
        path.write_text(json.dumps({'version': 1, 'mask_inventory': 'inventory.json',
                                   'projections': {'exterior': {
                                       'source_sha256': 'a' * 64, 'state': 'covered',
                                       'assignments': assignments}}}))
        return SourceMaskConstraints(path, label, 'a' * 64, (4, 2),
                                     image_loader=lambda p: self.bitmaps[int(p.stem)])

    def test_component_precedence_and_matching_scalar_vector_results(self):
        assignments = [
            {'reviewed': True, 'asset_group': 'tower', 'mask_indices': [1]},
            {'reviewed': True, 'source_node': 'building-215', 'mask_indices': [2]},
            {'reviewed': True, 'source_node': 'building-215',
             'projection_component': 'hoist', 'mask_indices': [3],
             'exclude_mask_indices': [4], 'exclusions_reviewed': True,
             'exclusion_reason': 'Reviewed foreground fixture'},
        ]
        constraints = self.load(assignments)
        cases = [
            ({'asset_group': 'tower', 'source_node': 'building-215',
              'projection_component': 'hoist'}, [False, False, False, True]),
            ({'asset_group': 'tower', 'source_node': 'building-215',
              'projection_component': 'parapet'}, [True, True, False, False]),
            ({'asset_group': 'tower', 'source_node': 'building-214',
              'projection_component': 'hoist'}, [True] * 4),
            ({'source_node': 'unassigned'}, [True] * 4),
        ]
        for obj, expected in cases:
            with self.subTest(obj=obj):
                self.assertEqual([constraints.allowed_pixel(obj, x, 0)
                                  for x in range(4)], expected)
                self.assertEqual(constraints.allowed(constraints.for_object(obj),
                                                     np.arange(4), np.ones(4, dtype=int)).tolist(), expected)
        self.assertIsNone(self.load(assignments, 'another-layer').for_object(cases[0][0]))

    def test_invalid_and_duplicate_component_assignments_fail(self):
        base = {'reviewed': True, 'source_node': 'building-215',
                'projection_component': 'hoist', 'mask_indices': [3]}
        for invalid in ({**base, 'projection_component': ''},
                        {**base, 'projection_component': 5},
                        {'reviewed': True, 'asset_group': 'tower',
                         'projection_component': 'hoist', 'mask_indices': [3]}):
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                self.load([invalid])
        with self.assertRaisesRegex(ValueError, 'Duplicate'):
            self.load([base, base])
        self.load([base, {**base, 'projection_component': 'parapet'}])


if __name__ == '__main__':
    unittest.main()
