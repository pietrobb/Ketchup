# Ketchup rule library.
#
# Domain helpers built only from the generic builtins:
#   param(name, default, min=, max=, doc=)
#   box(name, size, at=, material=, grain=, color=, tool=)  -> part
#   extrude(name, profile=, distance=, at=, tool=), revolve(name, profile=, axis=, angle=, at=, tool=)
#   sweep(name, profile=, path=, bend=, at=, tool=), loft(name, sections=, at=, tool=)
#   rotate(part, axis=(x, y, z), angle=degrees, pivot=(x, y, z))  -> part
#   place(part, origin=(x, y, z), z=(x, y, z), x=(x, y, z))  -> part
#   part_info(part)  -> struct(name, size, at, min, max, x, y, z)
#   subtract(part, tool, name=), intersect(part, tool, name=)  -> part
#     `tool` is a helper body made with box/extrude/revolve/sweep/loft(..., tool=True)
#     or another real part, taken as it is at the call. Booleans come after
#     the part's other features. A part never collides with a part it
#     subtracted, with a part lying inside one box tool it subtracted (a notch,
#     a trim), nor with parts outside the tools it was intersected with.
#   math.sqrt/sin/cos/tan/asin/acos/atan/atan2/hypot/radians/degrees, math.pi
#   hole(part, face, at=(u, v) | world=(x, y, z), diameter=, depth=, id=)
#   pocket(part, face, rect=(u_min, v_min, u_max, v_max), depth=, id=)
#   contact(a, b)  -> struct(axis, face_a, face_b, min, max, normal, u, v,
#                    origin, size, points) or None; works for rotated parts
#   joint(a, b, kind=, fasteners=, fastener=, volume=, name=)
#
# Placing parts by relation instead of computing coordinates. A face is named
# in the target's own frame (so it follows a rotated target: a seat revolved
# and turned upright has its bottom at "y-"), or given as a world direction
# (0, 0, -1). Rotated parts are measured by their real extent:
#   on(part, target, face="z+", gap=0, align=None, center=None)  -> part
#     rests part against target's face from outside; align="y+" or ["x-", "y+"]
#     also makes it flush with those target faces, center="x" / "xy" centres it
#   flush(part, target, face, offset=0)  -> part: level with target's face
#   center_on(part, target, axes="xy")  -> part
#   between(part, a, b, face=None)  -> part: centred in the gap from a to b
#   move(part, by=(x, y, z))  -> part: shift by a world vector
#   face_normal(part, face)  -> world outward normal of a face
#   reach(part, direction)  -> how far the part reaches along a world direction
#   distance(a, b)  -> struct(distance, overlap, touching, direction)
#     direction points from a to b; overlap is how deep they intersect
#   nearest(part, among=None)  -> struct(name, distance, overlap, touching,
#     direction) of the closest other part, or None
#   e.g. on(back, seat, align="y+", center="x") stands a tilted back on the
#   seat, flush with its back edge and centred left to right;
#   on(leg, top, face="z-", align=["x-", "y-"]) puts a leg under a corner.
# They measure the body before cuts, finishes and booleans; distance() and
# nearest() use each part's box in its own frame (exact for box parts).
#
# Profile parts. `profile` is a closed loop in the part's local XY plane:
# either points [[x, y], ...] (faces become "segment1", "segment2", ...) or
# named segments [["name", [x0, y0], [x1, y1]], ...]. A named segment with a
# fourth item is a circular arc from start to end: {"through": (x, y)} (a
# point on the arc), {"radius": r, "clockwise": False, "large": False} or
# {"center": (x, y), "clockwise": False}; e.g. an arched apron bottom
# ["arch", [600, 0], [0, 0], {"through": (300, 60)}]. A full circle is two
# arcs. round_corners(points, radius, names=None) turns a point loop into
# named segments with every corner rounded by a tangent arc "corner<i+1>"
# (radius: one number or one per corner, 0 = sharp), e.g.
# extrude("top", profile=round_corners([[0,0],[800,0],[800,500],[0,500]], 40),
# distance=18). Arcs are exact in the solid; extrude() pads it along
# local +z; revolve() turns it around `axis` = [[x0, y0], [x1, y1]] in that
# plane. Faces of a profile part: each segment's name, the caps "start"
# and "end" (extrude: z = 0 and z = distance; revolve: only when angle < 360),
# a face left by a cut "cut_name.segment",
# and a face split in two by a cut "name#1", "name#2" (ordered by x, y, z).
# The four operations below work only on extrude()/revolve() parts, not box().
# Whatever the call order, a part applies all cuts, then fillets/chamfers,
# then push_pulls, then subtract/intersect. So fillet/chamfer name faces of
# the uncut profile, and a fillet on a face that a cut splits fails. A
# push_pull of a face that touches a fillet also fails for now.
#   fillet(part, edges=[[face_a, face_b], ...], radius=, name=)
#     rounds each edge where the two named faces meet, e.g.
#     fillet(top, edges=[["front", "end"], ["right", "end"]], radius=3)
#   chamfer(part, edges=[[face_a, face_b], ...], distance=, name=)
#     bevels the same way, e.g. chamfer(ring, edges=[["top", "outer"]], distance=2)
#   cut(part, profile=, depth=, name=)
#     removes a closed profile (same form as above, local XY) from the "end"
#     cap down by `depth`; let the profile overshoot the part to cut through
#     an edge, e.g. cut(board, profile=[["in", [150, -1], [158, -1]],
#     ["right", [158, -1], [158, 301]], ["out", [158, 301], [150, 301]],
#     ["left", [150, 301], [150, -1]]], depth=6, name="groove")
#   push_pull(part, face=, distance=, name=)
#     moves one named planar face along its outward normal (negative = in),
#     e.g. push_pull(board, face="end#2", distance=10, name="raise right half")
#
# Swept and lofted parts (curved rails, bent tubes, tapered legs). Both take
# at= and tool=True like extrude(), and subtract()/intersect() in either role;
# fillet/chamfer/cut/push_pull do not apply to them.
#   sweep(name, profile=, path=, bend=None, at=, tool=)
#     carries a closed profile along a smooth path in the part's frame. path
#     is 3D points [(x, y, z), ...] whose corners bend=r rounds with tangent
#     arcs (a corner without bend is refused), or segments [start, end] and
#     [start, end, {"through": (x, y, z)}] / [start, end, {"center": c,
#     "normal": n}] (counter-clockwise about n) that must join tangentially.
#     The profile (u, v) stands square to the path at its start: going
#     horizontally, u points right of the direction of travel and v up (+z);
#     starting up along +z, u is -x and v is +y. The profile is carried
#     without twist, e.g. a 30 x 20 rail turning a 100 mm bend:
#     sweep("rail", profile=[(-15, 0), (15, 0), (15, 20), (-15, 20)],
#           path=[(0, 0, 0), (600, 0, 0), (600, 400, 0)], bend=100)
#   loft(name, sections=[(profile, z), ...], at=, tool=)
#     a solid through 2 to 16 closed profiles, each in the local XY plane at
#     height z (strictly increasing), e.g. a leg tapering 40 -> 24 mm:
#     loft("leg", sections=[([(0,0),(40,0),(40,40),(0,40)], 0),
#                           ([(8,8),(32,8),(32,32),(8,32)], 700)])
#   Reach, bounds and on()/distance() measure a sweep exactly; a loft through
#   more than two sections may bulge slightly past them. Collisions of
#   either are decided on the exact solids.
#
# Stating intent. Each helper records a condition that is measured on the
# final model (after every move), so write them anywhere; one that does not
# hold is an `expectation_failed` error naming the measured and required mm.
# Faces follow the same rule as on(): the target's own frame, or a world
# direction. Measures use the body before cuts and booleans.
#   expect_contact(part, target, face=None, tolerance=0.1)  touching, not
#     apart or overlapping; with face, face to face against that target face
#   expect_gap(part, target, mm, tolerance=0.1)  clearance of mm between them
#   expect_flush(part, target, face, tolerance=0.1)  level with target's face
#   expect_symmetric(a, b, about, axis="x", tolerance=0.1)  a and b mirror
#     each other about the middle of `about` across its own axis
#   expect_inside(part, container, tolerance=0.1)  within the container
# They are built on one generic builtin:
#   expect(name, terms=[(coefficient, measure), ...], op="==", value=0,
#          tolerance=0.1, unit="mm", hint=)
#     measure = ("reach", part, direction) | ("distance", a, b) (negative
#     when overlapping) | ("contact_area", a, b[, face_of_a]); direction =
#     (x, y, z) or (part, "z+"); op is "==", "<=", ">=" or ">", e.g.
#     expect("seat height", terms=[(1, ("reach", seat, (0, 0, 1)))], value=450)
#
# Every part has its own frame: `at` is its local origin in world, and
# rotate()/place() turn that frame. Sizes, faces, holes and pockets are always
# in the part's own frame, so they follow the part when it is rotated.
# part_info().min/max are world bounds. contact() and dowels() work between
# any faces that lie flat against each other, rotated or not.
#
# Faces are "x-", "x+", "y-", "y+", "z-", "z+" in the part's own frame.
# Face coordinates (u, v): z faces use (x, y), x faces use (y, z), y faces
# use (x, z), measured from the part's minimum corner.
#
# Adding a helper here never requires a change in Rust.

