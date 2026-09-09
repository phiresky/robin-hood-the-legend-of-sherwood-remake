"""Synthetic orchestration tests, not GPU or game acceptance."""
from pathlib import Path
import struct
import tempfile
import unittest
from unittest.mock import Mock

from capture_live import dimensions, exercise_capture


def png(width, height, pixels=b"fixture"):
    return b"\x89PNG\r\n\x1a\n" + struct.pack(">I", 13) + b"IHDR" + struct.pack(">II", width, height) + pixels


class CaptureLiveTests(unittest.TestCase):
    def test_invalid_images_are_not_success(self):
        for image in (b"", b"not png", png(0, 20)):
            with self.assertRaises(RuntimeError):
                dimensions(image)

    def run_capture(self, images, states):
        summary = {"checks": {}}
        with tempfile.TemporaryDirectory() as root:
            exercise_capture(Mock(side_effect=states), Mock(side_effect=images), Path(root), summary)
        return summary

    def test_success_requires_both_repeats_and_unchanged_state(self):
        view, full = png(10, 10), png(100, 100)
        summary = self.run_capture([view, full, view, full, view], [{"frame": 10}] * 3)
        self.assertTrue(summary["checks"]["full_map_preserves_live_view_and_state"])
        self.assertTrue(summary["checks"]["repeated_full_map_pixels_match"])

    def test_changed_viewport_or_engine_or_map_is_rejected(self):
        view, full = png(10, 10), png(100, 100)
        cases = [([view, full, png(10, 10, b"changed")], [{"frame": 10}], "viewport"),
                 ([view, full, view], [{"frame": 10}, {"frame": 11}], "engine state"),
                 ([view, full, view, png(100, 100, b"changed"), view], [{"frame": 10}] * 3, "captures differ"),
                 ([view, view], [{"frame": 10}], "dimensions")]
        for images, states, error in cases:
            with self.subTest(error=error), self.assertRaisesRegex(RuntimeError, error):
                self.run_capture(images, states)
