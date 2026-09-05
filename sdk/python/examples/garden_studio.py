"""Create a 6 x 4 m garden studio through the public Kečup process API.

All parts are convex, straight-sided prisms in millimetres. Native collision
validation is retained verbatim. A supplementary convex-polyhedron SAT check
resolves conservative OBB candidates; it is NOT an OCCT Boolean validator.
No Blender, GUI automation, private document edits, or external geometry engine.
"""
import argparse
import itertools
import json
import math
from pathlib import Path

from ketchup import Session


PARTS = []
TOLERANCE_MM = 1e-6


def prism(name, axis, low, high, profile):
    assert axis in (0, 2) and high > low
    area2 = sum(a[0] * b[1] - b[0] * a[1]
                for a, b in zip(profile, profile[1:] + profile[:1]))
    if area2 < 0:
        profile = list(reversed(profile))
    assert abs(area2) > 1e-6
    # SAT is complete only for convex input; reject unsupported shapes.
    for a, b, c in zip(profile, profile[1:] + profile[:1], profile[2:] + profile[:2]):
        assert (b[0]-a[0])*(c[1]-b[1]) - (b[1]-a[1])*(c[0]-b[0]) >= -1e-7
    vertices = []
    for height in (low, high):
        for u, v in profile:
            vertices.append([height, u, v] if axis == 0 else [u, v, height])
    n = len(profile)
    faces = [list(range(n-1, -1, -1)), list(range(n, 2*n))]
    faces.extend([[i, (i+1) % n, (i+1) % n+n, i+n] for i in range(n)])
    translation = [low, 0, 0] if axis == 0 else [0, 0, low]
    entities = [{"type": "line", "id": i+1, "start_mm": a, "end_mm": b}
                for i, (a, b) in enumerate(zip(profile, profile[1:] + profile[:1]))]
    PARTS.append({"name": name, "vertices": vertices, "faces": faces,
                  "volume_mm3": abs(area2) * (high-low) / 2,
                  "operation": {"operation": "create_part", "name": name,
                    "workplane": {"type": "principal", "plane": "yz" if axis == 0 else "xy"},
                    "entities": entities, "constraints": [], "translation_mm": translation,
                    "feature": {"type": "extrusion", "distance_mm": high-low}}})


def box(name, x0, x1, y0, y1, z0, z1):
    assert x1 > x0 and y1 > y0 and z1 > z0
    prism(name, 2, z0, z1, [[x0, y0], [x1, y0], [x1, y1], [x0, y1]])


