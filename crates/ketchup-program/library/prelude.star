# Ketchup rule library.
#
# Helpers written only from the generic builtins; adding one never requires a
# change in Rust. The file is split into topics by "#@topic id: title" lines:
# KetchupDiscover section=program lists the topics with their helpers, and
# section=program name=<id> returns one topic (docs and code).

#@topic basics: Parts, frames, faces, parameters, numbers
#
#   param(name, default, min=, max=, doc=)  -> number the user can override
#   box(name, size, at=, material=, grain=, color=, tool=)  -> part
#   board(name, size, at=, material=, grain=, color=)  -> a panel for the cut list
#   member(name, start, end, section, across=)  -> a bar from point to point
#   rotate(part, axis=(x, y, z), angle=degrees, pivot=(x, y, z))  -> part
#   place(part, origin=(x, y, z), z=(x, y, z), x=(x, y, z))  -> part
#   part_info(part)  -> struct(name, size, at, min, max, local_min, local_max,
#     x, y, z): min/max in world, local_min/local_max in the part's own frame
#   math.sqrt/sin/cos/tan/asin/acos/atan/atan2/hypot/radians/degrees, math.pi
#   sum(values), round_to(value, step), spread(start, end, count),
#   divide(span, count, round_to=), vec_add/vec_sub/vec_scale/vec_length
#   print(...) lines come back in the report's log.
#
# Every part has its own frame: `at` is its local origin in world, and
# rotate()/place() turn that frame. Sizes, faces, holes and pockets are always
# in the part's own frame, so they follow the part when it is rotated.
# part_info().min/max are world bounds. contact() and dowels() work between
# any flat faces that lie against each other, on any body, rotated or not.
#
# Faces are "x-", "x+", "y-", "y+", "z-", "z+" in the part's own frame.
# Face coordinates (u, v): z faces use (x, y), x faces use (y, z), y faces
# use (x, z), measured from the part's minimum corner.
#
# Any part's faces can be looked up by name: a box has "x-" ... "z+", an
# extruded or revolved profile "start", "end" and its segment names, a face a
# boolean left "<operation>.<tool face>", a shell's inner wall "<shell>.<face>".
# They follow push_pull, mirror, rotate and place:
#   face(part, name)  -> struct(part, name, kind, origin, normal, u, v, min,
#     max, radius, center) in world. kind "planar": the points
#     origin + a*u + b*v for (a, b) from min to max, normal outward.
#     kind "cylindrical": the points origin + b*v + radius*(cos a*u +
#     sin a*(v x u)), a in degrees from min[0] to max[0]; normal is the
#     outward normal at a = 0 (u on a round outside, -u in a bore)
#   faces(part)  -> every flat or cylindrical face, like face()
#   face_at(part, point)  -> the face a world point lies on, or None
#

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

def spread(start, end, count):
    """`count` evenly spaced positions from start to end (inclusive)."""
    if count < 1:
        fail("spread(): count must be at least 1, got %s" % count)
    if count == 1:
        return [(start + end) / 2.0]
    step = (end - start) / (count - 1)
    return [start + step * i for i in range(count)]

def divide(span, count, round_to = 1):
    """Split `span` into `count` fields rounded to `round_to`, spreading the
    remainder over the first fields. Returns the field lengths."""
    base = int(span / count / round_to) * round_to
    remainder = span - base * count
    extra = int(remainder / round_to + 0.5)
    return [base + (round_to if i < extra else 0) for i in range(count)]

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

#@topic placement: Placing parts by relation, distances and contact
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
#   distribute(parts, a, b, face=None)  -> clear gap: spaces several parts
#     between a and b with equal gaps (shelves into equal compartments)
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
#   contact(a, b)  -> struct(axis, face_a, face_b, min, max, normal, u, v,
#                    origin, size, points) or None
#     the largest patch where a flat face of `a` lies against a flat face of
#     `b`, for any bodies (boxes, extrusions, revolves, cut or mirrored,
#     rotated or not); face_a/face_b are face names as in faces(), e.g.
#     "x+", "segment2" or "<boolean>.z+" for the face a cut left

