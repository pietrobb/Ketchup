"""Geometry translator regression tests without Blender or a running CAD process."""
import importlib.util
import math
from pathlib import Path
import unittest

SPEC = importlib.util.spec_from_file_location(
    'garden_studio_exact', Path(__file__).resolve().parents[1] / 'examples/garden_studio_exact.py')
house = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(house)


class RecordingDocument:
    def __init__(self):
        self.sketches = []
        self.pockets = []

    def extrude(self, name, profile, distance, **kwargs):
        return {'created': {'definition_ids': [1], 'occurrence_ids': [1], 'feature_ids': [3]},
                'state': {'features': [{'id': 3, 'kind': 'Pad'}]}}

    def create_sketch(self, definition, name, entities, **kwargs):
        self.sketches.append((entities, kwargs['workplane']))
        return {'created': {'feature_ids': [4]}, 'state': {'features': [{'id': 4, 'kind': 'Sketch'}]}}

    def pocket(self, definition, name, target, profile, depth):
        self.pockets.append(depth)
        return {'created': {'feature_ids': [5]}, 'state': {'features': [{'id': 5, 'kind': 'Pocket'}]}}


class GardenGeometryTests(unittest.TestCase):
    def setUp(self):
        house.PARTS.clear()

    def test_axis_mapping_and_units(self):
        part = house.prism('gable', 0, -3, -2.82, [(-2, .3), (2, .3), (2, 3), (0, 4), (-2, 3)], 'wall', 'Architecture')
        self.assertEqual(house.expected_bounds(part), [[-3000, -2000, 300], [-2820, 2000, 4000]])
        self.assertAlmostEqual(house.expected_volume(part), 2_304_000_000)

    def test_polygon_tube_is_not_a_smooth_cylinder(self):
        part = house.tube('leg', 0, 0, 0, .08, .025, .025, 'frame', 'Interior', 16)
        expected = 16 / 2 * math.sin(math.tau / 16) * 25 ** 2 * 80
        self.assertAlmostEqual(house.expected_volume(part), expected)
        self.assertLess(expected, math.pi * 25 ** 2 * 80)

    def test_frustum_uses_all_source_facets_with_correct_planes(self):
        part = house.tube('shade', 0, 0, 1.86, 2.20, .30, .11, 'metal', 'Interior')
        doc = RecordingDocument()
        house.create(doc, part)
        self.assertEqual(len(doc.pockets), 32)
        for i, (entities, plane) in enumerate(doc.sketches):
            nx, ny, _ = plane['x_axis']
            self.assertAlmostEqual(nx * nx + ny * ny, 1)
            self.assertEqual(plane['y_axis'], [0, 0, 1])
            self.assertAlmostEqual(sum(a * b for a, b in zip(plane['origin_mm'], plane['x_axis'])), 0)
            p0, p1 = entities[0]['start_mm'], entities[-1]['start_mm']
            slope = (p1[0] - p0[0]) / (p1[1] - p0[1])
            for z, radius in ((0, 300), (340, 110)):
                cut_radius = p0[0] + slope * (z - p0[1])
                self.assertAlmostEqual(cut_radius, radius * math.cos(math.pi / 32))
                for angle in (math.tau * i / 32, math.tau * (i + 1) / 32):
                    self.assertAlmostEqual(radius * (math.cos(angle) * nx + math.sin(angle) * ny), cut_radius)
        expected = 32 / 2 * math.sin(math.tau / 32) * 340 / 3 * (300 ** 2 + 300 * 110 + 110 ** 2)
        self.assertAlmostEqual(house.expected_volume(part), expected)


if __name__ == '__main__':
    unittest.main()
