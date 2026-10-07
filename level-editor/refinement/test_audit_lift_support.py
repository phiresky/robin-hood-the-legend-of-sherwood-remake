import copy
import importlib.util
import math
from pathlib import Path
import unittest


spec = importlib.util.spec_from_file_location(
    "audit_lift_support", Path(__file__).with_name("audit-lift-support.py")
)
audit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)


def rectangle(x0, y0, x1, y1):
    return [[x0, y0], [x1, y0], [x1, y1], [x0, y1]]


class ContactSupportTests(unittest.TestCase):
    def setUp(self):
        self.bound = {
            "definition": {
                "boundary": rectangle(-10, -10, 10, 0),
                "plane": [0, 1, 0], "obstacles": [],
                "doors": [{"inside": [0, 0, 0], "middle": [0, 0, 0], "outside": [0, 4, 0]}],
            },
            "landings": [], "layer": 0, "area": 0, "obstacle_states": [],
        }
        self.landing = {
            "boundary": rectangle(-10, 0, 10, 10), "holes": [], "obstacles": [],
            "layer": 1, "area": 0, "plane": [0, 0, 0],
        }

    def contact(self, states=None):
        return audit.inspect(self.bound, states or [[0], [0]], [6, 3])["doors"][0]["inside"]

    def test_connected_landing_supplies_only_its_own_free_coverage(self):
        self.assertEqual(self.contact()["unsupported_area"], 20)
        self.bound["landings"] = [self.landing]
        self.assertEqual(self.contact()["unsupported_area"], 0)
        self.landing["holes"] = [rectangle(-1, -1, 1, 1)]
        # The hole may remove landing support, but not the separate flight.
        self.assertEqual(self.contact()["unsupported_area"], 2)

    def test_motion_states_control_flight_and_landing_obstacles(self):
        self.bound["landings"] = [self.landing]
        self.landing["obstacles"] = [{
            "state": 2, "polygon": rectangle(-1, 0, 1, 1),
        }]
        self.bound["definition"]["obstacles"] = [{
            "motion_obstacle": 0, "polygon": rectangle(-1, -1, 1, 0),
        }]
        self.bound["obstacle_states"] = [4]
        self.assertEqual(self.contact()["blocked_area"], 0)
        self.assertEqual(self.contact([[4], [0]])["blocked_area"], 2)
        self.assertEqual(self.contact([[0], [2]])["unsupported_area"], 2)

    def test_height_diagnostic_is_explicit_extrapolation_without_mutation(self):
        before = copy.deepcopy(self.bound)
        result = audit.inspect(self.bound, [[0], [0]], [6, 3])
        self.assertEqual(result["flight_height_range"], [-10, 0])
        self.assertEqual(result["doors"][0]["inside"]["extrapolated_plane_footprint_height_range"], [-2, 2])
        self.assertEqual(self.bound, before)

    def test_surface_frame_preserves_3d_lengths_and_plane_heights(self):
        for a, b in [[0, 0], [2, -5], [1, 0], [0, 1], [-9, -9]]:
            with self.subTest(a=a, b=b):
                self.bound["definition"]["plane"] = [a, b, 0]
                before = copy.deepcopy(self.bound)
                framed = audit.clearance_frame(self.bound, "surface")["definition"]
                points = self.bound["definition"]["boundary"]
                for i, (p, q) in enumerate(zip(points, points[1:])):
                    dx, dy = q[0]-p[0], q[1]-p[1]
                    fp, fq = framed["boundary"][i:i+2]
                    self.assertAlmostEqual(math.hypot(dx, dy, a*dx+b*dy),
                                           math.dist(fp, fq), places=10)
                    fa, fb, fc = framed["plane"]
                    self.assertAlmostEqual(fa*fp[0]+fb*fp[1]+fc, a*p[0]+b*p[1], places=10)
                self.assertEqual(self.bound, before)

    def test_alternative_frame_does_not_reduce_actor_dimensions(self):
        self.bound["definition"]["boundary"] = rectangle(-10, -1, 10, 1)
        self.bound["definition"]["plane"] = [0, 20, 0]
        self.assertEqual(self.contact()["unsupported_area"], 20)
        framed = audit.clearance_frame(self.bound, "surface")
        contact = audit.inspect(framed, [[0], [0]], [6, 3])["doors"][0]["inside"]
        self.assertEqual(contact["unsupported_area"], 0)
        # Still rejects a contact whose across-surface width cannot hold the box.
        self.bound["definition"]["boundary"] = rectangle(-4, -1, 4, 1)
        framed = audit.clearance_frame(self.bound, "surface")
        contact = audit.inspect(framed, [[0], [0]], [6, 3])["doors"][0]["inside"]
        self.assertEqual(contact["unsupported_area"], 8)

    def test_screen_collapse_and_piecewise_floors_are_not_faked(self):
        self.bound["definition"]["plane"] = [0, 1, 0]
        with self.assertRaisesRegex(ValueError, "degenerate"):
            audit.clearance_frame(self.bound, "screen")
        audit.clearance_frame(self.bound, "surface")
        self.bound["definition"]["floor_patches"] = [{}]
        with self.assertRaisesRegex(ValueError, "piecewise"):
            audit.clearance_frame(self.bound, "surface")

    def test_clear_endpoints_do_not_hide_a_blocked_straight_sweep(self):
        self.bound["definition"]["boundary"] = rectangle(-20, -10, 20, 10)
        self.bound["definition"]["doors"] = [
            {"inside": [x, 0, 0], "middle": [x, 0, 0]} for x in [-6, 6]
        ]
        self.bound["definition"]["obstacles"] = [{
            "motion_obstacle": 0, "polygon": rectangle(-0.5, -0.5, 0.5, 0.5),
        }]
        self.bound["obstacle_states"] = [0]
        result = audit.inspect(self.bound, [[0], [0]], [6, 3])
        self.assertTrue(all(door["inside"]["blocked_area"] == 0 for door in result["doors"]))
        route = result["direct_inside_routes"][0]
        self.assertEqual(route["blocked_area"], 1)
        self.assertEqual(route["unsupported_area"], 0)
        self.assertEqual(route["center_length_outside_flight"], 0)

    def test_saved_runtime_flag_selects_surface_clearance(self):
        self.bound["definition"]["boundary"] = rectangle(-10, -1, 10, 1)
        self.bound["definition"]["plane"] = [0, 20, 0]
        offsets = audit.runtime_offsets(self.bound, [6, 3])
        self.assertEqual(audit.inspect(self.bound, [[0]], [6, 3], offsets)["doors"][0]["inside"]["unsupported_area"], 20)
        self.bound["climbing"] = True
        offsets = audit.runtime_offsets(self.bound, [6, 3])
        self.assertEqual(audit.inspect(self.bound, [[0]], [6, 3], offsets)["doors"][0]["inside"]["unsupported_area"], 0)


if __name__ == "__main__":
    unittest.main()