def face_normal(part, face):
    """World outward normal of the part's face named `face` (see faces(); on
    a round face its normal at angle 0), or a world direction (x, y, z) given
    as it is. A part without named faces (swept, lofted) takes "x-" ... "z+"
    as the faces of its box."""
    if type(face) != "string":
        return vec_scale(face, 1.0 / vec_length(face))
    named = [f for f in faces(part) if f.name == face]
    if named:
        return named[0].normal
    if len(face) != 2 or face[0] not in AXES or face[1] not in "+-":
        fail("face %r does not exist on %s; its faces are %s, or give a world direction" %
             (face, part_info(part).name, [f.name for f in faces(part)]))
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
    n = face_normal(a, face or _face_towards(a, b))
    low = reach(a, n)
    high = -reach(b, vec_scale(n, -1))
    return move(part, by = vec_scale(n, (low + high) / 2.0 - _middle(part, n)))

def _face_towards(a, b):
    """The face of `a` whose normal points most directly at `b`."""
    ia, ib = part_info(a), part_info(b)
    towards = vec_sub(vec_add(ib.min, ib.max), vec_add(ia.min, ia.max))
    best = None
    for letter in "xyz".elems():
        axis = getattr(ia, letter)
        along = axis[0] * towards[0] + axis[1] * towards[1] + axis[2] * towards[2]
        if best == None or abs(along) > best[0]:
            best = (abs(along), letter + ("+" if along >= 0 else "-"))
    return best[1]

def distribute(parts, a, b, face = None):
    """Spaces `parts` (in the order given) between `a` and `b` so that every
    clear gap along the normal of `a`'s `face` towards `b` is equal: the
    gap from a to the first part, between the parts, and from the last to b.
    E.g. distribute([shelf1, shelf2], bottom, top) makes three equal
    compartments. Other directions are kept. Returns the clear gap in mm."""
    n = face_normal(a, face or _face_towards(a, b))
    back = vec_scale(n, -1)
    low = reach(a, n)
    high = -reach(b, back)
    thickness = sum([reach(part, n) + reach(part, back) for part in parts])
    gap = (high - low - thickness) / (len(parts) + 1.0)
    if gap < 0:
        fail("distribute(): the parts are %s mm thick together but only %s mm lie between %s and %s" %
             (_mm(thickness), _mm(high - low), part_info(a).name, part_info(b).name))
    position = low + gap
    for part in parts:
        size = reach(part, n) + reach(part, back)
        move(part, by = vec_scale(n, position + reach(part, back)))
        position += size + gap
    return gap

#@topic profiles: Profile, revolved, swept and lofted parts; arcs, fillet, chamfer, cut, push_pull
#
#   extrude(name, profile=, distance=, at=, tool=)
#   revolve(name, profile=, axis=, angle=, at=, tool=)
#
# Profile parts. `profile` is a closed loop in the part's local XY plane:
# either points [[x, y], ...] (faces become "segment1", "segment2", ...) or
# named segments [["name", [x0, y0], [x1, y1]], ...]. A named segment with a
# fourth item is a circular arc from start to end: {"through": (x, y)} (a
# point on the arc), {"radius": r, "clockwise": False, "large": False} or
# {"center": (x, y), "clockwise": False}; e.g. an arched apron bottom
# ["arch", [600, 0], [0, 0], {"through": (300, 60)}]. A full circle is two
# arcs; it becomes one round face, so either arc's name refers to all of it.
# round_corners(points, radius, names=None) turns a point loop into
# named segments with every corner rounded by a tangent arc "corner<i+1>"
# (radius: one number or one per corner, 0 = sharp), e.g.
# extrude("top", profile=round_corners([[0,0],[800,0],[800,500],[0,500]], 40),
# distance=18). polygon(sides, radius, center=(0, 0), angle=0) is a regular
# polygon's corner points on a circle of `radius` (faces "segment1", ...);
# ellipse(rx, ry, center=(0, 0), angle=0, name="side") is an ellipse as four
# named quarter curves "side1" ... "side4" (cubic Bezier, within 0.03 % of
# the true ellipse). A named segment whose fourth item is {"controls":
# [(x1, y1), (x2, y2)]} is such a cubic curve from start to end.
# Arcs are exact in the solid; extrude() pads it along
# local +z; revolve() turns it around `axis` = [[x0, y0], [x1, y1]] in that
# plane. Faces of a profile part: each segment's name, the caps "start"
# and "end" (extrude: z = 0 and z = distance; revolve: only when angle < 360),
# a face left by a cut "cut_name.segment",
# and a face split in two by a cut "name#1", "name#2" (ordered by x, y, z).
# The four operations below work on box() (faces x-, x+, y-, y+, z-, z+) and
# extrude()/revolve() parts. Every operation of a part (these four, trim,
# subtract, intersect) applies in the order it is written, so a fillet after
# a cut or subtract rounds the faces they left: "<operation name>.<tool face>"
# (trim: "<trim name>.z-" is the new flat face), a split face "name#1", and a
# rounded edge "fillet(a,b)" / "chamfer(a,b)". A radius the faces next to an
# edge cannot hold is refused with the amount named.
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

