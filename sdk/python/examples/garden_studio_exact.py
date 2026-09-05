"""Translate the trusted Blender garden-studio generator's complete geometry.

Run with sdk/python on PYTHONPATH. --source must be the operator's trusted Python
script, not untrusted input: geometry generation executes its dimension assignments
and build functions, never top-level delete/save/render or presentation commands.
Metres become millimetres, with no rounding. --reference-only runs inside Blender.
--colors adds base colors during generation. --color-existing reads the original
.validation.json sidecar and colors an existing exact model without executing any
source code or generating geometry. Both require a new --output filename.
Material calls are parsed as AST literals only; scene-linear RGB becomes sRGB bytes.
Glass remains opaque: transparency, physical materials and Blender lighting/view
transforms are not reproduced.
"""
import argparse
import ast
import hashlib
from copy import deepcopy
import json
import math
from pathlib import Path
import sys

BUILDERS = ('build_site', 'build_shell', 'build_openings', 'build_deck',
            'build_cladding', 'build_interior')
PARTS = []
COLOR_LIMITS = ('Base color only: Blender scene-linear RGB converted to rounded 8-bit sRGB. '
                'Glass remains opaque; alpha, transmission, metallic and roughness are not applied. '
                'No textures, lights, shadows, reflections, cameras, Filmic/view transform or exposure '
                'are transferred; this is not Blender render parity.')


def linear_to_srgb_bytes(color):
    """IEC 61966-2-1 transfer, nearest byte (half up); reject unsupported HDR."""
    if not isinstance(color, (tuple, list)) or len(color) != 3:
        raise ValueError('Material color must contain three scene-linear RGB channels.')
    result = []
    for value in color:
        if type(value) not in (int, float) or not math.isfinite(value) or not 0 <= value <= 1:
            raise ValueError('Material RGB channels must be finite numbers in [0, 1].')
        encoded = 12.92 * value if value <= 0.0031308 else 1.055 * value ** (1 / 2.4) - 0.055
        result.append(int(math.floor(encoded * 255 + 0.5)))
    return result


def material_colors(source):
    """Read top-level MAT_* = material(...) literals, never execute material/source."""
    text = Path(source).read_text(encoding='utf-8')
    tree = ast.parse(text, filename=str(source))
    palette = {}
    parameters = ('name', 'color', 'metallic', 'roughness', 'transmission', 'alpha')
    for node in tree.body:
        if not isinstance(node, ast.Assign):
            continue
        targets = [t.id for t in node.targets if isinstance(t, ast.Name) and t.id.startswith('MAT_')]
        if not targets:
            continue
        call = node.value
        if (len(targets) != 1 or len(node.targets) != 1 or targets[0] in palette
                or not isinstance(call, ast.Call) or not isinstance(call.func, ast.Name)
                or call.func.id != 'material' or len(call.args) > len(parameters)):
            raise ValueError('Expected a unique MAT_* = material(...) literal assignment.')
        values = dict(zip(parameters, (ast.literal_eval(arg) for arg in call.args)))
        for keyword in call.keywords:
            if keyword.arg not in parameters or keyword.arg in values:
                raise ValueError('Unsupported or duplicate material argument.')
            values[keyword.arg] = ast.literal_eval(keyword.value)
        if not isinstance(values.get('name'), str) or 'color' not in values:
            raise ValueError('Material requires literal name and RGB color.')
        palette[targets[0]] = {'name': values['name'], 'linear_rgb': values['color'],
                               'color': linear_to_srgb_bytes(values['color']),
                               'ignored_parameters': {k: v for k, v in values.items()
                                                      if k not in ('name', 'color')}}
    if not palette:
        raise ValueError('No literal material assignments found.')
    return hashlib.sha256(text.encode()).hexdigest(), palette