AXES = {"x": 0, "y": 1, "z": 2}

def sum(values, start = 0):
    """Sum of numbers (Starlark has no built-in sum)."""
    total = start
    for value in values:
        total += value
    return total

def round_to(value, step = 1):
    """`value` rounded to the nearest multiple of `step`."""
    count = int(value / step + (0.5 if value >= 0 else -0.5))
    return count * step

# name: (diameter, length) in mm
DOWELS = {
    "6x30": (6, 30),
    "8x30": (8, 30),
    "8x35": (8, 35),
    "8x40": (8, 40),
    "10x40": (10, 40),
    "10x50": (10, 50),
}

def face_axes(face):
    """(normal axis, u axis, v axis) indices of a face name like "z+"."""
    axis = AXES[face[0]]
    if axis == 0:
        return (0, 1, 2)
    if axis == 1:
        return (1, 0, 2)
    return (2, 0, 1)

def board(name, size, at = (0, 0, 0), material = "board", grain = None, color = None):
    """A flat panel: an axis-aligned cuboid with a material for the cut list."""
    return box(name, size, at = at, material = material, grain = grain, color = color)

def vec_sub(a, b):
    return (a[0] - b[0], a[1] - b[1], a[2] - b[2])

def vec_add(a, b):
    return (a[0] + b[0], a[1] + b[1], a[2] + b[2])

