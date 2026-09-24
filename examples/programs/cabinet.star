# A carcass cabinet: two sides, bottom, top, adjustable shelves on pins,
# a back panel in grooves, all doweled together.
#
#   ketchup-program check examples/programs/cabinet.star --set width=700

W = param("width", 600, min = 300, max = 1200, doc = "outer width")
H = param("height", 720, min = 300, max = 2400, doc = "outer height")
D = param("depth", 350, min = 200, max = 650, doc = "outer depth")
T = param("thickness", 18, doc = "carcass board thickness")
BACK = param("back_thickness", 4, doc = "back panel thickness (HDF)")
SHELVES = param("shelves", 2, min = 0, max = 8)

GROOVE_DEPTH = 8
GROOVE_FROM_BACK = 10          # groove starts this far from the back edge
inner_w = W - 2 * T

left = board("carcass/left", (T, D, H), at = (0, 0, 0))
right = board("carcass/right", (T, D, H), at = (W - T, 0, 0))
bottom = board("carcass/bottom", (inner_w, D, T), at = (T, 0, 0))
top = board("carcass/top", (inner_w, D, T), at = (T, 0, H - T))

for side in (left, right):
    for panel in (bottom, top):
        dowels(panel, side, dowel = "8x30", margin = 50)

# Back panel sits in grooves cut into the sides, top and bottom.
groove_y = D - GROOVE_FROM_BACK - BACK
for side, face in ((left, "x+"), (right, "x-")):
    groove(side, face, along = "z", width = BACK, depth = GROOVE_DEPTH, offset = groove_y)
for panel, face in ((bottom, "z+"), (top, "z-")):
    groove(panel, face, along = "x", width = BACK, depth = GROOVE_DEPTH, offset = groove_y)
board("carcass/back", (inner_w + 2 * GROOVE_DEPTH, BACK, H - 2 * T + 2 * GROOVE_DEPTH),
      at = (T - GROOVE_DEPTH, groove_y, T - GROOVE_DEPTH), material = "HDF 4")

# Shelves rest on 5 mm pins in System 32 rows; they stop short of the back panel.
inner_h = H - 2 * T
shelf_depth = groove_y - 2
for i in range(SHELVES):
    z = T + inner_h * (i + 1) / (SHELVES + 1)
    board("shelves/%d" % (i + 1), (inner_w - 2, shelf_depth, T), at = (T + 1, 0, z))
for side, face in ((left, "x+"), (right, "x-")):
    for y in (37, shelf_depth - 37):
        hole_row(side, face, start = (y, T + 96), step = 32, count = int((inner_h - 192) / 32) + 1,
                 diameter = 5, depth = 10, direction = "v")
for i in range(SHELVES):
    for side in (left, right):
        joint("shelves/%d" % (i + 1), side, kind = "shelf_pins", max_gap = 2)