def _turn(point, center, angle):
    """`point` (x, y) turned by `angle` degrees about the origin, then moved to `center`."""
    c, s = math.cos(math.radians(angle)), math.sin(math.radians(angle))
    return (center[0] + c * point[0] - s * point[1], center[1] + s * point[0] + c * point[1])

def polygon(sides, radius, center = (0, 0), angle = 0):
    """Corner points of a regular polygon with `sides` corners on a circle of
    `radius` around `center`, the first corner `angle` degrees from local +x.
    Its faces are "segment1" (first to second corner) ... "segment<sides>"."""
    if type(sides) != "int" or sides < 3:
        fail("polygon(): sides must be a whole number of at least 3, got %r" % sides)
    if radius <= 0:
        fail("polygon(): radius must be positive, got %s" % radius)
    step = 360.0 / sides
    return [_turn((radius * math.cos(math.radians(step * i)), radius * math.sin(math.radians(step * i))),
                  center, angle) for i in range(sides)]

# 4/3 (sqrt(2) - 1): quarter-ellipse cubics through the axis ends.
_KAPPA = 0.5522847498307936

def ellipse(rx, ry, center = (0, 0), angle = 0, name = "side"):
    """Named segments of an ellipse with half-axes `rx` (along local x turned
    by `angle` degrees) and `ry` around `center`: four quarter curves
    "<name>1" ... "<name>4" counter-clockwise from the +x end."""
    if rx <= 0 or ry <= 0:
        fail("ellipse(): rx and ry must be positive, got %s and %s" % (rx, ry))
    ends = [(rx, 0), (0, ry), (-rx, 0), (0, -ry)]
    tangents = [(0, ry), (-rx, 0), (0, -ry), (rx, 0)]
    segments = []
    for i in range(4):
        j = (i + 1) % 4
        start, end = ends[i], ends[j]
        first = (start[0] + _KAPPA * tangents[i][0], start[1] + _KAPPA * tangents[i][1])
        second = (end[0] - _KAPPA * tangents[j][0], end[1] - _KAPPA * tangents[j][1])
        segments.append(["%s%d" % (name, i + 1), _turn(start, center, angle), _turn(end, center, angle),
                         {"controls": [_turn(first, center, angle), _turn(second, center, angle)]}])
    return segments

