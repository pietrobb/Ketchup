# Ketchup rule library.
#
# Helpers written only from the generic builtins; adding one never requires a
# change in Rust. The file is split into topics by "#@topic id: title" lines:
# the program documentation an AI reads lists the topics with their helpers
# and returns one topic (docs and code) by its id.

#@topic basics: Parts, frames, faces, parameters, numbers
#
#   param(name, default, min=, max=, doc=)  -> number the user can override
#   box(name, size, at=, material=, color=, attributes=, tool=)  -> part
#   board(name, size, at=, material=, grain=, color=)  -> a panel for the cut list
#   Every body (box, extrude, revolve, sweep, loft) takes material=, color=
#   (0-255 channels), attributes= (a dict of strings), tags= and grounded=False.
#   tags= names document tags (layers), one name or a list, e.g. tags=["concept", "roof"].
#     tag(items, tags) adds tags to a part, a group (all its parts) or a list of them.
#     A part is hidden while any of its tags is hidden. Apply creates missing tags, keeps
#     tags the user added by hand and the user's tag visibility, and deletes a tag the
#     program stopped naming once no part uses it. Hiding a tag never detaches the program.
#   alternatives(["concept", "construction"]) declares tags that represent the same thing
#     in different detail: parts carrying different ones of them may overlap without a
#     collision. Show one at a time with saved views; see buildup() in topic buildup.
#   grain= of board() and member() is the attribute "grain": "x", "y" or "z".
#   member(name, start, end, section, across=)  -> a bar from point to point
#   floor(z) -> set one horizontal support plane at world z millimetres in program and document reports.
#     Defaults to z=0; does not move parts. An explicit floor reports unsupported bodies even with no contact seed.
#   grounded=True anchors a part without moving it; touching bodies may receive its support.
#     group/component(..., grounded=True) anchors every descendant, not unrelated siblings.
#     instance(..., grounded=) inherits the source root by default; False releases only that root,
#     not explicit internal anchors. Copies inherit their source part's anchor.
#     Change grounding via a boolean expression of param(); anchors persist through Save/Open
#     and typed edits that detach the program. This is support, not a lock on explicit placement.
#   group(name, members, grounded=False) -> group in the document tree, without moving parts.
#     Members are existing parts/groups (or their names), each with one parent.
#     Declare children before parents; names are unique across parts and groups.
#     Group names preserve document IDs across apply, including membership edits.
#     Reports warn about disconnected sets of members, even when grounded. Contact outside the
#     group and joint declarations cannot hide separation; non-box contacts require exact verification.
#     After typed edits, request the group_connectivity validator in apply_and_verify; it uses
#     canonical membership and native contacts even when the program has been detached.
#     Physical joint declarations also persist through detachment and Save/Open; the validator
#     reports exact distance and max_gap in mm. Motion-only joints do not require contact.
#   component(name, members) -> shared assembly, including its first instance.
#     Finish members' machining/placement first; members are parts, groups or component instances.
#   instance(name, component, at=(0,0,0), x=(1,0,0), z=(0,0,1)) -> instance.
#     at/x/z transform the entire assembly about the PROGRAM ORIGIN, not its centre.
#     Read copied parts as "instance-name/original-part-name"; edit shared members
#     before component(), or copy() a part for independent machining. Internal
#     joints/fasteners are copied; external joints and expect() conditions are not.
#     Members may include components and instances; nested copies compose their transforms.
#     Shared dimension edits, instance placement/add/remove and regrouping preserve surviving IDs.
#     Nested edits update shared geometry once, including rebuilds, local copies and local groups.
#     Incremental apply keeps the component definitions and member owners; move members only
#     between groups of the same component. Changing owners or the component set needs a new build.
#   continue_part(part, was="previous-name") -> part; explicitly retain a previous part's ID.
#     Use on one surviving piece after a rename/split. Other pieces are new parts.
#     Keep the declaration on later edits; a fresh build uses the new names normally.
#     Works on root/grouped parts and shared source members, including nested components.
#     Shared members retain their local key in the same component; all instance paths follow.
#     Declare on the source member, not a copied leaf. Definition/feature IDs may change on rebuild.
#   joint(a, b, kind="motion", motion=slide((0,1,0), 0, 500), position=param("open", 0))
#     Moves the whole b (part/group/component, including nested members) relative to fixed a.
#     Position is an absolute offset from the FINISHED PROGRAM placement, not the previous pose.
#     slide(axis, min, max) uses mm; rotate_motion(axis, min, max, pivot=(0,0,0)) uses degrees.
#     Axes/pivots are in PROGRAM WORLD coordinates; rotation is right-handed.
#     Motion is applied after program evaluation: machining, part_info and prints use reference frames;
#     report/expect/collisions and the document use the posed geometry. Change position via param overrides.
#     Travel limits report issues without clamping. Motion is not a physical contact/fastener declaration.
#     Declare internal shared motions with both endpoints BEFORE component(); every instance inherits them.
#     Instance placement carries axes/pivots too. A parent pose carries both sides of internal joints;
#     inner motion runs first. Copied leaves cannot be driven separately; move the instance instead.
#     Independent driven-anchor chains and overlapping drivers are rejected, not silently solved.
#   rotate(part, axis=(x, y, z), angle=degrees, pivot=(x, y, z))  -> part
#   place(part, origin=(x, y, z), z=(x, y, z), x=(x, y, z))  -> part
#   part_info(part)  -> struct(name, size, at, min, max, local_min, local_max,
#     x, y, z): min/max in world, local_min/local_max in the part's own frame
#   math.sqrt/sin/cos/tan/asin/acos/atan/atan2/hypot/radians/degrees, math.pi
#   sum(values), round_to(value, step), spread(start, end, count),
#   divide(span, count, round_to=), vec_add/vec_sub/vec_scale/vec_length
#   fmt_mm(value, decimals=1) -> text, 0-9 places, no trailing zeros or unit; print(...) returns in the report's log.
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