def vec_scale(a, factor):
    return (a[0] * factor, a[1] * factor, a[2] * factor)

def vec_length(a):
    return math.sqrt(a[0] * a[0] + a[1] * a[1] + a[2] * a[2])

def plane_length(dx, dy):
    return math.sqrt(dx * dx + dy * dy)

def round_corners(points, radius, names = None):
    """Named segments of the closed point loop `points` with each corner
    rounded by a tangent arc. `radius` is one number or one per corner (0
    keeps it sharp). Sides are named names[i] or "segment<i+1>" (from
    points[i] to points[i+1]); the arc at points[i] is "corner<i+1>"."""
    count = len(points)
    radii = radius if type(radius) in ("list", "tuple") else [radius] * count
    if len(radii) != count or (names != None and len(names) != count):
        fail("round_corners: give one radius and one name per corner (%d)" % count)
    side = [names[i] if names != None else "segment%d" % (i + 1) for i in range(count)]
    cuts = []
    for i in range(count):
        p = points[i]
        a, b = points[(i - 1) % count], points[(i + 1) % count]
        la = plane_length(a[0] - p[0], a[1] - p[1])
        lb = plane_length(b[0] - p[0], b[1] - p[1])
        u1 = ((a[0] - p[0]) / la, (a[1] - p[1]) / la)
        u2 = ((b[0] - p[0]) / lb, (b[1] - p[1]) / lb)
        c = u1[0] * u2[0] + u1[1] * u2[1]
        r = radii[i]
        if r <= 0 or c <= -1 + 1e-9:
            cuts.append(None)
            continue
        if c >= 1 - 1e-9:
            fail("round_corners: corner %d folds back on itself" % (i + 1))
        t = r * math.sqrt((1 + c) / (1 - c))  # r / tan(half angle)
        d = r / math.sqrt((1 - c) / 2)  # r / sin(half angle)
        bis = (u1[0] + u2[0], u1[1] + u2[1])
        lbis = plane_length(bis[0], bis[1])
        turn = (p[0] - a[0]) * (b[1] - p[1]) - (p[1] - a[1]) * (b[0] - p[0])
        cuts.append(dict(
            t = t,
            first = [p[0] + u1[0] * t, p[1] + u1[1] * t],
            second = [p[0] + u2[0] * t, p[1] + u2[1] * t],
            center = (p[0] + bis[0] / lbis * d, p[1] + bis[1] / lbis * d),
            clockwise = turn < 0,
        ))
    segments = []
    for i in range(count):
        j = (i + 1) % count
        start = cuts[i]["second"] if cuts[i] != None else list(points[i])
        end = cuts[j]["first"] if cuts[j] != None else list(points[j])
        used = (cuts[i]["t"] if cuts[i] != None else 0) + (cuts[j]["t"] if cuts[j] != None else 0)
        length = plane_length(points[j][0] - points[i][0], points[j][1] - points[i][1])
        if used > length + 1e-6:
            fail("round_corners: radii at corners %d and %d need %s mm but side %r is %s mm" %
                 (i + 1, j + 1, used, side[i], length))
        if used < length - 1e-6:
            segments.append([side[i], start, end])
        if cuts[j] != None:
            segments.append(["corner%d" % (j + 1), cuts[j]["first"], cuts[j]["second"],
                             {"center": cuts[j]["center"], "clockwise": cuts[j]["clockwise"]}])
    return segments