#@topic machining: Holes, pockets, grooves, rebates, trims and booleans
#
# Positions (u, v) are in the face's own coordinates (see basics). `face` is
# a face name or a face() value of any part: a hole goes into any flat or
# round face along its inward normal, a pocket into any flat face, before or
# after a mirror or boolean.
#   hole(part, face, at=(u, v) | world=(x, y, z), diameter=, depth=, id=)
#     on a round face at=(angle in degrees, v): a radial hole, see face()
#   pocket(part, face, rect=(u_min, v_min, u_max, v_max), depth=, id=)
#   pocket_shape(part, face, profile, depth, name=)  -> part: any closed
#     profile (points or named segments in (u, v)) milled into any face of any
#     part, e.g. pocket_shape(top, "y-", [(20, 5), (80, 5), (50, 35)], 6);
#     its walls are "<name>.<segment>", its floor "<name>.start" or ".end"
#   boss(part, face, profile, height, name=)  -> part: the same profile
#     standing out of the face, joined in as one solid
#   groove(part, face, along, width, depth, offset, margin=)
#   rabbet(part, face, edge, width, depth)
#   hole_row(part, face, start, step, count, diameter, depth, direction=)
#   countersunk_hole(part, face, at, diameter, depth, head, angle=90, name=)
#     -> part: a hole (u, v) = at with a cone of `head` diameter at the face
#     narrowing at the included `angle` down to `diameter`, for flat-head screws
#   circular_pattern(part, count, axis=(0, 0, 1), center=(0, 0, 0), angle=360,
#     names=None)  -> the count - 1 new copies of part turned about the axis
#     through `center`: evenly round a full turn, or spread over `angle`
#     (first and last included), named "<part> 2", "<part> 3", ... or names
#   trim(part, point, normal, name=)  cut off at a plane
#   split(part, point, normal, name=)  -> the new part: cut in two at a plane,
#     both halves stay (part against normal, the new part along it)
#   copy(part, name)  -> a new identical part
#   shell(part, thickness=, open=[faces], name=)  -> part: hollowed to walls
#     `thickness` thick inside it, open at the listed faces (at least one);
#     the inner wall along face F is "<name>.F"
#   mirror(part, axis="x", name=)  -> part: its solid reflected across its
#     own middle plane across local axis; faces keep their names, so later
#     steps find them where they now are
#   mirrored(part, name, point, normal)  -> a new part: the mirror image of
#     part across the world plane through `point` with `normal`
#   subtract(part, tool, name=)  -> part: part minus tool
#   intersect(part, tool, name=)  -> part: only what part and tool share
#   union(part, other, name=)  -> part: other joined in as one solid (they
#     must touch or overlap); other is no longer a separate part
#     `tool` is a helper body made with box/extrude/revolve/sweep/loft(..., tool=True)
#     or another real part, taken as it is at the call. Every operation
#     applies in the order written. A part never collides with a part it
#     subtracted, with a part lying inside one box tool it subtracted (a notch,
#     a trim), nor with parts outside the tools it was intersected with.

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

def countersunk_hole(part, face, at, diameter, depth, head, angle = 90, name = None):
    """Drills a `diameter` hole `depth` mm deep into `face` at face
    coordinates `at`, its mouth widened by a cone `head` mm across at the face
    whose sides meet at the included `angle` degrees. Returns `part`."""
    info = part_info(part)
    r, big = diameter / 2.0, head / 2.0
    if r <= 0 or depth <= 0:
        fail("countersunk_hole(%s): diameter and depth must be positive" % info.name)
    if big <= r:
        fail("countersunk_hole(%s): head %s must be wider than the hole %s" % (info.name, head, diameter))
    if angle <= 0 or angle >= 180:
        fail("countersunk_hole(%s): angle must lie between 0 and 180 degrees, got %s" % (info.name, angle))
    sink = (big - r) / math.tan(math.radians(angle / 2.0))
    if sink >= depth:
        fail("countersunk_hole(%s): the %s mm cone is deeper than the %s mm hole" % (info.name, _mm(sink), depth))
    if name == None:
        # Tool names become face-name prefixes: no "." "(" or ",".
        name = "countersink %s %s" % (face, " ".join([("%s" % round_to(v, 0.001)).replace(".", "_") for v in at]))
    # Profile (radius, height above the face); the part lies below height 0.
    profile = [(0, -depth), (r, -depth), (r, -sink), (big, 0), (big, 1), (0, 1)]
    tool = revolve("%s/%s" % (info.name, name), profile = profile, axis = [[0, 0], [0, 1]], tool = True)
    n_axis, u_axis, v_axis = face_axes(face)
    axes = (info.x, info.y, info.z)
    local = [info.local_min[0], info.local_min[1], info.local_min[2]]
    if face[1] == "+":
        local[n_axis] = info.local_max[n_axis]
    local[u_axis] += at[0]
    local[v_axis] += at[1]
    origin = info.at
    for axis in range(3):
        origin = vec_add(origin, vec_scale(axes[axis], local[axis]))
    normal = face_normal(part, face)
    # The revolve's axis is its local y: z x x must be the outward normal.
    across = axes[u_axis]
    x = (normal[1] * across[2] - normal[2] * across[1],
         normal[2] * across[0] - normal[0] * across[2],
         normal[0] * across[1] - normal[1] * across[0])
    place(tool, origin = origin, z = across, x = x)
    return subtract(part, tool, name = name)