def grain_attributes(grain):
    """The attributes recording a material's grain direction ("x", "y", "z")."""
    if grain == None:
        return {}
    if grain not in AXES:
        fail("grain must be \"x\", \"y\" or \"z\", got %r" % grain)
    return {"grain": grain}

def board(name, size, at = (0, 0, 0), material = "board", grain = None, color = None):
    """A flat panel: an axis-aligned cuboid with a material for the cut list."""
    return box(name, size, at = at, material = material, color = color, attributes = grain_attributes(grain))

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
    part = box(name, (section[0], section[1], length), material = material, color = color, attributes = grain_attributes(grain))
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
#     "x+", "segment2" or "<boolean>.z+" for the face a cut left.
#     points is an even/odd boundary walk, not a convex hull: holes and
#     disconnected regions use doubled zero-area connectors. Cancel reverse
#     edge pairs when measuring clearance. origin/size are only bounds.
#     Curved boundaries use 0.001 mm chord sampling. Sweep/loft faces have no
#     analytic boundary and are not replaced by their boxes.

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
             (fmt_mm(thickness), fmt_mm(high - low), part_info(a).name, part_info(b).name))
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
#   sweep(name, profile=, path=, bend=None, up=None, at=, tool=)
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
#     A segment [start, end, {"controls": [c1, c2]}] is a cubic Bezier curve
#     (as in profiles) along which the profile is carried without twist.
#     up=(x, y, z) instead holds v on that direction and u on tangent x up
#     all along (the path must never run along up): the profile plane keeps
#     up in it, so it does not twist about up.
#   helix(radius=, pitch=, turns=, at=(0, 0, 0), axis=(0, 0, 1),
#         start_angle=0, left=False)  -> a sweep path: the helix around the
#     axis through `at`, starting `start_angle` degrees from local +x (for
#     the default axis), rising `pitch` per turn, counter-clockwise seen from
#     the axis tip (left=True: clockwise), as cubic quarter turns within
#     0.03 % of the true helix. Swept with up=axis (up=-axis when left=True)
#     the profile stays in the axial section, u pointing away from the axis
#     and v along it, so any closed profile narrower than the pitch becomes a
#     thread or a spring with the same section on every turn, e.g. an M10
#     thread on a 8.4 mm core:
#     sweep("thread", profile=v_thread(depth=0.8, width=1.3),
#           path=helix(radius=4.2, pitch=1.5, turns=12), up=(0, 0, 1))
#     (without up the profile is carried without twist like along any curve
#     and so turns about the path relative to the axis, ~19 deg per M10 turn).
#   v_thread(depth, width), trapezoid_thread(depth, width, crest) and
#     round_thread(radius, name="thread") are such profiles with their base
#     on the path: a triangle `width` wide with its tip `depth` out along u,
#     a trapezoid narrowing to `crest` at that depth, and a circle centred on
#     the path.
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

def v_thread(depth, width):
    """A thread tooth: base `width` wide along v on the path, tip `depth` out along u."""
    return trapezoid_thread(depth, width, 0)

def trapezoid_thread(depth, width, crest):
    """A thread tooth: base `width` wide along v on the path, `crest` wide `depth` out along u."""
    if depth <= 0 or width <= 0 or crest < 0 or crest >= width:
        fail("thread profile: need depth > 0 and 0 <= crest < width, got depth %s, width %s, crest %s" %
             (depth, width, crest))
    if crest == 0:
        return [(0, -width / 2), (depth, 0), (0, width / 2)]
    return [(0, -width / 2), (depth, -crest / 2), (depth, crest / 2), (0, width / 2)]

def round_thread(radius, name = "thread"):
    """A round thread section: a circle of `radius` centred on the path, as two named half arcs."""
    if radius <= 0:
        fail("round_thread(): radius must be positive, got %s" % radius)
    return [[name, (-radius, 0), (radius, 0), {"center": (0, 0)}],
            [name + "_back", (radius, 0), (-radius, 0), {"center": (0, 0)}]]

#@topic machining: Holes, pockets, grooves, rebates, trims and booleans
#
# Positions (u, v) are in the face's own coordinates (see basics). `face` is
# a face name or a face() value of any part: a hole goes into any flat or
# round face along its inward normal, a pocket into any flat face, before or
# after a mirror or boolean.
#   hole(part, face, at=(u, v) | world=(x, y, z), diameter=, depth=, through=False, id=)
#     Returns struct(part=part_name, id=operation_id), usable in joint_link(); callers may ignore it.
#     depth is always explicit; through=True declares an intentional through bore, not a blind bore accidentally breaking out.
#     Set depth to at least the part span along the drilling direction; a shorter through bore reports an error.
#     Intent is retained in source, machining reports and canonical through cuts. Shallow through requests keep numeric geometry and report an error, not a silently deeper cut.
#     Neutral manufacturing output preserves through intent; machine-specific BTLx/HOMAG through export is unsupported and rejected. Later/non-cuboid bores remain boolean cuts, not inferred machine drilling.
#     On a round face at=(angle in degrees, v): a radial hole, see face(); keep 3 mm between opposite bores (report warns below 3 mm, errors when they meet).
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
        fail("countersunk_hole(%s): the %s mm cone is deeper than the %s mm hole" % (info.name, fmt_mm(sink), depth))
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
#   joint(a, b, kind=, fasteners=, fastener=, volume=, max_gap=0, name=, links=[])
#     links=[joint_link([hole_result, ...], hardware=[physical_part, ...])]
#     explicitly owns operations by (part, id); no ownership is inferred from position.
#     For named pockets/booleans use struct(part=part, id=operation_name).
#     Hardware omitted means metadata only; linked solids must already exist.
#     records that a and b are joined (the part is carried, not floating);
#     Fastener world centres follow the first endpoint a when it is placed/moved/rotated after declaration; moving b does not double-transform them or repair a separated joint.
#     fastener names are counted in the report's hardware list. The parts
#     must touch, or stay within max_gap mm (a door on hinges across its
#     reveal: joint(door, side, kind="hinge", max_gap=3)).
#   dowels(a, b, dowel="8x35", count=, margin=50, spacing=250, clearance=1.5, rest=6, offset=0)
#     drills matching holes into two touching parts and records the joint.
#     offset is signed mm along the selected row (positive towards increasing contact u/v).
#     It shifts both mating patterns from the margin-based centres, without changing count or depth.
#     Returns world centres and logs nonzero offsets; no automatic staggering or safety claim.
#     Use e.g. offset=11 for 8 mm opposing bores to leave 3 mm laterally; inspect validation.
#     An offset outside the actual contact retains requested centres and reports a validation issue, never clamps.
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

