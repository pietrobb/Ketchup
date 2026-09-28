# Ketchup rule library.
#
# Domain helpers built only from the generic builtins:
#   param(name, default, min=, max=, doc=)
#   box(name, size, at=, material=, grain=, color=, tool=)  -> part
#   extrude(name, profile=, distance=, at=, tool=), revolve(name, profile=, axis=, angle=, at=, tool=)
#   rotate(part, axis=(x, y, z), angle=degrees, pivot=(x, y, z))  -> part
#   place(part, origin=(x, y, z), z=(x, y, z), x=(x, y, z))  -> part
#   part_info(part)  -> struct(name, size, at, min, max, x, y, z)
#   subtract(part, tool, name=), intersect(part, tool, name=)  -> part
#     `tool` is a helper body made with box/extrude/revolve(..., tool=True)
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
# Profile parts. `profile` is a closed loop in the part's local XY plane:
# either points [[x, y], ...] (faces become "segment1", "segment2", ...) or
# named segments [["name", [x0, y0], [x1, y1]], ...]. extrude() pads it along
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