def circular_pattern(part, count, axis = (0, 0, 1), center = (0, 0, 0), angle = 360, names = None):
    """`count` - 1 copies of `part` turned about the world `axis` through
    `center`: evenly spaced round a full turn when `angle` is 360, else
    spread from 0 to `angle` degrees. Returns the new copies."""
    info = part_info(part)
    if type(count) != "int" or count < 2:
        fail("circular_pattern(%s): count must be a whole number of at least 2, got %r" % (info.name, count))
    if names != None and len(names) != count - 1:
        fail("circular_pattern(%s): give %d names, one per copy" % (info.name, count - 1))
    if angle == 0 or abs(angle) > 360:
        fail("circular_pattern(%s): angle must be within (0, 360] degrees, got %s" % (info.name, angle))
    step = angle / float(count) if abs(angle) == 360 else angle / (count - 1.0)
    copies = []
    for i in range(1, count):
        other = copy(part, names[i - 1] if names != None else "%s %d" % (info.name, i + 1))
        copies.append(rotate(other, axis = axis, angle = step * i, pivot = center))
    return copies

def mirrored(part, name, point, normal):
    """A new part `name`: the mirror image of `part` across the world plane
    through `point` with direction `normal`."""
    info = part_info(part)
    length = vec_length(normal)
    if length <= 0:
        fail("mirrored(%s): normal must be a non-zero direction" % info.name)
    n = vec_scale(normal, 1.0 / length)
    reflect = lambda v: vec_sub(v, vec_scale(n, 2 * _dot(v, n)))
    k = _own_axis(info, n)[0]
    axes = [reflect(getattr(info, "xyz"[j])) for j in range(3)]
    flipped = axes[k]
    axes[k] = vec_scale(flipped, -1)
    middle = (info.local_min[k] + info.local_max[k]) / 2.0
    origin = vec_add(vec_add(reflect(vec_sub(info.at, point)), point), vec_scale(flipped, 2 * middle))
    other = copy(part, name)
    mirror(other, axis = "xyz"[k])
    return place(other, origin = origin, z = axes[2], x = axes[0])

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

def split(part, point, normal, name = None):
    """Cuts `part` in two at the plane through world `point`: `part` keeps
    the side against `normal`, a new part `name` (default "<part> 2") the
    side `normal` points to. Both new faces are "split.z-". Returns the new
    part; a plane that misses the part is refused."""
    info = part_info(part)
    n = vec_scale(normal, 1.0 / vec_length(normal))
    at = point[0] * n[0] + point[1] * n[1] + point[2] * n[2]
    high = reach(part, n)
    low = -reach(part, vec_scale(n, -1.0))
    if at <= low + 0.001 or at >= high - 0.001:
        fail("split(%s): the plane at %s along %s misses the part, which spans %s to %s" % (
            info.name, round_to(at, 0.001), tuple(n), round_to(low, 0.001), round_to(high, 0.001)))
    if name == None:
        name = info.name + " 2"
    other = copy(part, name)
    trim(part, point, n, name = "split")
    trim(other, point, vec_scale(n, -1.0), name = "split")
    return other

def _shape_tool(part, face, profile, start, length, name):
    """A tool prism of `profile` (face coordinates of `face`) covering `start`
    to `start + length` mm along the face's outward normal."""
    if type(face) != "string" or len(face) != 2 or face[0] not in AXES or face[1] not in "+-":
        fail("%s: face must be one of x-, x+, y-, y+, z-, z+; got %r" % (name, face))
    info = part_info(part)
    n_axis, u_axis, v_axis = face_axes(face)
    axes = (info.x, info.y, info.z)
    local = [info.local_min[0], info.local_min[1], info.local_min[2]]
    sign = 1.0
    if face[1] == "+":
        local[n_axis] = info.local_max[n_axis]
    else:
        sign = -1.0
    origin = info.at
    for axis in range(3):
        origin = vec_add(origin, vec_scale(axes[axis], local[axis]))
    normal = vec_scale(axes[n_axis], sign)
    # The prism runs along u x v, which is the outward normal on x+, y- and
    # z+ faces and the inward one on the others.
    along = 1.0 if (n_axis == 1) != (sign > 0) else -1.0
    base = start if along > 0 else start + length
    tool = extrude("%s/%s" % (info.name, name), profile = profile, distance = length, tool = True)
    place(tool, origin = vec_add(origin, vec_scale(normal, base)), z = vec_scale(normal, along), x = axes[u_axis])
    return tool

