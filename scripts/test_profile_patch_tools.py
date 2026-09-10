#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = ["jsonpatch>=1.33,<2"]
# ///
import unittest

import jsonpatch

from profile_patch_tools import ARCHETYPE, CAPACITIES, resolve_soldier, soldier_copy_patch


class ProfilePatchTests(unittest.TestCase):
    def setUp(self):
        previous = {field: 0 for field in ARCHETYPE}
        previous.update({field: 90 for field in CAPACITIES})
        previous.update(filename="Guard03", life_point=135, display_name="Red", hostile=True)
        current = {**previous, "filename": "Guard04", "life_point": 145, "fighting": 100}
        self.catalog = {"soldiers": {"Guard03": previous, "Guard04": current}}

    def test_generated_formula_and_cloning_preserve_sources(self):
        operations = soldier_copy_patch(
            self.catalog, "guard04", "Purple/~Guard", "Purple", hostile=False,
            progression_from="guard03",
        )
        result = jsonpatch.apply_patch(self.catalog, operations)
        added = result["soldiers"]["Purple/~Guard"]
        self.assertEqual(added["life_point"], 155)
        self.assertEqual(added["fighting"], 100)
        self.assertFalse(added["hostile"])
        self.assertNotIn("Purple/~Guard", self.catalog["soldiers"])
        self.assertEqual(result["soldiers"]["Guard04"], self.catalog["soldiers"]["Guard04"])
        self.catalog["soldiers"]["Guard04"]["life_point"] = 200
        with self.assertRaises(jsonpatch.JsonPatchTestFailed):
            jsonpatch.apply_patch(self.catalog, operations)

    def test_duplicate_identity_is_explicit(self):
        self.catalog["soldiers"] = {
            "Knight02#52": {"filename": "Knight02"},
            "Knight02#54": {"filename": "Knight02"},
        }
        self.assertEqual(resolve_soldier(self.catalog, "knight02__54")[0], "Knight02#54")
        with self.assertRaises(ValueError):
            resolve_soldier(self.catalog, "knight02")

    def test_mismatched_archetype_and_existing_destination_are_errors(self):
        with self.assertRaises(ValueError):
            soldier_copy_patch(self.catalog, "guard04", "Guard03", "existing")
        self.catalog["soldiers"]["Guard03"]["rider"] = True
        with self.assertRaises(ValueError):
            soldier_copy_patch(self.catalog, "guard04", "New", "new", progression_from="guard03")


if __name__ == "__main__":
    unittest.main()
