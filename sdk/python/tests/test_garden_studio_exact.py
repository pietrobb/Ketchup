"""Geometry/color translator regressions without Blender or a running CAD process."""
import copy
import hashlib
import importlib.util
import json
import math
from pathlib import Path
import sys
import tempfile
import types
import unittest
from unittest.mock import patch

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


class GardenColorTests(unittest.TestCase):
    def setUp(self):
        house.PARTS.clear()
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.source = self.root / 'source.py'
        self.source.write_text('raise RuntimeError("top level must not execute")\n'
                               'def material(*args, **kwargs):\n'
                               '    raise RuntimeError("material must not execute")\n'
                               'MAT_GLASS = material("Glass", (0.22, 0.45, 0.60), alpha=0.3, transmission=0.78)\n',
                               encoding='utf-8')

    def test_linear_to_srgb_threshold_endpoints_and_known_values(self):
        self.assertEqual(house.linear_to_srgb_bytes([0, 1, 0.5]), [0, 255, 188])
        self.assertEqual(house.linear_to_srgb_bytes([0.003, 0.0031308, 0.0032]), [10, 10, 11])
        self.assertEqual(house.linear_to_srgb_bytes([0.22, 0.45, 0.60]), [129, 179, 203])
        self.assertEqual(house.linear_to_srgb_bytes([0.12, 0.24, 0.10]), [97, 134, 89])

    def test_reject_invalid_rgb(self):
        for rgb in ([0, 1], [0, 1, 0, 1], [True, 0, 0], ['0', 0, 0],
                    [-0.1, 0, 0], [1.1, 0, 0], [math.nan, 0, 0], [math.inf, 0, 0], None):
            with self.subTest(rgb=rgb), self.assertRaises(ValueError):
                house.linear_to_srgb_bytes(rgb)

    def test_ast_literals_do_not_execute_source_or_material(self):
        digest, palette = house.material_colors(self.source)
        self.assertEqual(digest, hashlib.sha256(self.source.read_text(encoding='utf-8').encode()).hexdigest())
        self.assertEqual(palette['MAT_GLASS']['color'], [129, 179, 203])
        self.assertEqual(palette['MAT_GLASS']['ignored_parameters'], {'alpha': 0.3, 'transmission': 0.78})
        self.assertEqual(palette['MAT_GLASS']['linear_rgb'], (0.22, 0.45, 0.60))

    def test_ast_keyword_arguments(self):
        self.source.write_text('MAT_A = material(color=[0, 0.5, 1], name="A")', encoding='utf-8')
        self.assertEqual(house.material_colors(self.source)[1]['MAT_A']['color'], [0, 188, 255])

    def test_nonliteral_and_ambiguous_materials_fail_closed(self):
        for text in ('MAT_A = material("A", dangerous())',
                     'MAT_A = material("A", (0, 0, 0), alpha=dangerous())',
                     'MAT_A = material("A", RGB)',
                     'MAT_A = material("A", (0, 0, 0), **settings)',
                     'MAT_A = material("A", (0, 0, 0), color=(1, 1, 1))',
                     'MAT_A = material("A", (0, 0, 0))\nMAT_A = material("B", (1, 1, 1))',
                     'MAT_A = other("A", (0, 0, 0))',
                     'MAT_A = MAT_B = material("A", (0, 0, 0))',
                     'MAT_A = material(42, (0, 0, 0))',
                     'def nested():\n    MAT_A = material("A", (0, 0, 0))'):
            with self.subTest(text=text), self.assertRaises(ValueError):
                self.source.write_text(text, encoding='utf-8')
                house.material_colors(self.source)

    def fixture(self):
        digest, palette = house.material_colors(self.source)
        parts = [{'name': f'Part_{i}', 'source_material': 'MAT_GLASS'} for i in range(140)]
        ids = {p['name']: {'occurrence_id': i + 1, 'definition_id': i + 1} for i, p in enumerate(parts)}
        state = {'document_id': 12, 'canonical_digest': 'original', 'definitions': [], 'features': [],
                 'occurrences': [{'name': name, 'id': item['occurrence_id'],
                                  'definition_id': item['definition_id'], 'transform': [1, 0, 0], 'color': None}
                                 for name, item in ids.items()]}
        evaluation = {'complete': True, 'geometry': [
            {'definition_id': i + 1, 'feature_id': i + 1, 'result_fingerprint': f'geometry-{i}'}
            for i in range(140)]}
        proof = {'source': {'sha256': digest, 'generator_version': 'test'}, 'part_count': 140,
                 'parts': parts, 'ids': ids, 'evaluation': evaluation}
        model = self.root / 'exact.ketchup'
        model.write_bytes(b'original model sentinel')
        model.with_suffix('.validation.json').write_text(json.dumps(proof), encoding='utf-8')
        house.PARTS[:] = parts
        return model, proof, state, palette

    def test_name_mapping_is_exact_and_order_independent(self):
        _, proof, state, palette = self.fixture()
        state['occurrences'].reverse()
        plan = house.color_plan(state, proof['ids'], palette)
        self.assertEqual(len(plan), 140)
        self.assertEqual(plan['Part_139'], {'occurrence_id': 140, 'source_material': 'MAT_GLASS',
                                          'color': [129, 179, 203]})
        for damage in ('duplicate', 'missing', 'renamed', 'identity', 'definition', 'material'):
            broken, colors = copy.deepcopy(state), copy.deepcopy(palette)
            if damage == 'duplicate':
                broken['occurrences'].append(broken['occurrences'][0])
            elif damage == 'missing':
                broken['occurrences'].pop()
            elif damage == 'renamed':
                broken['occurrences'][0]['name'] = 'Wrong'
            elif damage == 'identity':
                broken['occurrences'][0]['id'] = 999
            elif damage == 'definition':
                broken['occurrences'][0]['definition_id'] = 999
            else:
                colors.clear()
            with self.subTest(damage=damage), self.assertRaises(ValueError):
                house.color_plan(broken, proof['ids'], colors)

    def test_original_evidence_hash_and_all_140_names_required(self):
        model, proof, _, _ = self.fixture()
        source_hash = proof['source']['sha256']
        loaded, provenance = house.load_existing_evidence(model, source_hash)
        self.assertEqual(loaded, proof)
        self.assertTrue(provenance['source_sha256_verified'])
        with self.assertRaisesRegex(ValueError, 'SHA256'):
            house.load_existing_evidence(model, 'wrong')
        proof['parts'][-1] = proof['parts'][0]
        model.with_suffix('.validation.json').write_text(json.dumps(proof), encoding='utf-8')
        with self.assertRaisesRegex(ValueError, '140'):
            house.load_existing_evidence(model, source_hash)

    def run_existing(self, damage=None, extra=()):
        model, proof, state, _ = self.fixture()
        output = self.root / 'colored.ketchup'
        calls, sessions = [], []
        saved_state = None

        class Document:
            def __init__(self, current):
                self.state = copy.deepcopy(current)
                self.validators = types.SimpleNamespace(run=lambda _: {'collision': {'checked': 100, 'cap': 100}})

            def set_color(self, occurrence_ids, color):
                calls.append((occurrence_ids, color))
                for occurrence in self.state['occurrences']:
                    if occurrence['id'] in occurrence_ids:
                        occurrence['color'] = list(color)
                self.state['canonical_digest'] = 'colored'
                if damage == 'transform':
                    self.state['occurrences'][0]['transform'][0] = 2

            def save(self, path):
                nonlocal saved_state
                saved_state = copy.deepcopy(self.state)
                Path(path).write_bytes(b'colored model sentinel')
                return {'state': copy.deepcopy(self.state)}

        class Session:
            def __init__(self, **kwargs):
                sessions.append(self)

            def __enter__(self):
                return self

            def __exit__(self, *_):
                pass

            def open_document(self, path):
                current = copy.deepcopy(state if Path(path) == model else saved_state)
                if Path(path) == output and damage == 'reopen_color':
                    current['occurrences'][0]['color'] = None
                return Document(current)

        def verify(doc, ids, reference):
            evaluation = copy.deepcopy(proof['evaluation'])
            if damage == 'fingerprint' and calls:
                evaluation['geometry'][0]['result_fingerprint'] = 'changed'
            if damage == 'original_fingerprint':
                evaluation['geometry'][0]['result_fingerprint'] = 'wrong input model'
            return evaluation, []

        argv = ['garden_studio_exact.py', '--source', str(self.source), '--color-existing', str(model),
                '--output', str(output), *extra]
        original_bytes = model.read_bytes(), model.with_suffix('.validation.json').read_bytes()
        with patch.object(sys, 'argv', argv), patch.dict(sys.modules, {'ketchup': types.SimpleNamespace(Session=Session)}), \
                patch.object(house, 'design', side_effect=AssertionError('must not execute builders')), \
                patch.object(house, 'create', side_effect=AssertionError('must not regenerate geometry')), \
                patch.object(house, 'verify', side_effect=verify):
            house.main()
        self.assertEqual(original_bytes, (model.read_bytes(), model.with_suffix('.validation.json').read_bytes()))
        return output, calls, sessions

    def test_existing_cli_colors_only_and_verifies_fresh_reopen(self):
        output, calls, sessions = self.run_existing()
        self.assertEqual(len(sessions), 2)
        self.assertEqual(calls, [(list(range(1, 141)), [129, 179, 203])])
        report = json.loads(output.with_suffix('.validation.json').read_text(encoding='utf-8'))
        transfer = report['color_transfer']
        self.assertEqual(transfer['colored_occurrence_count'], 140)
        self.assertFalse(transfer['geometry_regenerated'])
        self.assertTrue(transfer['save_open_color_equality_verified'])
        self.assertTrue(transfer['geometry_unchanged_verified'])
        self.assertEqual(transfer['geometry_fingerprints_before'], transfer['geometry_fingerprints_reopened'])
        self.assertTrue(report['original_exact']['files_unchanged_verified'])
        self.assertIn('Glass remains opaque', report['presentation'])
        self.assertEqual(report['native_validator']['collision']['cap'], 100)

    def test_transfer_and_reopen_fail_on_geometry_or_color_change(self):
        for damage in ('transform', 'fingerprint', 'original_fingerprint', 'reopen_color'):
            with self.subTest(damage=damage), self.assertRaises(ValueError):
                self.run_existing(damage)
            self.assertFalse((self.root / 'colored.validation.json').exists())
            (self.root / 'colored.ketchup').unlink(missing_ok=True)

    def test_cli_never_overwrites_input_output_or_evidence(self):
        model, _, _, _ = self.fixture()
        output = self.root / 'colored.ketchup'
        for target in (model, output, output.with_suffix('.validation.json')):
            if target != model:
                target.write_bytes(b'existing sentinel')
            chosen = model if target == model else output
            argv = ['house', '--source', str(self.source), '--color-existing', str(model), '--output', str(chosen)]
            before = target.read_bytes()
            with patch.object(sys, 'argv', argv), self.assertRaises(SystemExit):
                house.main()
            self.assertEqual(target.read_bytes(), before)
            if target != model:
                target.unlink()

    def test_reference_mode_rejects_color_flags(self):
        for flag in ('--colors', '--color-existing'):
            argv = ['house', '--source', str(self.source), '--output', str(self.root / 'new.json'),
                    '--reference-only', flag]
            if flag == '--color-existing':
                argv.append(str(self.root / 'exact.ketchup'))
            with patch.object(sys, 'argv', argv), self.assertRaises(SystemExit):
                house.main()

    def test_operator_source_matches_original_140_part_material_mapping(self):
        source = Path('C:/Sources8/Supervisor/temp/blender_house_pilot/generate_house.py')
        evidence = Path(__file__).resolve().parents[3] / 'examples/garden-studio-exact.validation.json'
        if not source.exists() or not evidence.exists():
            self.skipTest('Operator Blender source/exact evidence not available on this machine.')
        digest, palette = house.material_colors(source)
        original = json.loads(evidence.read_text(encoding='utf-8'))
        self.assertEqual(digest, original['source']['sha256'])
        self.assertEqual(len(palette), 20)
        # This explicit trusted-builder test compares the preserved sidecar mapping;
        # the --color-existing production path does not execute these builders.
        info = house.design(source)
        self.assertEqual(info['sha256'], digest)
        self.assertEqual(house.PARTS, original['parts'])
        self.assertEqual(len(house.PARTS), 140)
        self.assertTrue(all(p['source_material'] in palette for p in house.PARTS))
        self.assertEqual(palette['MAT_GLASS']['color'], [129, 179, 203])
        self.assertEqual(palette['MAT_DARK_WOOD']['color'], [53, 44, 39])


if __name__ == '__main__':
    unittest.main()