def load_source(path, overrides=None):
    text = Path(path).read_text(encoding='utf-8')
    tree = ast.parse(text, filename=str(path))
    # Only the dimension section and named geometry builders, never the source's
    # top-level delete/save/render commands or presentation setup.
    start = next(n.lineno for n in tree.body if isinstance(n, ast.Assign)
                 and any(isinstance(t, ast.Name) and t.id == 'GENERATOR_VERSION' for t in n.targets))
    end = next(n.lineno for n in tree.body if isinstance(n, ast.FunctionDef) and n.name == 'span')
    names = set(BUILDERS) | {'span'}
    if overrides is None:
        names |= {'collection', 'prism', 'box', 'tube', 'cut'}
    nodes = [n for n in tree.body if (start <= n.lineno < end and isinstance(n, ast.Assign))
             or (isinstance(n, ast.FunctionDef) and n.name in names)]
    env = {'math': math, 'BOXES': []}
    materials = {n.id for n in ast.walk(tree) if isinstance(n, ast.Name) and n.id.startswith('MAT_')}
    if overrides is None:
        import bpy
        env['bpy'] = bpy
        env.update({name: bpy.data.materials.new(name) for name in materials})
    else:
        env.update({name: name for name in materials})
        env.update(overrides)
    exec(compile(ast.Module(body=nodes, type_ignores=[]), str(path), 'exec'), env)
    for name in BUILDERS:
        env[name]()
    return {'path': str(Path(path).resolve()), 'sha256': hashlib.sha256(text.encode()).hexdigest(),
            'generator_version': env['GENERATOR_VERSION']}


def prism(name, axis, a0, a1, profile, mat, col):
    part = {'name': name, 'kind': 'prism', 'axis': axis, 'low': a0 * 1000,
            'high': a1 * 1000, 'profile': [[u * 1000, v * 1000] for u, v in profile],
            'collection': col, 'source_material': mat, 'cuts': []}
    PARTS.append(part)
    return part


def box(name, x0, x1, y0, y1, z0, z1, mat, col='Architecture', record=True):
    return prism(name, 2, z0, z1, [(x0, y0), (x1, y0), (x1, y1), (x0, y1)], mat, col)


def tube(name, cx, cy, z0, z1, r0, r1, mat, col, segments=32):
    angles = [math.tau * i / segments for i in range(segments)]
    part = prism(name, 2, z0, z1, [(cx + r0 * math.cos(a), cy + r0 * math.sin(a))
                                 for a in angles], mat, col)
    part.update(kind='tube', center=[cx * 1000, cy * 1000], radii=[r0 * 1000, r1 * 1000],
                segments=segments)
    return part


def cut(target, name, x0, x1, y0, y1, z0, z1):
    target['cuts'].append({'name': name, 'bounds': [[x0 * 1000, y0 * 1000, z0 * 1000],
                                                  [x1 * 1000, y1 * 1000, z1 * 1000]]})


def design(source):
    PARTS.clear()
    info = load_source(source, dict(prism=prism, box=box, tube=tube, cut=cut))
    assert len({p['name'] for p in PARTS}) == len(PARTS)
    return info


def lines(points):
    return [{'type': 'line', 'id': i + 1, 'start_mm': list(a), 'end_mm': list(b)}
            for i, (a, b) in enumerate(zip(points, points[1:] + points[:1]))]


def feature_id(result, kind):
    ids = set(result['created']['feature_ids'])
    return next(f['id'] for f in result['state']['features'] if f['id'] in ids and f['kind'] == kind)


def frame(origin, x, y):
    return {'type': 'frame', 'origin_mm': origin, 'x_axis': x, 'y_axis': y}


def add_pocket(doc, definition, target, name, points, workplane, depth):
    sketch = doc.create_sketch(definition, name + ' profile', lines(points), workplane=workplane)
    result = doc.pocket(definition, name, target, feature_id(sketch, 'Sketch'), depth)
    return feature_id(result, 'Pocket')