def fmt_mm(value, decimals = 1):
    """Millimetres as text, with 0-9 decimal places, trailing zeros omitted.
    Rounds ties away from zero; includes no unit suffix or negative zero.
    """
    if type(decimals) != "int" or decimals < 0 or decimals > 9:
        fail("fmt_mm(): decimals must be an integer from 0 to 9, got %r" % decimals)
    scale = int("1" + "0" * decimals)
    rounded = int(abs(value) * scale + 0.5)
    digits = str(rounded)
    digits = "0" * max(0, decimals + 1 - len(digits)) + digits
    text = digits if decimals == 0 else (digits[:-decimals] + "." + digits[-decimals:]).rstrip("0").rstrip(".")
    return ("-" if value < 0 and rounded != 0 else "") + text

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
        check(False, ("dowels(%s, %s): a %s dowel needs %s mm of holes plus %s mm clearance at each end, " +
              "but %s (%s mm thick there) takes a hole of at most %s mm and %s (%s mm) at most %s mm " +
              "(each leaves max(rest=%s, a third of the thickness) undrilled); use a shorter dowel " +
              "or thicker parts") %
             (names[0], names[1], dowel, length, clearance, names[0], fmt_mm(rooms[0][0]), fmt_mm(rooms[0][1]),
              names[1], fmt_mm(rooms[1][0]), fmt_mm(rooms[1][1]), rest),
              parts = [a, b], hint = "Use a shorter dowel or thicker parts. No joint or holes were generated.")
        return None
    return (ends[0] + clearance, ends[1] + clearance)

def joint_link(operations, hardware = []):
    """Explicitly associates operation references (hole() results or struct(part=, id=))
    with one joint. hardware contains existing solid parts, never catalog labels.
    Empty hardware means metadata only. Each operation belongs to one link only.
    Include linked hardware in a component's members when instancing its joints.
    Removing a declaration does not erase machining: edit the producing source helper.
    """
    return struct(operations = operations, hardware = hardware)

def dowels(a, b, dowel = "8x35", count = None, margin = 50, spacing = 250, clearance = 1.5, rest = 6, offset = 0):
    """Dowels a and b along the face where they touch.

    Holes are drilled into both parts from the shared face, so moving a part
    or changing `margin`/`count` moves the holes in both. A hole never comes
    closer to the far side of its part than `rest` mm or a third of the part's
    thickness there, so an 18 mm board takes 12 mm. When one part cannot take
    half the dowel (drilled into its face), it gets what it can take and the
    other part (drilled into its edge) the rest; each hole is `clearance` mm
    deeper than the dowel end in it. Reports an issue with the numbers and
    returns [] without machining when no joint fits. `offset` translates the selected row along itself,
    consuming end margin; it never changes its count, direction or depths.
    Returned world centres and the printed offset announce the placement.
    Opposing-hole validation still reports unsafe placements; this does not
    certify assembly feasibility or load capacity.
    """
    if dowel not in DOWELS:
        fail("dowels(): unknown dowel %r; use one of %s" % (dowel, sorted(DOWELS.keys())))
    diameter, length = DOWELS[dowel]
    c = contact(a, b)
    if not check(c != None, "dowels(%s, %s): the parts do not touch" % (part_info(a).name, part_info(b).name),
                 parts = [a, b], hint = "Place the parts face to face. No joint or holes were generated."):
        return []
    depths = _dowel_depths(a, b, c.face_a, c.face_b, dowel, length, clearance, rest)
    if depths == None:
        return []
    depth_a, depth_b = depths
    points = _contact_row(c, count, margin, spacing, diameter, offset)
    if points == None and offset != 0:
        points = _contact_row(c, count, margin, spacing, diameter, offset, check_shift = False)
        if points != None:
            check(False, "dowels(%s, %s): no row fits with offset %s mm" % (part_info(a).name, part_info(b).name, offset),
                  parts = [a, b],
                  hint = "Reduce the explicit offset or enlarge the contact. Requested centres are retained, not clamped.")
    if not check(points != None, "dowels(%s, %s): no row fits inside the contact with margin %s mm, diameter %s mm and offset %s mm" % (part_info(a).name, part_info(b).name, margin, diameter, offset),
                 parts = [a, b], hint = "Reduce count/margin/offset or use a larger contact. No joint or holes were generated."):
        return []
    links = []
    for i, point in enumerate(points):
        first = hole(a, c.face_a, world = point, diameter = diameter, depth = depth_a, id = "dowel:%s:%d" % (part_info(b).name, i + 1))
        second = hole(b, c.face_b, world = point, diameter = diameter, depth = depth_b, id = "dowel:%s:%d" % (part_info(a).name, i + 1))
        links.append(joint_link([first, second]))
    joint(a, b, kind = "dowel", fasteners = points, fastener = "dowel " + dowel, links = links)
    if offset != 0:
        print("dowels(%s, %s): explicit row offset %s mm; world centres %s; check validation issues for bore clearance" % (part_info(a).name, part_info(b).name, offset, points))
    return points