def face_normal(part, face):
    """World outward normal of `face` ("x-" ... "z+") in the part's own frame,
    or a world direction (x, y, z) given as it is."""
    if type(face) != "string":
        return vec_scale(face, 1.0 / vec_length(face))
    if len(face) != 2 or face[0] not in AXES or face[1] not in "+-":
        fail("face must be one of x-, x+, y-, y+, z-, z+ or a world direction; got %r" % face)
    axis = getattr(part_info(part), face[0])
    return axis if face[1] == "+" else vec_scale(axis, -1)

def _middle(part, direction):
    return (reach(part, direction) - reach(part, vec_scale(direction, -1))) / 2.0

def _names(value):
    if value == None:
        return []
    return [value] if type(value) == "string" else value

def flush(part, target, face, offset = 0):
    """Moves `part` along the normal of `target`'s `face` until its outermost
    point that way is level with that face, `offset` mm inside it (negative:
    sticking out). Other directions are kept."""
    n = face_normal(target, face)
    return move(part, by = vec_scale(n, reach(target, n) - offset - reach(part, n)))

def center_on(part, target, axes = "xy"):
    """Centres `part` on `target` along each of `target`'s own axes named in
    `axes` (e.g. "x", "xy", "xyz"). Other directions are kept."""
    info = part_info(part)
    for letter in axes.elems():
        d = face_normal(target, letter + "+")
        info = move(part, by = vec_scale(d, _middle(target, d) - _middle(part, d)))
    return info