def create(doc, part):
    axis, low, high = part['axis'], part['low'], part['high']
    translation = [low, 0, 0] if axis == 0 else [0, 0, low]
    result = doc.extrude(part['name'], lines(part['profile']), high - low,
                         plane='yz' if axis == 0 else 'xy', translation_mm=translation)
    definition = result['created']['definition_ids'][0]
    occurrence = result['created']['occurrence_ids'][0]
    target = feature_id(result, 'Pad')
    for cutter in part['cuts']:
        a, b = [[v - t for v, t in zip(p, translation)] for p in cutter['bounds']]
        # Cut along the wall thickness, not the original gable extrusion axis.
        if part['name'] in ('Wall_Front', 'Wall_Rear'):
            points = [[a[0], a[2]], [b[0], a[2]], [b[0], b[2]], [a[0], b[2]]]
            plane = frame([0, b[1], 0], [1, 0, 0], [0, 0, 1])
            depth = b[1] - a[1]
        else:
            points = [[a[1], a[2]], [b[1], a[2]], [b[1], b[2]], [a[1], b[2]]]
            plane = frame([a[0], 0, 0], [0, 1, 0], [0, 0, 1])
            depth = b[0] - a[0]
        target = add_pocket(doc, definition, target, cutter['name'], points, plane, depth)
    if part['kind'] == 'tube' and part['radii'][0] != part['radii'][1]:
        # Intersect a polygon prism with all tapered side half-spaces using
        # ordinary planar sketch pockets. This retains flat Blender facets.
        r0, r1 = part['radii']
        assert r0 > r1 > 0
        count, height = part['segments'], high - low
        ap0, ap1 = r0 * math.cos(math.pi / count), r1 * math.cos(math.pi / count)
        slope, reach = (ap0 - ap1) / height, r0 + 10
        cx, cy = part['center']
        for i in range(count):
            angle = math.tau * (i + 0.5) / count
            nx, ny = math.cos(angle), math.sin(angle)
            points = [[ap0 + slope, -1], [reach, -1],
                      [reach, height + 1], [ap1 - slope, height + 1]]
            plane = frame([cx - ny * reach, cy + nx * reach, 0],
                          [nx, ny, 0], [0, 0, 1])
            target = add_pocket(doc, definition, target, f'Taper facet {i + 1}', points, plane, 2 * reach)
    return {'definition_id': definition, 'occurrence_id': occurrence,
            'feature_id': target, 'translation_mm': translation}


def polygon_area(points):
    return abs(sum(a[0] * b[1] - b[0] * a[1]
                   for a, b in zip(points, points[1:] + points[:1]))) / 2


def expected_volume(part):
    height = part['high'] - part['low']
    volume = polygon_area(part['profile']) * height
    if part['kind'] == 'tube':
        r0, r1 = part['radii']
        ratio = r1 / r0
        volume *= (1 + ratio + ratio * ratio) / 3
    for cutter in part['cuts']:
        a, b = cutter['bounds']
        # The source openings are rectangles wholly below the roof profile.
        if part['name'] in ('Wall_Front', 'Wall_Rear'):
            ys = [p[0] for p in part['profile']]
            volume -= (b[0] - a[0]) * (max(ys) - min(ys)) * (b[2] - max(a[2], 300))
        else:
            volume -= height * (b[1] - a[1]) * (b[2] - max(a[2], 300))
    return volume


def expected_bounds(part):
    axis = part['axis']
    coords = [[part['low'], part['high']],
              [min(p[0] for p in part['profile']), max(p[0] for p in part['profile'])],
              [min(p[1] for p in part['profile']), max(p[1] for p in part['profile'])]]
    axes = [axis, (axis + 1) % 3, (axis + 2) % 3]
    result = [[0., 0., 0.], [0., 0., 0.]]
    for a, values in zip(axes, coords):
        for j in range(2):
            result[j][a] = values[j]
    return result