def build_design():
    PARTS.clear()
    box("Terrain", -4300, 4300, -5100, 3100, -200, 0)
    box("Foundation_Slab", -3125, 3125, -2125, 2125, 0, 300)
    box("Interior_Floor", -2820, 2820, -1820, 1820, 300, 360)
    box("Path", -1150, 1150, -5000, -3950, 0, 50)
    box("Entry_Step", -1300, 1300, -3950, -3600, 0, 150)
    # Walls are partitioned at apertures, with no hidden cutter solids.
    openings = [("Door_Front", 1, -2000, -1820, -1225, 1225, 300, 2450),
                ("Window_Rear", 1, 1820, 2000, 250, 2250, 1050, 2300),
                ("Window_West", 0, -3000, -2820, -850, 950, 1050, 2300),
                ("Window_East", 0, 2820, 3000, -1250, -100, 1200, 2250)]
    for name, axis, lo, hi, u0, u1, z0, z1 in openings:
        wall_lo, wall_hi = (-2820, 2820) if axis == 1 else (-2000, 2000)
        def part(suffix, a, b, c, d, depth_lo=lo, depth_hi=hi):
            if axis == 1:
                box(name+suffix, a, b, depth_lo, depth_hi, c, d)
            else:
                box(name+suffix, depth_lo, depth_hi, a, b, c, d)
        part("_Wall_L", wall_lo, u0, 300, 3000)
        part("_Wall_R", u1, wall_hi, 300, 3000)
        part("_Wall_Header", u0, u1, z1, 3000)
        if z0 > 300:
            part("_Wall_Sill", u0, u1, 300, z0)
        part("_Frame_L", u0, u0+75, z0, z1)
        part("_Frame_R", u1-75, u1, z0, z1)
        part("_Frame_B", u0+75, u1-75, z0, z0+75)
        part("_Frame_T", u0+75, u1-75, z1-75, z1)
        mid = (lo+hi)/2
        part("_Glazing", u0+75, u1-75, z0+75, z1-75, mid-12, mid+12)
    prism("Gable_West", 0, -3000, -2820, [[-2000, 3000], [2000, 3000], [0, 4000]])
    prism("Gable_East", 0, 2820, 3000, [[-2000, 3000], [2000, 3000], [0, 4000]])
    prism("Wall_Front_Slope_Fill", 0, -2820, 2820,
          [[-2000, 3000], [-1820, 3000], [-1820, 3090]])
    prism("Wall_Rear_Slope_Fill", 0, -2820, 2820,
          [[1820, 3000], [2000, 3000], [1820, 3090]])
    prism("Roof_South", 0, -3300, 3300,
          [[-2300, 2850], [0, 4000], [0, 4120], [-2300, 2970]])
    prism("Roof_North", 0, -3300, 3300,
          [[0, 4000], [2300, 2850], [2300, 2970], [0, 4120]])
    for i, (a, b) in enumerate([(-3550, -3400), (-2900, -2750), (-2275, -2125)]):
        box(f"Deck_Joist_{i+1}", -2550, 2550, a, b, 0, 240)
    for i in range(12):
        x = -2550 + i*425
        box(f"Deck_Board_{i+1:02}", x, x+410, -3600, -2125, 240, 300)
    for i, x in enumerate([-2950, -2550, -2150, -1750, 1700, 2100, 2500, 2900]):
        box(f"Front_Timber_Batten_{i+1}", x, x+50, -2035, -2000, 300, 2980)
    for i, y in enumerate([-1750, -1400, 1200, 1600]):
        box(f"West_Timber_Batten_{i+1}", -3035, -3000, y, y+50, 300, 2980)
    box("Desk_Top", -2720, -620, 620, 1340, 740, 820)
    for i, (x, y) in enumerate(itertools.product([-2690, -720], [650, 1240])):
        box(f"Desk_Leg_{i+1}", x, x+70, y, y+70, 360, 740)
    box("Laptop_Base", -1900, -1350, 780, 1140, 820, 845)
    box("Laptop_Screen", -1900, -1350, 1140, 1165, 820, 1200)
    box("Rug", 200, 2500, -600, 1800, 360, 376)
    box("Sofa_Base", 600, 2400, 900, 1620, 376, 680)
    box("Sofa_Back", 600, 2400, 1620, 1780, 376, 1180)
    for i, x in enumerate([630, 1220, 1810]):
        box(f"Sofa_Cushion_{i+1}", x, x+580, 950, 1570, 680, 800)
    box("Coffee_Table_Pedestal", 1230, 1370, 80, 220, 376, 790)
    box("Coffee_Table_Top", 950, 1650, -200, 500, 790, 850)
    for i, y in enumerate([100, 1410]):
        box(f"Shelf_Upright_{i+1}", 2550, 2820, y, y+90, 360, 2100)
    for i, z in enumerate([550, 1150, 1750]):
        box(f"Shelf_Board_{i+1}", 2550, 2820, 190, 1410, z, z+50)
    assert len(PARTS) <= 100, "Native validator coverage must include the whole scene"
    assert len({p['name'] for p in PARTS}) == len(PARTS)


def sub(a, b):
    return [x-y for x, y in zip(a, b)]


def dot(a, b):
    return sum(x*y for x, y in zip(a, b))


def cross(a, b):
    return [a[1]*b[2]-a[2]*b[1], a[2]*b[0]-a[0]*b[2], a[0]*b[1]-a[1]*b[0]]


def directions(part):
    normals, edges = [], []
    vertices = part['vertices']
    for face in part['faces']:
        points = [vertices[i] for i in face]
        normals.append(cross(sub(points[1], points[0]), sub(points[2], points[0])))
        edges.extend(sub(b, a) for a, b in zip(points, points[1:]+points[:1]))
    return normals, edges


def intersects(a, b):
    # Fast exact AABB rejection, followed by the complete convex SAT axis set.
    for axis in range(3):
        if min(max(p[axis] for p in a['vertices']), max(p[axis] for p in b['vertices'])) - max(min(p[axis] for p in a['vertices']), min(p[axis] for p in b['vertices'])) <= TOLERANCE_MM:
            return False
    na, ea = directions(a)
    nb, eb = directions(b)
    for axis in itertools.chain(na, nb, (cross(x, y) for x in ea for y in eb)):
        length = math.sqrt(dot(axis, axis))
        if length <= 1e-12:
            continue
        axis = [x/length for x in axis]
        pa, pb = [dot(p, axis) for p in a['vertices']], [dot(p, axis) for p in b['vertices']]
        if min(max(pa), max(pb)) - max(min(pa), min(pb)) <= TOLERANCE_MM:
            return False
    return True


def self_test():
    box("a", 0, 10, 0, 10, 0, 10)
    box("touch", 10, 20, 0, 10, 0, 10)
    box("overlap", 9, 19, 0, 10, 0, 10)
    assert not intersects(PARTS[0], PARTS[1])
    assert intersects(PARTS[0], PARTS[2])
    # Disjoint triangles whose bounding boxes overlap.
    prism("triangle_a", 0, 0, 10, [[0, 0], [10, 0], [0, 10]])
    prism("triangle_b", 0, 0, 10, [[10, 10], [10, 1], [1, 10]])
    assert not intersects(PARTS[3], PARTS[4])
    PARTS.clear()