def pocket_shape(part, face, profile, depth, name = "pocket"):
    """Removes a closed `profile` (points or named segments in face
    coordinates (u, v) of `face`) `depth` mm deep into `part`; it may run off
    the face edges. Works on any part, rotated or not, and after any other
    operation. Returns `part`."""
    if depth <= 0:
        fail("pocket_shape(%s): depth must be positive, got %s" % (part_info(part).name, depth))
    return subtract(part, _shape_tool(part, face, profile, -depth, depth + 1, name), name = name)

def boss(part, face, profile, height, name = "boss"):
    """Adds a closed `profile` (face coordinates of `face`) standing `height` mm
    out of `face`, joined into `part` as one solid. Returns `part`."""
    if height <= 0:
        fail("boss(%s): height must be positive, got %s" % (part_info(part).name, height))
    return union(part, _shape_tool(part, face, profile, 0, height, name), name = name)

#@topic joinery: Joints, dowels and hardware
#
#   joint(a, b, kind=, fasteners=, fastener=, volume=, max_gap=0, name=)
#     records that a and b are joined (the part is carried, not floating);
#     fastener names are counted in the report's hardware list. The parts
#     must touch, or stay within max_gap mm (a door on hinges across its
#     reveal: joint(door, side, kind="hinge", max_gap=3)).
#   dowels(a, b, dowel="8x35", count=, margin=50, spacing=250, clearance=1.5, rest=6)
#     drills matching holes into two touching parts and records the joint.
#     Depths follow the parts: into the face of an 18 mm board at most 12 mm
#     (never nearer the far side than rest or a third of the thickness), the
#     rest of the dowel into the other part's edge, every hole clearance mm
#     longer than its dowel end; e.g. 8x35 side-to-shelf: 12 mm + 26 mm.
#   hinge(door, side, count=None, margin=None, cup=35, cup_depth=13,
#         cup_edge=4, setback=37, pitch=32, plate_hole=5, plate_depth=12,
#         max_gap=4, name=None)  -> world centres of the cups
#     concealed cup hinges on the door edge next to `side`, inset or overlay:
#     cup holes in the door's inner face (cup_edge mm from its edge), two
#     mounting-plate holes per hinge in the side's inner face, setback mm
#     behind the door and pitch mm apart, and a hinge joint that carries the
#     door across its reveal (up to max_gap mm), so it is not floating.
#     count defaults by door height (<= 900 mm: 2, 1600: 3, 2000: 4, else 5),
#     margin (door end to cup centre) to min(100, a quarter of the height).
#   contact(a, b) (see placement) finds the face they share.

# name: (diameter, length) in mm
DOWELS = {
    "6x30": (6, 30),
    "8x30": (8, 30),
    "8x35": (8, 35),
    "8x40": (8, 40),
    "10x40": (10, 40),
    "10x50": (10, 50),
}

def _mm(value):
    """`value` to 0.1 mm, without ".0" when whole."""
    value = round_to(value, 0.1)
    return int(value) if value == int(value) else value

def _drill_room(part, face, rest):
    """(thickness behind `face`, deepest hole leaving `rest` of it) of a part:
    how far the part reaches across the face's normal."""
    n = face_normal(part, face)
    thickness = reach(part, n) + reach(part, vec_scale(n, -1))
    return (thickness, thickness - max(rest, thickness / 3.0))