def verify(doc, ids, reference):
    evaluation = doc.evaluate(timeout_ms=300000)
    assert evaluation['complete'], {k: v for k, v in evaluation.items() if k not in ('geometry', 'topology_geometry')}
    geometry = {g['definition_id']: g for g in evaluation['geometry']}
    assert len(geometry) == len(PARTS), (len(geometry), len(PARTS))
    occurrences = {o['id']: o for o in doc.state['occurrences']}
    assert len(occurrences) == len(PARTS)
    comparisons = []
    for part in PARTS:
        name = part['name']
        identity = ids[name]
        g = geometry[identity['definition_id']]
        actual_volume = (g.get('native_evidence') or {}).get('volume_mm3', abs(g['mesh_signed_volume_mm3']))
        expected = expected_volume(part)
        assert math.isclose(actual_volume, expected, rel_tol=1e-8, abs_tol=0.02), (name, actual_volume, expected)
        bounds = [[v + t for v, t in zip(p, identity['translation_mm'])] for p in g['bounds_mm']]
        err = max(abs(a - b) for p, q in zip(bounds, expected_bounds(part)) for a, b in zip(p, q))
        assert err <= 2e-5, (name, err, bounds, expected_bounds(part))
        occurrence = occurrences[identity['occurrence_id']]
        tx, ty, tz = identity['translation_mm']
        assert occurrence['name'] == name
        assert occurrence['transform'] == [1, 0, 0, tx, 0, 1, 0, ty, 0, 0, 1, tz, 0, 0, 0, 1]
        comparison = {'name': name, 'volume_mm3': actual_volume, 'bounds_mm': bounds,
                      'analytical_volume_mm3': expected, 'bounds_error_mm': err}
        if reference:
            ref = reference['objects'][name]
            ref_err = max(abs(a - b) for p, q in zip(bounds, ref['bounds_mm']) for a, b in zip(p, q))
            # Blender stores mesh positions in float32 metres; Kečup uses f64 mm.
            assert ref_err <= 0.001, (name, ref_err)
            assert math.isclose(actual_volume, ref['volume_mm3'], rel_tol=3e-5, abs_tol=0.1), (name, actual_volume, ref['volume_mm3'])
            comparison.update(blender_bounds_error_mm=ref_err, blender_volume_mm3=ref['volume_mm3'])
        if name == 'Pendant_Shade':
            topology = g['native_evidence']['topology_counts']
            assert topology[2] == 34 and topology[4] == 1, topology
            comparison['topology_counts'] = topology
        comparisons.append(comparison)
    return evaluation, comparisons


def geometry_signatures(evaluation):
    if not evaluation['complete']:
        raise ValueError('Incomplete geometry evaluation.')
    return sorted((g['definition_id'], g['feature_id'], g['result_fingerprint'])
                  for g in evaluation['geometry'])


def geometry_state(state):
    """Snapshot identity/structure/transforms, excluding only occurrence color."""
    return deepcopy({'document_id': state['document_id'], 'definitions': state['definitions'],
                     'features': state['features'],
                     'occurrences': [{k: v for k, v in o.items() if k != 'color'}
                                     for o in state['occurrences']]})


def load_existing_evidence(model, source_hash):
    """The operator's original exact sidecar supplies source-bound name/material IDs."""
    path = Path(model).with_suffix('.validation.json')
    proof = json.loads(path.read_text(encoding='utf-8'))
    parts = proof['parts']
    names = {p['name'] for p in parts}
    if proof['source']['sha256'] != source_hash:
        raise ValueError('Existing geometry evidence does not match the exact source SHA256.')
    if proof['part_count'] != 140 or len(parts) != 140 or len(names) != 140 or set(proof['ids']) != names:
        raise ValueError('Existing exact evidence must map all 140 unique part names.')
    geometry_signatures(proof['evaluation'])
    return proof, {'model': str(Path(model).resolve()),
                   'model_sha256': hashlib.sha256(Path(model).read_bytes()).hexdigest(),
                   'evidence': str(path.resolve()),
                   'evidence_sha256': hashlib.sha256(path.read_bytes()).hexdigest(),
                   'source_sha256_verified': True}


def color_plan(state, ids, palette):
    """Resolve every source name one-to-one before making any color mutation."""
    occurrences = state['occurrences']
    by_name = {o['name']: o for o in occurrences}
    names = {p['name'] for p in PARTS}
    if (len(names) != len(PARTS) or len(by_name) != len(occurrences)
            or set(by_name) != names or set(ids) != names
            or len({o['id'] for o in occurrences}) != len(occurrences)):
        raise ValueError('Color transfer requires a one-to-one mapping of all source part names.')
    plan = {}
    for part in PARTS:
        name, material = part['name'], part['source_material']
        occurrence, identity = by_name[name], ids[name]
        if (occurrence['id'] != identity['occurrence_id']
                or occurrence['definition_id'] != identity['definition_id']):
            raise ValueError(f'Occurrence identity mismatch: {name}')
        if material not in palette:
            raise ValueError(f'No literal source color for {name}: {material}')
        plan[name] = {'occurrence_id': occurrence['id'], 'source_material': material,
                      'color': palette[material]['color']}
    return plan