def on(part, target, face = "z+", gap = 0, align = None, center = None):
    """Moves `part` along the normal of `target`'s `face` so it rests against
    that face from outside (`gap` mm away), e.g. on(back, seat) stands the
    back on the seat, on(leg, top, face="z-") hangs a leg under a top.
    `align` = a face name or list of `target` faces to make `part` flush with
    (see flush), `center` = `target` axes to centre on (see center_on).
    Rotated parts rest on their outermost point, so a tilted back touches the
    seat with its lowest edge."""
    n = face_normal(target, face)
    info = move(part, by = vec_scale(n, reach(target, n) + gap + reach(part, vec_scale(n, -1))))
    for other in _names(align):
        info = flush(part, target, other)
    if center != None:
        info = center_on(part, target, center)
    return info

def between(part, a, b, face = None):
    """Centres `part` in the gap between `a` and `b`, measured along the
    normal of `a`'s `face` that looks towards `b` (picked automatically when
    None). Other directions are kept."""
    if face == None:
        ia, ib = part_info(a), part_info(b)
        towards = vec_sub(vec_add(ib.min, ib.max), vec_add(ia.min, ia.max))
        best = None
        for letter in "xyz".elems():
            axis = getattr(ia, letter)
            along = axis[0] * towards[0] + axis[1] * towards[1] + axis[2] * towards[2]
            if best == None or abs(along) > best[0]:
                best = (abs(along), letter + ("+" if along >= 0 else "-"))
        face = best[1]
    n = face_normal(a, face)
    low = reach(a, n)
    high = -reach(b, vec_scale(n, -1))
    return move(part, by = vec_scale(n, (low + high) / 2.0 - _middle(part, n)))

def member(name, start, end, section, across = None, material = "timber", grain = "z", color = None):
    """A straight bar of rectangular `section` (width, height) whose centre
    line runs from world point `start` to `end` (legs, rails, struts).

    The bar's local z runs along its length (z- face at `start`, z+ at `end`);
    local x is `across` made perpendicular to the length (default: world x, or
    world y when the bar runs along x). Ends are square; trim them with
    booleans when they must follow another surface.
    """
    direction = vec_sub(end, start)
    length = vec_length(direction)
    if length <= 0:
        fail("member(%r): start and end must differ" % name)
    if across == None:
        across = (0, 1, 0) if abs(direction[0]) > 0.9 * length else (1, 0, 0)
    part = box(name, (section[0], section[1], length), material = material, grain = grain, color = color)
    placed = place(part, origin = start, z = direction, x = across)
    offset = vec_add(vec_scale(placed.x, -section[0] / 2.0), vec_scale(placed.y, -section[1] / 2.0))
    return place(part, origin = vec_add(start, offset), z = direction, x = across)

def trim(part, point, normal, name = None):
    """Cuts `part` off at the plane through world `point`, removing everything
    on the side `normal` points to (normal (0, 0, -1) at z = 0 trims at the
    floor; the underside of a seat trims legs with normal (0, 0, 1))."""
    info = part_info(part)
    n = vec_scale(normal, 1.0 / vec_length(normal))
    centre = vec_scale(vec_add(info.min, info.max), 0.5)
    reach = vec_length(vec_sub(info.max, info.min)) + abs(vec_length(vec_sub(centre, point)))
    size = 4 * reach + 1
    along = (centre[0] - point[0]) * n[0] + (centre[1] - point[1]) * n[1] + (centre[2] - point[2]) * n[2]
    foot = vec_sub(centre, vec_scale(n, along))
    if name == None:
        name = "trim %s %s" % (tuple([round_to(v, 0.001) for v in point]), tuple([round_to(v, 0.001) for v in n]))
    across = (0, 1, 0) if abs(n[0]) > 0.9 else (1, 0, 0)
    tool_name = "%s/%s" % (info.name, name)
    tool = box(tool_name, (size, size, size), tool = True)
    placed = place(tool, origin = foot, z = n, x = across)
    corner = vec_add(foot, vec_add(vec_scale(placed.x, -size / 2.0), vec_scale(placed.y, -size / 2.0)))
    place(tool, origin = corner, z = n, x = across)
    return subtract(part, tool, name = name)

