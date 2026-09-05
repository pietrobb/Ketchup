"""Translate the trusted Blender garden-studio generator's complete geometry.

Run with sdk/python on PYTHONPATH. --source must be the operator's trusted Python
script, not untrusted input: its dimension assignments and build functions execute.
Presentation code is not executed. Metres become millimetres, with no rounding.
The optional --reference-only mode runs inside background Blender and independently
measures the original builders, including Blender's applied Boolean openings.
"""
import argparse
import ast
import hashlib
import itertools
import json
import math
from pathlib import Path
import sys

BUILDERS = ('build_site', 'build_shell', 'build_openings', 'build_deck',
            'build_cladding', 'build_interior')
PARTS = []


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
    argv = sys.argv[sys.argv.index('--') + 1:] if '--' in sys.argv else None
    args = parser.parse_args(argv)
    output = Path(args.output).resolve()
    report_path = output.with_suffix('.validation.json')
    if output.exists() or (not args.reference_only and report_path.exists()):
        raise SystemExit('Output already exists; choose a new filename.')
    if args.reference_only:
        blender_reference(args.source, output)
        return
    from ketchup import Session
    info = design(args.source)
    reference = json.loads(Path(args.reference).read_text(encoding='utf-8')) if args.reference else None
    if reference:
        assert reference['source']['sha256'] == info['sha256']
        assert set(reference['objects']) == {p['name'] for p in PARTS}
    config = dict(executable=args.executable, worker=args.worker, timeout=360)
    print(f'Design: {len(PARTS)} source objects; no geometric omissions', flush=True)
    ids = {}
    with Session(**config) as session:
        doc = session.new_document()
        for i, part in enumerate(PARTS):
            ids[part['name']] = create(doc, part)
            print(f'Created {i + 1}/{len(PARTS)} {part["name"]}', flush=True)
        doc.set_grounded([ids['Terrain']['occurrence_id']])
        evaluation, comparisons = verify(doc, ids, reference)
        native = doc.validators.run('collision')
        saved = doc.save(str(output))
    with Session(**config) as session:
        doc = session.open_document(str(output))
        assert doc.state['canonical_digest'] == saved['state']['canonical_digest']
        reopened, _ = verify(doc, ids, reference)
        signatures = lambda e: sorted((g['definition_id'], g['feature_id'], g['result_fingerprint']) for g in e['geometry'])
        assert signatures(evaluation) == signatures(reopened)
        assert native['collision'] == doc.validators.run('collision')['collision']
    proof = {'source': info, 'model': str(output), 'part_count': len(PARTS),
             'canonical_digest': saved['state']['canonical_digest'], 'parts': PARTS, 'ids': ids,
             'comparisons': comparisons, 'evaluation': evaluation, 'native_validator': native,
             'blender_reference_verified': bool(reference), 'fresh_process_reopen_verified': True,
             'collision_scope': 'Native conservative envelopes; coverage and issue caps retained verbatim. Not an exact collision certificate.',
             'presentation': 'Materials, transparency, lighting and cameras intentionally deferred.'}
    report_path.write_text(json.dumps(proof, indent=2, allow_nan=False), encoding='utf-8')
    print(json.dumps({'model': str(output), 'report': str(report_path), 'objects': len(PARTS),
                      'blender_reference_verified': bool(reference), 'reopen': 'passed'}), flush=True)


if __name__ == '__main__':
    main()