def verify_colors(state, plan):
    actual = {o['name']: {'occurrence_id': o['id'], 'color': o.get('color')}
              for o in state['occurrences']}
    expected = {name: {k: item[k] for k in ('occurrence_id', 'color')} for name, item in plan.items()}
    if len(state['occurrences']) != len(plan) or actual != expected:
        raise ValueError('Occurrence colors differ from the source sRGB mapping.')
    return actual


def apply_colors(doc, plan):
    # Group by byte color, not guessed material names; no geometry operations.
    groups = {}
    for item in plan.values():
        groups.setdefault(tuple(item['color']), []).append(item['occurrence_id'])
    for color, occurrences in groups.items():
        doc.set_color(occurrences, list(color))
    return verify_colors(doc.state, plan)


def blender_reference(source, output):
    import bpy
    bpy.ops.object.select_all(action='SELECT')
    bpy.ops.object.delete(use_global=False)
    info = load_source(source)
    objects = {}
    for obj in bpy.context.scene.objects:
        if obj.type != 'MESH':
            continue
        mesh = obj.data
        mesh.calc_loop_triangles()
        points = [tuple(obj.matrix_world @ v.co) for v in mesh.vertices]
        volume = abs(math.fsum(a[0]*(b[1]*c[2]-b[2]*c[1])+a[1]*(b[2]*c[0]-b[0]*c[2])+a[2]*(b[0]*c[1]-b[1]*c[0])
                              for t in mesh.loop_triangles for a, b, c in [[points[i] for i in t.vertices]]) / 6) * 1e9
        objects[obj.name] = {'bounds_mm': [[min(p[k] for p in points) * 1000 for k in range(3)],
                                          [max(p[k] for p in points) * 1000 for k in range(3)]],
                             'volume_mm3': volume,
                             'vertices_mm': [[c * 1000 for c in p] for p in points],
                             'triangles': [list(t.vertices) for t in mesh.loop_triangles]}
    Path(output).write_text(json.dumps({'source': info, 'objects': objects}, indent=2), encoding='utf-8')
    print(f'Blender reference: {len(objects)} objects -> {output}', flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--executable')
    parser.add_argument('--worker')
    parser.add_argument('--reference')
    parser.add_argument('--reference-only', action='store_true')
    parser.add_argument('--colors', action='store_true', help='Transfer literal Blender base colors.')
    parser.add_argument('--color-existing', metavar='PATH',
                        help='Color an exact .ketchup using its original .validation.json; implies --colors.')
    argv = sys.argv[sys.argv.index('--') + 1:] if '--' in sys.argv else None
    args = parser.parse_args(argv)
    output = Path(args.output).resolve()
    report_path = output.with_suffix('.validation.json')
    existing = Path(args.color_existing).resolve() if args.color_existing else None
    colored = args.colors or existing is not None
    if args.reference_only and colored:
        parser.error('--reference-only cannot be combined with color options.')
    if existing is not None and (existing.suffix.lower() != '.ketchup' or not existing.is_file()):
        parser.error('--color-existing must name an existing .ketchup file.')
    if existing is not None and output == existing:
        parser.error('--output must be a distinct new filename, never the existing model.')
    if colored and output.suffix.lower() != '.ketchup':
        parser.error('Colored --output must have a .ketchup extension.')
    if output.exists() or (not args.reference_only and report_path.exists()):
        raise SystemExit('Output already exists; choose a new filename.')
    if args.reference_only:
        blender_reference(args.source, output)
        return
    palette = None
    if colored:
        source_hash, palette = material_colors(args.source)
    original = provenance = None
    if existing is not None:
        original, provenance = load_existing_evidence(existing, source_hash)
        PARTS[:] = original['parts']
        info = dict(original['source'], path=str(Path(args.source).resolve()))
        ids = original['ids']
    else:
        info = design(args.source)
        ids = {}
    if colored and info['sha256'] != source_hash:
        raise ValueError('Source changed while preparing color transfer.')
    reference = json.loads(Path(args.reference).read_text(encoding='utf-8')) if args.reference else None
    if reference:
        assert reference['source']['sha256'] == info['sha256']
        assert set(reference['objects']) == {p['name'] for p in PARTS}
    from ketchup import Session
    config = dict(executable=args.executable, worker=args.worker, timeout=360)
    print(f'Design: {len(PARTS)} source objects; no geometric omissions', flush=True)
    with Session(**config) as session:
        if existing is not None:
            doc = session.open_document(str(existing))
        else:
            doc = session.new_document()
            for i, part in enumerate(PARTS):
                ids[part['name']] = create(doc, part)
                print(f'Created {i + 1}/{len(PARTS)} {part["name"]}', flush=True)
            doc.set_grounded([ids['Terrain']['occurrence_id']])
        evaluation, comparisons = verify(doc, ids, reference)
        baseline = geometry_signatures(evaluation)
        if original is not None and baseline != geometry_signatures(original['evaluation']):
            raise ValueError('Existing model geometry differs from its original exact evidence.')
        if colored:
            before_state = geometry_state(doc.state)
            plan = color_plan(doc.state, ids, palette)
            colors_before_save = apply_colors(doc, plan)
            evaluation, comparisons = verify(doc, ids, reference)
            if geometry_state(doc.state) != before_state or geometry_signatures(evaluation) != baseline:
                raise ValueError('Color transfer changed geometry, transforms or identity.')
        native = doc.validators.run('collision')
        saved = doc.save(str(output))
    with Session(**config) as session:
        doc = session.open_document(str(output))
        assert doc.state['canonical_digest'] == saved['state']['canonical_digest']
        reopened, _ = verify(doc, ids, reference)
        if baseline != geometry_signatures(reopened):
            raise ValueError('Save/Open changed evaluated geometry fingerprints.')
        if colored:
            if geometry_state(doc.state) != before_state or verify_colors(doc.state, plan) != colors_before_save:
                raise ValueError('Save/Open changed colors, geometry structure or identity.')
        assert native['collision'] == doc.validators.run('collision')['collision']
    proof = {'source': info, 'model': str(output), 'part_count': len(PARTS),
             'canonical_digest': saved['state']['canonical_digest'], 'parts': PARTS, 'ids': ids,
             'comparisons': comparisons, 'evaluation': evaluation, 'native_validator': native,
             'blender_reference_verified': bool(reference), 'fresh_process_reopen_verified': True,
             'collision_scope': 'Native conservative envelopes; coverage and issue caps retained verbatim. Not an exact collision certificate.',
             'presentation': COLOR_LIMITS if colored else 'Materials, transparency, lighting and cameras intentionally deferred.'}
    if colored:
        proof['color_transfer'] = {'source_sha256': source_hash, 'materials': palette, 'by_name': plan,
                                   'colored_occurrence_count': len(plan), 'limits': COLOR_LIMITS,
                                   'extraction': 'AST literal material assignments; no material/presentation execution.',
                                   'mapping': 'Exact source builder names' if original is None else
                                              'Original trusted exact evidence names/IDs/materials, bound to source SHA256.',
                                   'geometry_regenerated': existing is None,
                                   'geometry_fingerprints_before': baseline,
                                   'geometry_fingerprints_after': geometry_signatures(evaluation),
                                   'geometry_fingerprints_reopened': geometry_signatures(reopened),
                                   'geometry_unchanged_verified': True, 'save_open_color_equality_verified': True}
    if provenance is not None:
        if (hashlib.sha256(existing.read_bytes()).hexdigest() != provenance['model_sha256']
                or hashlib.sha256(Path(provenance['evidence']).read_bytes()).hexdigest() != provenance['evidence_sha256']):
            raise ValueError('Original geometry model or evidence changed during transfer.')
        proof['original_exact'] = dict(provenance, files_unchanged_verified=True)
    with report_path.open('x', encoding='utf-8') as handle:
        json.dump(proof, handle, indent=2, allow_nan=False)
    print(json.dumps({'model': str(output), 'report': str(report_path), 'objects': len(PARTS),
                      'blender_reference_verified': bool(reference), 'colors': bool(colored), 'reopen': 'passed'}), flush=True)


if __name__ == '__main__':
    main()