def spread(start, end, count):
    """`count` evenly spaced positions from start to end (inclusive)."""
    if count < 1:
        fail("spread(): count must be at least 1, got %s" % count)
    if count == 1:
        return [(start + end) / 2.0]
    step = (end - start) / (count - 1)
    return [start + step * i for i in range(count)]

def dowels(a, b, dowel = "8x30", count = None, margin = 50, spacing = 250, clearance = 1):
    """Dowels a and b along the face where they touch.

    Holes are drilled into both parts from the shared face, so moving a part
    or changing `margin`/`count` moves the holes in both. Hole depth in each
    part is half the dowel length plus `clearance`.
    """
    if dowel not in DOWELS:
        fail("dowels(): unknown dowel %r; use one of %s" % (dowel, sorted(DOWELS.keys())))
    diameter, length = DOWELS[dowel]
    c = contact(a, b)
    if c == None:
        fail("dowels(%s, %s): the parts do not touch; place them face to face first" % (part_info(a).name, part_info(b).name))
    if c.size[0] >= c.size[1]:
        row, across, row_length, width = c.u, c.v, c.size[0], c.size[1]
    else:
        row, across, row_length, width = c.v, c.u, c.size[1], c.size[0]
    if row_length < 2 * margin:
        fail("dowels(%s, %s): the contact is %s mm long, shorter than twice the margin (%s mm)" % (part_info(a).name, part_info(b).name, row_length, margin))
    if count == None:
        count = max(2, 1 + int((row_length - 2 * margin) / spacing))
    start = vec_add(c.origin, vec_scale(across, width / 2.0))
    points = [vec_add(start, vec_scale(row, s)) for s in spread(margin, row_length - margin, count)]
    depth = length / 2.0 + clearance
    for i, point in enumerate(points):
        hole(a, c.face_a, world = point, diameter = diameter, depth = depth, id = "dowel:%s:%d" % (part_info(b).name, i + 1))
        hole(b, c.face_b, world = point, diameter = diameter, depth = depth, id = "dowel:%s:%d" % (part_info(a).name, i + 1))
    joint(a, b, kind = "dowel", fasteners = points, fastener = "dowel " + dowel)
    return points

def groove(part, face, along, width, depth, offset, margin = 0):
    """A groove across the whole face, parallel to axis `along` ("x"/"y"/"z").

    `offset` is where the groove starts on the other in-plane axis, measured
    from the part's minimum corner; `margin` keeps it short of both ends
    (0 = runs out at both ends).
    """
    info = part_info(part)
    normal, u_axis, v_axis = face_axes(face)
    along_axis = AXES[along]
    if along_axis == normal:
        fail("groove(): `along` must lie in the face %s" % face)
    run_min = margin
    run_max = info.size[along_axis] - margin
    if along_axis == u_axis:
        rect = (run_min, offset, run_max, offset + width)
    else:
        rect = (offset, run_min, offset + width, run_max)
    pocket(part, face, rect = rect, depth = depth)

def rabbet(part, face, edge, width, depth):
    """A rebate along one edge of a face. `edge` is "u-", "u+", "v-" or "v+"."""
    info = part_info(part)
    normal, u_axis, v_axis = face_axes(face)
    u_size = info.size[u_axis]
    v_size = info.size[v_axis]
    if edge == "u-":
        rect = (0, 0, width, v_size)
    elif edge == "u+":
        rect = (u_size - width, 0, u_size, v_size)
    elif edge == "v-":
        rect = (0, 0, u_size, width)
    elif edge == "v+":
        rect = (0, v_size - width, u_size, v_size)
    else:
        fail("rabbet(): edge must be u-, u+, v- or v+")
    pocket(part, face, rect = rect, depth = depth)

def hole_row(part, face, start, step, count, diameter, depth, direction = "u"):
    """A row of equal holes (shelf-pin rows, System 32). `start` is (u, v)."""
    for i in range(count):
        if direction == "u":
            at = (start[0] + step * i, start[1])
        else:
            at = (start[0], start[1] + step * i)
        hole(part, face, at = at, diameter = diameter, depth = depth)

