# Ketchup rule library.
#
# Domain helpers built only from the generic builtins:
#   param(name, default, min=, max=, doc=)
#   box(name, size, at=, material=, grain=, color=)  -> part
#   part_info(part)
#   hole(part, face, at=(u, v) | world=(x, y, z), diameter=, depth=, id=)
#   pocket(part, face, rect=(u_min, v_min, u_max, v_max), depth=, id=)
#   contact(a, b)  -> struct(axis, face_a, face_b, min, max) or None
#   joint(a, b, kind=, fasteners=, fastener=, volume=, name=)
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
    normal = AXES[c.axis]
    in_plane = [axis for axis in (0, 1, 2) if axis != normal]
    extents = [c.max[axis] - c.min[axis] for axis in in_plane]
    row = in_plane[0] if extents[0] >= extents[1] else in_plane[1]
    across = in_plane[1] if row == in_plane[0] else in_plane[0]
    row_length = c.max[row] - c.min[row]
    if row_length < 2 * margin:
        fail("dowels(%s, %s): the contact is %s mm long, shorter than twice the margin (%s mm)" % (part_info(a).name, part_info(b).name, row_length, margin))
    if count == None:
        count = max(2, 1 + int((row_length - 2 * margin) / spacing))
    centre_across = (c.min[across] + c.max[across]) / 2.0
    points = []
    for position in spread(c.min[row] + margin, c.max[row] - margin, count):
        point = [0.0, 0.0, 0.0]
        point[normal] = c.min[normal]
        point[row] = position
        point[across] = centre_across
        points.append(tuple(point))
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