def _contact_row(c, count, margin, spacing, diameter, offset = 0, check_shift = True):
    """A row inside actual material. Margin is end-to-centre; the entire
    bore must clear every real boundary, including holes and concave notches.
    Doubled connectors in contact.points carry no boundary clearance."""
    if margin < 0 or spacing <= 0 or (count != None and (count < 1 or count != int(count))):
        fail("dowels(): use nonnegative margin, positive spacing and a positive integer count")
    points = c.points
    edges = [(points[i], points[(i + 1) % len(points)]) for i in range(len(points))]
    edges = [(a, b) for a, b in edges if a != b and (b, a) not in edges]
    best, best_length = None, -1
    inset = max(margin, diameter / 2.0)
    directions = [c.u, c.v] + [vec_sub(b, a) for a, b in edges if vec_length(vec_sub(b, a)) >= diameter]
    for delta in directions:
        length = vec_length(delta)
        row = vec_scale(delta, 1.0 / length)
        if _dot(row, c.u if abs(_dot(row, c.u)) >= abs(_dot(row, c.v)) else c.v) < 0:
            row = vec_scale(row, -1)
        across = vec_sub(vec_scale(c.v, _dot(row, c.u)), vec_scale(c.u, _dot(row, c.v)))
        vertices = [(_dot(vec_sub(p, c.origin), row), _dot(vec_sub(p, c.origin), across)) for p in points]
        levels = sorted([p[1] for p in vertices])
        heights = [(levels[0] + levels[-1]) / 2.0] + [(levels[i] + levels[i + 1]) / 2.0 for i in range(len(levels) - 1) if levels[i + 1] - levels[i] >= diameter]
        for height in heights:
            hits = []
            for i, p in enumerate(vertices):
                q = vertices[(i + 1) % len(vertices)]
                if (p[1] > height) != (q[1] > height):
                    hits.append(p[0] + (q[0] - p[0]) * (height - p[1]) / (q[1] - p[1]))
            hits = sorted(hits)
            for i in range(0, len(hits) - 1, 2):
                low, high = hits[i] + inset, hits[i + 1] - inset
                if high < low or high - low <= best_length:
                    continue
                n = count if count != None else max(2, 1 + int((high - low) / spacing))
                if n > 1 and (high - low) / (n - 1) < diameter:
                    continue
                candidate = [vec_add(c.origin, vec_add(vec_scale(row, s), vec_scale(across, height))) for s in spread(low, high, n)]
                safe = True
                for p in candidate:
                    for x, y in edges:
                        d = vec_sub(y, x)
                        t = max(0, min(1, _dot(vec_sub(p, x), d) / _dot(d, d)))
                        radial = vec_sub(p, vec_add(x, vec_scale(d, t)))
                        if _dot(radial, radial) < (diameter / 2.0 - 0.000001) * (diameter / 2.0 - 0.000001):
                            safe = False
                if safe:
                    best, best_length = candidate, high - low
                    best_row, best_low, best_high = row, hits[i], hits[i + 1]
    if best == None or offset == 0:
        return best
    shifted = [vec_add(p, vec_scale(best_row, offset)) for p in best]
    if not check_shift:
        return shifted
    for p in shifted:
        along = _dot(vec_sub(p, c.origin), best_row)
        if along < best_low + diameter / 2.0 or along > best_high - diameter / 2.0:
            return None
        for x, y in edges:
            d = vec_sub(y, x)
            t = max(0, min(1, _dot(vec_sub(p, x), d) / _dot(d, d)))
            if vec_length(vec_sub(p, vec_add(x, vec_scale(d, t)))) < diameter / 2.0 - 0.000001:
                return None
    return shifted

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
    to max_gap mm. Each cup and its two plate holes are explicitly linked to their hinge. Returns the cup centres (world)."""
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
             (di.name, si.name, count, cup, fmt_mm(height), fmt_mm(margin)))
    gap = distance(door, side).distance
    if gap > max_gap:
        fail("hinge(%s, %s): the door is %s mm from the side, more than max_gap = %s mm a hinge bridges" %
             (di.name, si.name, fmt_mm(gap), max_gap))
    door_face = "xyz"[t] + ("+" if t_sign > 0 else "-")
    side_axis, side_sign = _own_axis(si, across)
    side_face = "xyz"[side_axis] + ("-" if side_sign > 0 else "+")
    plane = -reach(side, vec_scale(across, -1))
    along = getattr(di, "xyz"[l])
    label = name or "hinge:%s" % di.name
    cups, links = [], []
    for i, height_at in enumerate(spread(margin, height - margin, count)):
        local = [0, 0, 0]
        local[t] = di.size[t] if t_sign > 0 else 0
        local[a] = di.size[a] - cup_edge - cup / 2.0 if a_sign > 0 else cup_edge + cup / 2.0
        local[l] = height_at
        centre = _world(di, local)
        cups.append(centre)
        operations = [hole(door, door_face, world = centre, diameter = cup, depth = cup_depth, id = "%s:cup:%d" % (label, i + 1))]
        plate = vec_add(centre, vec_scale(inward, setback))
        plate = vec_add(plate, vec_scale(across, plane - _dot(plate, across)))
        for j, offset in enumerate([-pitch / 2.0, pitch / 2.0]):
            operations.append(hole(side, side_face, world = vec_add(plate, vec_scale(along, offset)), diameter = plate_hole,
                 depth = plate_depth, id = "%s:plate:%d.%d" % (label, i + 1, j + 1)))
        links.append(joint_link(operations))
    joint(door, side, kind = "hinge", fasteners = cups, fastener = "hinge %s mm with plate" % cup, links = links, max_gap = max(gap, 0) + 0.5, name = label)
    return cups

#@topic intent: Stating intent that the check measures (expect_*)
#
#   check(condition, message, parts=[...], hint=) returns the boolean condition.
#     A false condition records a program_condition_failed error without aborting.
#     It evaluates now (for library design rules); use expect_* for final geometry.
#     Parts remain editable. No missing geometry or hardware is silently generated.
# Stating intent. Each helper records a condition that is measured on the
# final model (after every move), so write them anywhere; one that does not
# hold is an `expectation_failed` error naming the measured and required mm.
# Faces follow the same rule as on(): the target's own frame, or a world
# direction. Measures use the body before cuts and booleans, except that
# the Kečup window (program apply) takes distance and contact_area
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

#@topic space: Free space: landings, headroom, room to use things, zones a kind of part avoids
#
#   keep_clear(zone, name=None, only=None, ignore=[], hint=None)
#     `zone` is a tool body (box(..., tool=True)): never built, drawn or listed.
#     Every part of the final model that reaches into it is a free_space_occupied
#     error naming the part and the overlap in mm; editing is not blocked.
#     only= tag name(s): only parts carrying one of them count (windows over a bed).
#     ignore= parts that belong in the space. A part whose solid boxes and plain
#     profiles cannot decide is a free_space_unverified warning, never a pass.
#   free_space(name, size, at, only=None, ignore=[], hint=None)  -> the zone
#   service_space(part, side, depth, height=None, name=None, hint=None)  -> zone
#     room in front of a world side ("x-", "x+", "y-", "y+") of the part's box to
#     open and use it (a stove's fire door, a cupboard, an appliance), as wide as
#     the part, from its bottom up to `height` (default: the part's height).
#   stair_space(name, treads, top, landing=900, headroom=2000)  -> zones
#     `treads` in climbing order: a landing max(landing, stair width) deep before
#     the first tread and after the last one on the upper floor at world z `top`,
#     and `headroom` clear above every tread and both landings.
#   no_window_over(part, tag="window", margin=300, height=1500)  -> zone
#     nothing tagged `tag` above `part` (a bed) or within `margin` of it sideways,
#     up to `height` above its top.
# Example: stove = box("stove", (450, 500, 800), at=(3000, 1200, 0))
#          service_space(stove, "x+", 1000, height=1800)

def free_space(name, size, at, only = None, ignore = [], hint = None):
    """A tool box at `at` of `size` that parts (with a tag of `only`) stay out of."""
    zone = box(name, size, at = at, tool = True)
    keep_clear(zone, name = name, only = only, ignore = ignore, hint = hint)
    return zone

def _space_between(name, lo, hi, hint, only = None, ignore = []):
    return free_space(name, (hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]), (lo[0], lo[1], lo[2]),
                      only = only, ignore = ignore, hint = hint)

def service_space(part, side, depth, height = None, name = None, hint = None):
    """Room `depth` deep in front of world side `side` of `part` to open and use it."""
    info = part_info(part)
    if len(side) != 2 or side[0] not in ("x", "y") or side[1] not in ("+", "-"):
        fail("service_space(%r): side must be \"x-\", \"x+\", \"y-\" or \"y+\", got %r" % (_name(part), side))
    axis = AXES[side[0]]
    lo, hi = list(info.min), list(info.max)
    if side[1] == "+":
        lo[axis], hi[axis] = info.max[axis], info.max[axis] + depth
    else:
        lo[axis], hi[axis] = info.min[axis] - depth, info.min[axis]
    hi[2] = info.min[2] + (height if height != None else info.max[2] - info.min[2])
    return _space_between(name or "%s/service space" % info.name, lo, hi,
                          hint or "Keep %s mm in front of %s (%s) free to open and use it: move what stands there, or move or turn %s." %
                                  (fmt_mm(depth), info.name, side, info.name),
                          ignore = [part])

def stair_space(name, treads, top, landing = 900, headroom = 2000):
    """Landings at both ends of a straight flight and headroom over every tread."""
    if len(treads) < 2:
        fail("stair_space(%r): give at least two treads in climbing order" % name)
    first, second, last = part_info(treads[0]), part_info(treads[1]), part_info(treads[-1])
    shift = vec_sub(last.min, first.min)
    axis = 0 if abs(shift[0]) >= abs(shift[1]) else 1
    across = 1 - axis
    up = shift[axis] > 0
    width = first.max[across] - first.min[across]
    depth = max(landing, width)
    rise = second.max[2] - first.max[2]

    def zone(label, a0, a1, z0, hint):
        lo, hi = [0, 0, z0], [0, 0, z0 + headroom]
        lo[axis], hi[axis] = min(a0, a1), max(a0, a1)
        lo[across], hi[across] = first.min[across], first.max[across]
        return _space_between("%s/%s" % (name, label), lo, hi, hint)

    landing_hint = ("A stair needs a free landing at least as deep as it is wide (%s mm here) before the first " +
                    "and after the last step, with %s mm headroom: move the stair or what stands there.") % (fmt_mm(depth), fmt_mm(headroom))
    foot = first.min[axis] if up else first.max[axis]
    head = last.max[axis] if up else last.min[axis]
    step = depth if up else -depth
    zones = [zone("landing at the foot", foot - step, foot, first.max[2] - rise, landing_hint)]
    for k in range(len(treads)):
        tread = part_info(treads[k])
        zones.append(zone("headroom %d" % (k + 1), tread.min[axis], tread.max[axis], tread.max[2],
                          "%s mm headroom above every step: open the floor above or lower what hangs over the stair." % fmt_mm(headroom)))
    zones.append(zone("landing at the top", head, head + step, top, landing_hint))
    return zones

def no_window_over(part, tag = "window", margin = 300, height = 1500):
    """No part tagged `tag` above `part` or within `margin` of it, up to `height`."""
    info = part_info(part)
    lo = (info.min[0] - margin, info.min[1] - margin, info.max[2])
    hi = (info.max[0] + margin, info.max[1] + margin, info.max[2] + height)
    return _space_between("%s/no %s above" % (info.name, tag), lo, hi,
                          "Never put a %s over %s (draught, cold and condensation on the sleeper, glass overhead): move the %s or %s." %
                          (tag, info.name, tag, info.name),
                          only = [tag])

#@topic buildup: Layered build-ups: framed walls, floors and roofs
#
#   buildup(name, origin, along, up, length, layers, height=None, top=None,
#           openings=[], tags=[])  -> group of every piece
#     A flat panel built of layers: a timber-frame wall, floor, ceiling or roof. Its
#     plane is spanned from world point `origin` by `along` (u, the length) and `up`
#     (v, the height); layers stack from the inside face outward along along x up.
#     The outline is u from 0 to `length` and v from 0 to `height`, or up to `top`:
#     points [(u, v), ...] from u = 0 to u = length, e.g. a gable
#     [(0, 2600), (2000, 4280), (4000, 2600)]. `top` may also be a function of a
#     layer's inner offset (mm from the inside face) returning such points, for a panel
#     whose end another plane cuts (roof halves meeting in the ridge).
#     A layer is a dict {"name", "thickness", "material", "color", "tags"}. A sheet layer
#     is cut into pieces around the openings. A framed layer adds "spacing" (stud
#     centres in mm), "stud" (stud width, default 60) and optionally "infill" (material
#     between the studs; "infill_color", "infill_tags"): a bottom plate, a top plate
#     following the outline, studs at `spacing`, a stud on each side of every opening
#     with a header above and a sill under it, and infill in every bay.
#     openings: [(u, v, width, height), ...] in panel coordinates, apart along u.
#     Pieces are "<name>/<layer>", "<name>/<layer>/stud 3", ... with `tags` plus their
#     layer's tags. Only extrusions, no booleans: every piece is rebuilt from the numbers.
#     Pair it with a concept body through alternatives() and tags, e.g. a wall facing -y:
#     buildup("wall S", (0, 200, 0), (1, 0, 0), (0, 0, 1), 4000, height = 2600,
#             layers = [{"name": "gypsum", "thickness": 12.5, "material": "gypsum board"},
#                       {"name": "frame", "thickness": 160, "material": "C24",
#                        "spacing": 625, "infill": "mineral wool"},
#                       {"name": "osb", "thickness": 15, "material": "OSB"}],
#             openings = [(800, 0, 900, 2100)], tags = ["construction"])

def _cross(a, b):
    return (a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0])

def _outline(name, length, height, top, offset):
    """The top line of a build-up layer as [(u, v), ...] from u = 0 to u = length."""
    if type(top) == "function":
        points = top(offset)
    elif top != None:
        points = top
    elif height != None:
        points = [(0, height), (length, height)]
    else:
        fail("buildup(%s): give height= or top=" % name)
    points = [(float(p[0]), float(p[1])) for p in points]
    if len(points) < 2 or abs(points[0][0]) > 0.001 or abs(points[-1][0] - length) > 0.001:
        fail("buildup(%s): top must run from u = 0 to u = %s, got %s" % (name, fmt_mm(length), points))
    for i in range(len(points) - 1):
        if points[i + 1][0] <= points[i][0]:
            fail("buildup(%s): top points must go along u, got %s" % (name, points))
        if points[i][1] <= 0 or points[i + 1][1] <= 0:
            fail("buildup(%s): top must lie above v = 0, got %s" % (name, points))
    return points

def _under(points, drops, a, b, bottom):
    """The outline between u = a and u = b from v = bottom up to the top lowered by
    drops (one per top segment), counter-clockwise; None if it has no room."""
    upper = []
    for i in range(len(points) - 1):
        u0, v0 = points[i]
        u1, v1 = points[i + 1]
        lo, hi = max(a, u0), min(b, u1)
        if hi - lo < 0.001:
            continue
        for u in (lo, hi):
            point = (u, v0 + (v1 - v0) * (u - u0) / (u1 - u0) - drops[i])
            if not upper or abs(upper[-1][0] - point[0]) > 0.001 or abs(upper[-1][1] - point[1]) > 0.001:
                upper.append(point)
    if not upper or min([p[1] for p in upper]) - bottom < 1:
        return None
    return [(a, bottom), (b, bottom)] + [upper[i] for i in range(len(upper) - 1, -1, -1)]

def _rect(u0, v0, u1, v1):
    return [(u0, v0), (u1, v0), (u1, v1), (u0, v1)]

def _column(points, drops, a, b, bottom, blocks):
    """Pieces of the column u = a..b from v = bottom to the top, around the blocked
    v ranges [(low, high), ...] (high None: up to the top)."""
    pieces = []
    for low, high in sorted(blocks):
        if low - bottom >= 1:
            pieces.append(_rect(a, bottom, b, low))
        if high == None:
            return pieces
        bottom = max(bottom, high)
    rest = _under(points, drops, a, b, bottom)
    return pieces + ([rest] if rest != None else [])

def _spans(length, cuts):
    """The parts of 0..length left between the (start, end) ranges in cuts."""
    spans, cursor = [], 0.0
    for start, end in sorted(cuts):
        if start - cursor >= 1:
            spans.append((cursor, start))
        cursor = max(cursor, end)
    if length - cursor >= 1:
        spans.append((cursor, length))
    return spans

def buildup(name, origin, along, up, length, layers, height = None, top = None, openings = [], tags = []):
    """A panel built of layers: sheets, and framed layers with plates, studs and infill.
    See the topic text above for the arguments."""
    along = vec_scale(along, 1.0 / vec_length(along))
    up = vec_sub(up, vec_scale(along, _dot(up, along)))
    if vec_length(up) < 0.001:
        fail("buildup(%s): up must not run along `along`" % name)
    up = vec_scale(up, 1.0 / vec_length(up))
    out = _cross(along, up)
    if length <= 0:
        fail("buildup(%s): length must be positive, got %s" % (name, length))
    holes = [tuple([float(x) for x in o]) for o in openings]
    for i in range(len(holes)):
        ou, ov, ow, oh = holes[i]
        if ow <= 0 or oh <= 0 or ou < 0 or ov < 0 or ou + ow > length + 0.001:
            fail("buildup(%s): opening %s must lie inside u = 0..%s" % (name, holes[i], fmt_mm(length)))
        for other in holes[:i]:
            if ou < other[0] + other[2] and other[0] < ou + ow and ov < other[1] + other[3] and other[1] < ov + oh:
                fail("buildup(%s): openings %s and %s overlap" % (name, other, holes[i]))
    base_tags = _names(tags)
    pieces = []

    def piece(label, poly, at, thickness, material, color, piece_tags):
        part = extrude("%s/%s" % (name, label), profile = [[p[0], p[1]] for p in poly], distance = thickness,
                       material = material, color = color, tags = piece_tags)
        pieces.append(place(part, origin = vec_add(origin, vec_scale(out, at)), z = out, x = along))

    def covering(a, b):
        return [o for o in holes if o[0] <= a + 0.001 and b <= o[0] + o[2] + 0.001]

    offset = 0.0
    for layer in layers:
        label = layer["name"]
        thickness = layer["thickness"]
        if thickness <= 0:
            fail("buildup(%s): layer %s needs a positive thickness" % (name, label))
        material = layer.get("material", label)
        color = layer.get("color")
        layer_tags = base_tags + _names(layer.get("tags"))
        points = _outline(name, length, height, top, offset)
        flat = [0.0 for _ in points]
        # Openings that leave no room above them run out through the top edge.
        through = [_under(points, flat, o[0], o[0] + o[2], o[1] + o[3]) == None for o in holes]
        if "spacing" not in layer:
            edges = sorted({e: 0 for e in [0.0, length] + [o[0] for o in holes] + [o[0] + o[2] for o in holes]}.keys())
            parts = []
            for k in range(len(edges) - 1):
                a, b = edges[k], edges[k + 1]
                if b - a < 1:
                    continue
                blocks = [(o[1], None if through[holes.index(o)] else o[1] + o[3]) for o in covering(a, b)]
                parts += _column(points, flat, a, b, 0.0, blocks)
            for k in range(len(parts)):
                piece(label if len(parts) == 1 else "%s %d" % (label, k + 1), parts[k], offset, thickness, material, color, layer_tags)
            offset += thickness
            continue
        stud = float(layer.get("stud", 60))
        spacing = float(layer["spacing"])
        if spacing < stud:
            fail("buildup(%s): layer %s spacing %s is narrower than its studs" % (name, label, fmt_mm(spacing)))
        slopes = [(points[i + 1][1] - points[i][1]) / (points[i + 1][0] - points[i][0]) for i in range(len(points) - 1)]
        drops = [stud * math.sqrt(1 + slope * slope) for slope in slopes]
        # A stud, joist or rafter is blocked over an opening and its sill and header.
        zones = []
        for i in range(len(holes)):
            ou, ov, ow, oh = holes[i]
            if through[i] or _under(points, drops, ou, ou + ow, ov + oh + stud) == None:
                if not through[i]:
                    fail("buildup(%s): opening %s leaves no room for a header under the top plate" % (name, holes[i]))
                high = None
            else:
                high = ov + oh + stud
            zones.append((ou, ou + ow, ov - stud if ov >= 2 * stud else 0.0, high))
        for i in range(len(zones)):
            for j in range(i):
                a, b = zones[i], zones[j]
                if a[0] < b[1] and b[0] < a[1] and a[2] < (b[3] if b[3] != None else 1e12) and b[2] < (a[3] if a[3] != None else 1e12):
                    fail("buildup(%s): openings %s and %s leave no room for a sill and a header between them" % (name, holes[j], holes[i]))
        bottom = _spans(length, [(o[0], o[0] + o[2]) for o in holes if o[1] < 0.001])
        for k in range(len(bottom)):
            a, b = bottom[k]
            piece("%s/bottom plate%s" % (label, "" if len(bottom) == 1 else " %d" % (k + 1)), _rect(a, 0.0, b, stud), offset, thickness, material, color, layer_tags)
        plates = []
        for a, b in _spans(length, [(holes[i][0], holes[i][0] + holes[i][2]) for i in range(len(holes)) if through[i]]):
            for i in range(len(points) - 1):
                u0, v0 = points[i]
                u1, v1 = points[i + 1]
                lo, hi = max(a, u0), min(b, u1)
                if hi - lo < 1:
                    continue
                top_lo = v0 + slopes[i] * (lo - u0)
                top_hi = v0 + slopes[i] * (hi - u0)
                plates.append([(lo, top_lo - drops[i]), (hi, top_hi - drops[i]), (hi, top_hi), (lo, top_lo)])
        for k in range(len(plates)):
            piece("%s/top plate%s" % (label, "" if len(plates) == 1 else " %d" % (k + 1)), plates[k], offset, thickness, material, color, layer_tags)
        kings = []
        for ou, ov, ow, oh in holes:
            for side in (ou - stud, ou + ow):
                if side >= -0.001 and side + stud <= length + 0.001:
                    # Two openings closer than a stud share the stud beside them.
                    if not [s for s in kings if abs(s - side) < stud - 0.001]:
                        kings.append(side)
                elif side > 0.001 and side + stud < length - 0.001:
                    fail("buildup(%s): opening at u = %s needs room for a stud beside it" % (name, fmt_mm(ou)))
        regular = [k * spacing for k in range(int((length - stud) / spacing) + 1) if length - stud - k * spacing >= stud]
        regular.append(length - stud)
        studs = kings + [u for u in regular if not [s for s in kings if abs(s - u) < stud]
                         and not [o for o in holes if (u < o[0] and o[0] < u + stud) or (u < o[0] + o[2] and o[0] + o[2] < u + stud)]]
        studs = sorted({u: 0 for u in studs}.keys())
        count = 0
        for u in studs:
            # Also a shared stud reaching into the next opening stops at its sill and header.
            blocks = [(z[2], z[3]) for z in zones if z[0] < u + stud - 0.001 and u < z[1] - 0.001]
            for poly in _column(points, drops, u, u + stud, stud, blocks):
                count += 1
                piece("%s/stud %d" % (label, count), poly, offset, thickness, material, color, layer_tags)
        for k in range(len(holes)):
            ou, ov, ow, oh = holes[k]
            if zones[k][3] != None:
                piece("%s/header %d" % (label, k + 1), _rect(ou, ov + oh, ou + ow, ov + oh + stud), offset, thickness, material, color, layer_tags)
            if ov >= 2 * stud:
                piece("%s/sill %d" % (label, k + 1), _rect(ou, ov - stud, ou + ow, ov), offset, thickness, material, color, layer_tags)
        if layer.get("infill") != None:
            fill_tags = base_tags + _names(layer.get("infill_tags", layer.get("tags")))
            bays = []
            for k in range(len(studs) - 1):
                a, b = studs[k] + stud, studs[k + 1]
                if b - a < 1:
                    continue
                blocks = [(z[2], z[3]) for z in zones if z[0] < b - 0.001 and a < z[1] - 0.001]
                bays += _column(points, drops, a, b, stud, blocks)
            for k in range(len(bays)):
                piece("%s/infill %d" % (label, k + 1), bays[k], offset, thickness, layer["infill"], layer.get("infill_color"), fill_tags)
        offset += thickness
    return group(name, pieces)

#@topic validation: Inputs for document validators
#
# MCP list_validators() discovers all host checks, inputs/roles, run examples and result paths.
# It does not run checks or claim the current model is valid. program validate is read-only;
# incomplete/not_evaluated/skipped are never passes. Only document IDs belong in model validators.
# material= also assigns ketchup.material.v1 on root/grouped parts. Unknown material
# properties are not guessed. Known elastic moduli: engineered_wood, aluminium, steel.
# attributes={"classification:<dimension>": "<category>"} assigns a canonical category;
# material= takes precedence over classification:ketchup.material.v1 if both are given.
# Example: box("shelf", (1000,300,20), material="steel",
#              attributes={"classification:ketchup.validator-role.v1":"physics.beam.xy"})
# MCP program(action="validate", validators=["beam_deflection"]) reports span, predicted
# deflection, material and the library design-load assumption (not a strength certificate).
# Changing/removing these attributes updates only source-owned assignments, in the same
# Undo step as the program; geometry IDs and unrelated classification dimensions remain.
# Shared component leaves are not supported by these occurrence-based classifications;
# their attributes stay in the program/BOM and NEVER classify the assembly root.
# Missing roles or unknown properties mean not_evaluated, not passed. Keep validation
# subjects as root/grouped parts until instance-path classification is supported.
# Mixed root/shared models report the unchecked instance paths; checked root failures remain visible.
# Without material=, deflection uses the library default and labels material_source="rules_default";
# this is an explicit assumption, not verification of the actual part's material.
# Numeric inputs: attributes={"input:physics.mass_kg.occurrence.{occurrence}":"10"}.
# {occurrence} resolves to the final root ID; values must be finite numeric strings.
# Static-load inputs: physics.gravity_x_m_s2, physics.gravity_y_m_s2, physics.gravity_z_m_s2;
# physics.mass_kg.occurrence.{occurrence}, physics.applied_load_n.occurrence.{occurrence},
# physics.support_capacity_n.occurrence.{occurrence}. Declare all three gravity components.
# Static roles: classification:ketchup.validator-role.v1 = physics.static.load:<case>
# or physics.static.support:<case>. On each load also declare
# classification:ketchup.static-load-mode.v1 = e.g. compression, shear, or pullout.
# Each support needs classification:ketchup.support-capacity.v1 containing a JSON string (max 1024 bytes):
# '{"source":"test/report reference","units":"N","mode":"compression","direction_world":[0,0,-1],"assumptions":"material, fixing, safety factors and applicability","additive":false}'
# These are declared design capacities, NOT strengths inferred from shape, material or joint names.
# The signed WORLD direction is the supported force direction, not the opposite support reaction.
# It must match gravity (applied_load_n also acts along gravity); modes must match within a case.
# Multiple supports require additive=true and documented load-sharing assumptions on EVERY support.
# Only N is supported. Missing source/assumptions/mode, units or uncovered direction = not_evaluated.
# A passing static_load report compares declared numbers; it does not verify sources, load paths,
# moments, local reactions or certify strength. Smooth pins do not prove axial retention;
# pullout, shear and adhesive capacities require supplied data or a separately verified model.
# Removing an input removes its evaluator node; referenced inputs cannot be removed.
# Identical shared global inputs coalesce; conflicting values or non-program name collisions reject.
# Other attributes (e.g. grain) remain program metadata; they are not evaluator inputs.
# Motion: name a joint, e.g. joint(a,b,kind="motion",name="travel",motion=slide((1,0,0),0,100)).
# MCP program(action="validate",motion={"name":"travel","from":0,"to":100}) checks the
# whole interval using native solid distances and conservative interval travel bounds.
# Units: mm for slide, degrees for rotation. Stay inside declared limits. Other joints
# remain at their current poses; a nonzero driven parent frame is not yet supported.
# Contact, uncertified envelopes and work limits are incomplete, never successful samples.
# validation.motion names obstacles and full instance paths. Preview arrays have explicit totals/truncation.
# Assembly: assembly_step("travel",start=100,end=0) appends an insertion in SOURCE ORDER.
# Each rigid part/group is inserted once, using a named motion; end must equal its modeled position.
# Parts without a step are already installed. Future step members are absent; previous members stay fixed.
# The motion's reference endpoint must be installed first. Steps on instances use expanded motion names.
# MCP program(action="validate",validators=["assembly_path"]) reports validation.assembly_path.
# Translation contact can pass only with a swept-hull proof within native contact tolerance.
# Missing paths/order, unsupported motion and unproven final contact remain incomplete, not editing errors.
# This checks declared paths only, not every possible assembly order, retention or internal group collisions.
# Tool access: t=box("driver-envelope",(12,12,60),at=(0,0,5),tool=True)
# tool_access("drive",envelope=t,motion=slide((0,0,1),0,100),start=100,end=0)
# The auxiliary solid at its modeled pose represents the whole relevant tool/holder;
# end must be zero (working pose). Axis/pivot use PROGRAM WORLD coordinates, not the tool frame.
# MCP program(action="validate",validators=["tool_access"]) reports validation.tool_access.
# A tool=True solid is never added to the physical model/BOM. Use unions for composite envelopes.
# Include drill chucks or driver housings, not just a point, axis or cutting tip.
# Positive clearance is required throughout the approach including the working pose.
# Cutting/engagement, flexible cables, hand reach and simultaneous part motion are NOT assessed.
# Missing envelope or path, unsupported geometry and exhausted exact work remain incomplete.
#
#@topic report: What program apply returns
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
#   diameter, depth).
# log: lines from print(); unused_overrides: override names no param() has.