def divide(span, count, round_to = 1):
    """Split `span` into `count` fields rounded to `round_to`, spreading the
    remainder over the first fields. Returns the field lengths."""
    base = int(span / count / round_to) * round_to
    remainder = span - base * count
    extra = int(remainder / round_to + 0.5)
    return [base + (round_to if i < extra else 0) for i in range(count)]

def _name(part):
    return part if type(part) == "string" else part.name

def _direction(part, face):
    """A face of `part` (followed to the end of the program) or a world direction."""
    return (part, face) if type(face) == "string" else face

def _reverse(direction):
    if type(direction) == "tuple" and len(direction) == 2:
        face = direction[1]
        return (direction[0], face[0] + ("-" if face[1] == "+" else "+"))
    return vec_scale(direction, -1)

def expect_contact(part, target, face = None, tolerance = 0.1, name = None):
    """`part` touches `target`, neither apart nor overlapping; with `face`
    (a face of target), face to face against that face."""
    label = name or "%s touches %s" % (_name(part), _name(target))
    expect(label, terms = [(1, ("distance", part, target))], tolerance = tolerance,
           hint = "Positive: move them together (on()); negative: they overlap, move them apart.")
    if face != None:
        expect(label + " on " + face, terms = [(1, ("contact_area", target, part, face))],
               op = ">", tolerance = 0, unit = "mm²",
               hint = "They do not lie face to face on that face; place with on(part, target, face=...).")

def expect_gap(part, target, mm, tolerance = 0.1, name = None):
    """A clearance of `mm` between `part` and `target`."""
    expect(name or "%s is %s mm from %s" % (_name(part), mm, _name(target)),
           terms = [(1, ("distance", part, target))], value = mm, tolerance = tolerance,
           hint = "Move one part by the difference (on(..., gap=mm) sets it directly).")

def expect_flush(part, target, face, tolerance = 0.1, name = None):
    """`part` reaches exactly as far as `target`'s `face` (level with it)."""
    d = _direction(target, face)
    expect(name or "%s flush with %s %s" % (_name(part), _name(target), face),
           terms = [(1, ("reach", part, d)), (-1, ("reach", target, d))], tolerance = tolerance,
           hint = "Positive: part sticks out past the face; negative: it stops short. flush() aligns it.")

def expect_symmetric(a, b, about, axis = "x", tolerance = 0.1, name = None):
    """`a` and `b` mirror each other about the middle of `about` across its
    own `axis` ("x", "y", "z") or a world direction: equally far from the
    middle on opposite sides, and equally wide that way."""
    label = name or "%s and %s symmetric about %s" % (_name(a), _name(b), _name(about))
    d = _direction(about, axis + "+") if type(axis) == "string" else axis
    m = _reverse(d)
    expect(label, terms = [(0.5, ("reach", a, d)), (-0.5, ("reach", a, m)),
                           (0.5, ("reach", b, d)), (-0.5, ("reach", b, m)),
                           (-1, ("reach", about, d)), (1, ("reach", about, m))],
           tolerance = tolerance,
           hint = "The number is twice how far the pair's middle lies off the middle of `about`; center one part or move both.")
    expect(label + " (width)", terms = [(1, ("reach", a, d)), (1, ("reach", a, m)),
                                        (-1, ("reach", b, d)), (-1, ("reach", b, m))],
           tolerance = tolerance, hint = "a is wider than b by this much across the axis.")

def expect_inside(part, container, tolerance = 0.1, name = None):
    """`part` lies within `container` (its box in its own frame)."""
    label = name or "%s inside %s" % (_name(part), _name(container))
    for face in ["x-", "x+", "y-", "y+", "z-", "z+"]:
        d = (container, face)
        expect("%s at %s" % (label, face), terms = [(1, ("reach", part, d)), (-1, ("reach", container, d))],
               op = "<=", tolerance = tolerance,
               hint = "The part sticks out of that face of the container by this much.")
