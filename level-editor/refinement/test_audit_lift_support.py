import copy
import importlib.util
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
                "doors": [{"inside": [0, 0, 0], "middle": [0, 0, 0]}],
            },
            "landings": [], "layer": 0, "area": 0, "obstacle_states": [],
        }
        self.landing = {
            "boundary": rectangle(-10, 0, 10, 10), "holes": [], "obstacles": [],
            "layer": 1, "area": 0,
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


if __name__ == "__main__":
    unittest.main()