def check_evaluation(document, ids):
    report = document.evaluate(timeout_ms=300000)
    if not report['complete'] or len(report['geometry']) != len(PARTS):
        raise RuntimeError(f"Incomplete evaluation: {report}")
    by_id = {g['definition_id']: g for g in report['geometry']}
    for part in PARTS:
        g = by_id[ids[part['name']]['definition_id']]
        # Profiles encode world X/Y or Y/Z, with extrusion low carried by occurrence.
        expected = part['volume_mm3']
        actual = (g.get('native_evidence') or {}).get('volume_mm3', abs(g['mesh_signed_volume_mm3']))
        if not math.isclose(actual, expected, rel_tol=1e-8, abs_tol=0.01):
            raise RuntimeError(f"Wrong evaluated volume for {part['name']}: {actual} vs {expected}")
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--executable', required=True)
    parser.add_argument('--worker', required=True)
    parser.add_argument('--output', required=True)
    args = parser.parse_args()
    output = Path(args.output).resolve()
    report_path = output.with_suffix('.validation.json')
    if output.exists() or report_path.exists():
        raise SystemExit('Output already exists; choose a new output filename.')
    self_test()
    build_design()
    print(f'Design: {len(PARTS)} convex parts', flush=True)
    ids = {}
    config = dict(executable=args.executable, worker=args.worker, timeout=360)
    with Session(**config) as session:
        document = session.new_document()
        for i, part in enumerate(PARTS):
            result = document.apply([part['operation']])
            ids[part['name']] = {'definition_id': result['created']['definition_ids'][0],
                                 'occurrence_id': result['created']['occurrence_ids'][0]}
            if i % 10 == 0:
                print(f"Created {i+1}/{len(PARTS)}: {part['name']}", flush=True)
        document.set_grounded([ids['Terrain']['occurrence_id']])
        evaluation = check_evaluation(document, ids)
        native = document.validators.run('collision')
        if not native['collision']['complete'] or not native['collision']['issues_complete']:
            raise RuntimeError(f'Incomplete native collision coverage: {native}')
        expected_pairs = len(PARTS)*(len(PARTS)-1)//2
        assert native['collision']['checked_pair_count'] == expected_pairs
        clashes = [[a['name'], b['name']] for a, b in itertools.combinations(PARTS, 2) if intersects(a, b)]
        print(f"Native envelope candidates: {native['collision']['issue_count']}; actual convex intersections: {len(clashes)}", flush=True)
        if clashes:
            raise RuntimeError(f'Repair real collisions before saving: {clashes}')
        names = {p['name']: p for p in PARTS}
        resolved = []
        for issue in native['collision']['issues']:
            left, right = issue['left_name'], issue['right_name']
            assert not intersects(names[left], names[right])
            resolved.append({'left': left, 'right': right, 'resolution': 'separated_or_surface_contact_convex_SAT'})
        saved = document.save(str(output))
    with Session(**config) as session:
        reopened = session.open_document(str(output))
        assert reopened.state['canonical_digest'] == saved['state']['canonical_digest']
        reevaluated = check_evaluation(reopened, ids)
        again = reopened.validators.run('collision')
        assert again['collision'] == native['collision']
        def signatures(e):
            return sorted((g['definition_id'], g['feature_id'], g['result_fingerprint']) for g in e['geometry'])
        assert signatures(evaluation) == signatures(reevaluated)
        # Bind the supplementary world geometry to actual document occurrence transforms.
        state = reopened.state
        occurrences = {o['id']: o for o in state['occurrences']}
        for part in PARTS:
            occurrence = occurrences[ids[part['name']]['occurrence_id']]
            assert occurrence['name'] == part['name'] and occurrence['transform'] == [1, 0, 0, part['operation']['translation_mm'][0], 0, 1, 0, part['operation']['translation_mm'][1], 0, 0, 1, part['operation']['translation_mm'][2], 0, 0, 0, 1]
    proof = {'model': str(output), 'canonical_digest': saved['state']['canonical_digest'],
             'part_count': len(PARTS), 'pair_count': expected_pairs, 'native_validator': native,
             'supplementary_validator': {'method': 'convex-polyhedron SAT; straight-sided prisms only',
                'tolerance_mm': TOLERANCE_MM, 'self_tests_passed': True, 'checked_pair_count': expected_pairs,
                'positive_volume_intersections': clashes, 'resolved_envelope_candidates': resolved,
                'scope': 'Authored convex planar prisms; not a general OCCT Boolean or structural certificate'},
             'reopen_same_digest_geometry_and_findings': True, 'evaluation': evaluation,
             'parts': PARTS, 'ids': ids}
    report_path.write_text(json.dumps(proof, indent=2, allow_nan=False), encoding='utf-8')
    print(json.dumps({'model': str(output), 'report': str(report_path), 'parts': len(PARTS),
                      'checked_pairs': expected_pairs, 'native_candidates': len(resolved),
                      'real_intersections': len(clashes), 'fresh_process_reopen': 'passed'}), flush=True)


if __name__ == '__main__':
    main()