def _dowel_depths(a, b, face_a, face_b, dowel, length, clearance, rest):
    """Hole depths (in a, in b): an even split when both parts have room,
    else the thinner side as deep as its rest allows and the rest of the
    dowel in the other; each hole `clearance` longer than its dowel end."""
    room_a, room_b = _drill_room(a, face_a, rest), _drill_room(b, face_b, rest)
    grip = [int((room[1] - clearance) * 2) / 2.0 for room in (room_a, room_b)]
    if min(grip) >= length / 2.0:
        return (length / 2.0 + clearance, length / 2.0 + clearance)
    thin = 0 if grip[0] <= grip[1] else 1
    ends = [0, 0]
    ends[thin] = grip[thin]
    ends[1 - thin] = length - grip[thin]
    names = (part_info(a).name, part_info(b).name)
    rooms = (room_a, room_b)
    if grip[thin] < length / 4.0 or ends[1 - thin] > grip[1 - thin]:
        fail(("dowels(%s, %s): a %s dowel needs %s mm of holes plus %s mm clearance at each end, " +
              "but %s (%s mm thick there) takes a hole of at most %s mm and %s (%s mm) at most %s mm " +
              "(each leaves max(rest=%s, a third of the thickness) undrilled); use a shorter dowel " +
              "or thicker parts") %
             (names[0], names[1], dowel, length, clearance, names[0], _mm(rooms[0][0]), _mm(rooms[0][1]),
              names[1], _mm(rooms[1][0]), _mm(rooms[1][1]), rest))
    return (ends[0] + clearance, ends[1] + clearance)

def dowels(a, b, dowel = "8x35", count = None, margin = 50, spacing = 250, clearance = 1.5, rest = 6):
    """Dowels a and b along the face where they touch.

    Holes are drilled into both parts from the shared face, so moving a part
    or changing `margin`/`count` moves the holes in both. A hole never comes
    closer to the far side of its part than `rest` mm or a third of the part's
    thickness there, so an 18 mm board takes 12 mm. When one part cannot take
    half the dowel (drilled into its face), it gets what it can take and the
    other part (drilled into its edge) the rest; each hole is `clearance` mm
    deeper than the dowel end in it. Fails with the numbers when the parts
    cannot hold the dowel.
    """
    if dowel not in DOWELS:
        fail("dowels(): unknown dowel %r; use one of %s" % (dowel, sorted(DOWELS.keys())))
    diameter, length = DOWELS[dowel]
    c = contact(a, b)
    if c == None:
        fail("dowels(%s, %s): the parts do not touch; place them face to face first" % (part_info(a).name, part_info(b).name))
    depth_a, depth_b = _dowel_depths(a, b, c.face_a, c.face_b, dowel, length, clearance, rest)
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
    for i, point in enumerate(points):
        hole(a, c.face_a, world = point, diameter = diameter, depth = depth_a, id = "dowel:%s:%d" % (part_info(b).name, i + 1))
        hole(b, c.face_b, world = point, diameter = diameter, depth = depth_b, id = "dowel:%s:%d" % (part_info(a).name, i + 1))
    joint(a, b, kind = "dowel", fasteners = points, fastener = "dowel " + dowel)
    return points

def _dot(a, b):
    return a[0] * b[0] + a[1] * b[1] + a[2] * b[2]

def _world(info, local):
    """World point of a point given in the part's own frame."""
    return vec_add(info.at, vec_add(vec_scale(info.x, local[0]),
                                    vec_add(vec_scale(info.y, local[1]), vec_scale(info.z, local[2]))))

def _thinnest(info):
    return 0 if info.size[0] <= min(info.size[1], info.size[2]) else (1 if info.size[1] <= info.size[2] else 2)

def _own_axis(info, direction):
    """(index, +1/-1) of the part's own axis most parallel to a world direction."""
    best = (0, 0)
    for i in range(3):
        d = _dot(getattr(info, "xyz"[i]), direction)
        if abs(d) > abs(best[1]):
            best = (i, d)
    return (best[0], 1 if best[1] >= 0 else -1)

HINGES_BY_HEIGHT = [(900, 2), (1600, 3), (2000, 4)]

