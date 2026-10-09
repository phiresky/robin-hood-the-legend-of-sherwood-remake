"""Binary boundary and pixel-format checks for the basic scene importer."""
import bz2
import struct
import unittest

from import_desperados import read_background, read_sight, sight_chunk


class ImportTests(unittest.TestCase):
    def record(self, projection=False):
        points = [(0, 0, 0, 20), (40, 0, 0, 30), (0, 40, 0, 20)]
        record = struct.pack("<H", len(points))
        record += b"".join(struct.pack("<4f", *p) for p in points)
        record += struct.pack("<6fB", 0, 0, 0, 40, 30, 40, projection)
        if projection:
            record += struct.pack("<HH", 7, 2)
        return record + bytes([32, 1, 1, 0]) + struct.pack("<ffIB", 1, 1, 100, 0)

    def test_conditional_record_alignment(self):
        data = struct.pack("<IH", 6, 2) + self.record(True) + self.record()
        obstacles = read_sight(data)
        self.assertEqual(obstacles[0]["projection_area"], [7, 2])
        self.assertIsNone(obstacles[1]["projection_area"])
        self.assertTrue(obstacles[0]["opaque"])
        self.assertEqual(obstacles[1]["points"][1]["z_top"], 30)
        with self.assertRaises(ValueError):
            read_sight(data + b"extra")
        with self.assertRaises((ValueError, struct.error, IndexError)):
            read_sight(data[:-1])

    def test_chunk_scan_uses_boundaries(self):
        data = struct.pack("<IH", 6, 1) + self.record()
        dvd = b"MISC" + struct.pack("<I", 4) + b"SGHT"
        dvd += b"SGHT" + struct.pack("<I", len(data)) + data
        self.assertEqual(sight_chunk(dvd), data)
        with self.assertRaises(ValueError):
            sight_chunk(dvd[:-1])

    def test_rgb565_channels_and_dimensions(self):
        packed = bz2.compress(struct.pack("<3H", 0xF800, 0x07E0, 0x001F))
        data = struct.pack("<HHII", 3, 1, 2, len(packed)) + packed
        image = read_background(data)
        self.assertEqual([image.getpixel((x, 0)) for x in range(3)], [(255, 0, 0), (0, 255, 0), (0, 0, 255)])
        with self.assertRaises(ValueError):
            read_background(struct.pack("<HHII", 4, 1, 2, len(packed)) + packed)


if __name__ == "__main__":
    unittest.main()