def hinge(door, side, count = None, margin = None, cup = 35, cup_depth = 13, cup_edge = 4,
          setback = 37, pitch = 32, plate_hole = 5, plate_depth = 12, max_gap = 4, name = None):
    """Concealed cup hinges hanging `door` on `side` (inset or overlay).

    The hinge edge is the door edge towards the side; the door's inner face is
    the one towards the side's middle. Cups (cup mm, cup_depth deep) sit
    cup_edge mm from that edge; each hinge's mounting plate takes two holes in
    the side's face towards the door, setback mm behind the door's inner face
    and pitch mm apart. The hinge joint carries the door across a reveal of up
    to max_gap mm. Returns the cup centres (world)."""
    di, si = part_info(door), part_info(side)
    to_side = vec_sub(vec_add(si.min, si.max), vec_add(di.min, di.max))
    across = getattr(si, "xyz"[_thinnest(si)])
    if _dot(across, to_side) < 0:
        across = vec_scale(across, -1)
    t = _thinnest(di)
    inward = getattr(di, "xyz"[t])
    t_sign = 1 if _dot(inward, to_side) >= 0 else -1
    inward = vec_scale(inward, t_sign)
    a, a_sign = _own_axis(di, across)
    if a == t:
        fail("hinge(%s, %s): the door must stand across the side's front edge, not parallel to it" % (di.name, si.name))
    l = 3 - t - a
    height = di.size[l]
    if count == None:
        count = 5
        for limit, n in reversed(HINGES_BY_HEIGHT):
            if height <= limit:
                count = n
    if margin == None:
        margin = min(100, height / 4.0)
    if count > 1 and (height - 2 * margin) / (count - 1) < cup + 10:
        fail("hinge(%s, %s): %d hinges with cups of %s mm do not fit on a %s mm door (margin %s mm)" %
             (di.name, si.name, count, cup, _mm(height), _mm(margin)))
    gap = distance(door, side).distance
    if gap > max_gap:
        fail("hinge(%s, %s): the door is %s mm from the side, more than max_gap = %s mm a hinge bridges" %
             (di.name, si.name, _mm(gap), max_gap))
    door_face = "xyz"[t] + ("+" if t_sign > 0 else "-")
    side_axis, side_sign = _own_axis(si, across)
    side_face = "xyz"[side_axis] + ("-" if side_sign > 0 else "+")
    plane = -reach(side, vec_scale(across, -1))
    along = getattr(di, "xyz"[l])
    label = name or "hinge:%s" % di.name
    cups = []
    for i, height_at in enumerate(spread(margin, height - margin, count)):
        local = [0, 0, 0]
        local[t] = di.size[t] if t_sign > 0 else 0
        local[a] = di.size[a] - cup_edge - cup / 2.0 if a_sign > 0 else cup_edge + cup / 2.0
        local[l] = height_at
        centre = _world(di, local)
        cups.append(centre)
        hole(door, door_face, world = centre, diameter = cup, depth = cup_depth, id = "%s:cup:%d" % (label, i + 1))
        plate = vec_add(centre, vec_scale(inward, setback))
        plate = vec_add(plate, vec_scale(across, plane - _dot(plate, across)))
        for j, offset in enumerate([-pitch / 2.0, pitch / 2.0]):
            hole(side, side_face, world = vec_add(plate, vec_scale(along, offset)), diameter = plate_hole,
                 depth = plate_depth, id = "%s:plate:%d.%d" % (label, i + 1, j + 1))
    joint(door, side, kind = "hinge", fasteners = cups, fastener = "hinge %s mm with plate" % cup,
          max_gap = max(gap, 0) + 0.5, name = label)
    return cups

#@topic intent: Stating intent that the check measures (expect_*)
#
# Stating intent. Each helper records a condition that is measured on the
# final model (after every move), so write them anywhere; one that does not
# hold is an `expectation_failed` error naming the measured and required mm.
# Faces follow the same rule as on(): the target's own frame, or a world
# direction. Measures use the body before cuts and booleans, except that
# KetchupProgram check/build and the window take distance and contact_area
# of a pair from the exact solids when a part of it is not its box (profile
# body, push_pull, subtract/intersect); the same holds for collisions,
# floating parts, joints and the relation map (relations lose "approx").
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

#@topic report: What KetchupProgram check/build returns
#
# ok, errors, warnings and issues: each issue has kind, severity, parts,
#   location in mm and a fix hint (collisions, floating parts, joints
#   without contact, failed expectations, ...). Fix errors and check again.
# relations: how each pair of parts meets: contact (which faces, area),
#   overlap (depth), gap (mm, within 20 mm), joint, subtracted/pocket;
#   "approx" when measured on boxes instead of exact solids.
# params: every param() with its value and limits (override by name).
# bom: cut_list (material, dimensions, count, parts), hardware (item,
#   count), machining per part (holes and pockets with face, position,
#   diameter, depth; KetchupProgram machining=true lists them all).
# log: lines from print(); unused_overrides: override names no param() has.
