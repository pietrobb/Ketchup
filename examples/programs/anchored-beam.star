# A timber beam anchored to a concrete footing: four DIN 975 threaded rods cast
# into the footing hold a steel base plate with ISO 7089 washers and ISO 4032
# nuts; two steel fin plates on the base plate clamp the beam with ISO 4014
# bolts. The footing carries a bottom mesh of B500B bars in two layers.
# The cut list names every fastener and bar by designation and standard with
# its source; the bars' mass_kg is the nominal EN 10080 mass.
SPAN = param("beam_length", 1000, min = 400, max = 3000, doc = "length of the beam along y")
FOOT = param("footing", 800, min = 500, max = 1500, doc = "side of the square footing")
DEPTH = 400
COVER = 50
BAR = param("bar_diameter", 12, min = 6, max = 40, doc = "B500B bars of the bottom mesh")
BARS = 4
ANCHOR = (80, 75)
PLATE = 20
FIN = 10
BEAM = (80, 140)

footing = box("footing", (FOOT, FOOT, DEPTH), at = (-FOOT / 2, -FOOT / 2, -DEPTH), material = "concrete C25/30")

# Bottom mesh: bars along x, then bars along y lying on them.
spacing = (FOOT - 2 * COVER) / (BARS - 1)
for i in range(BARS):
    offset = -FOOT / 2 + COVER + i * spacing
    low = rebar("bar x%d" % (i + 1), BAR, FOOT - 2 * COVER, at = (-FOOT / 2 + COVER, offset, -DEPTH + COVER + BAR / 2), axis = (1, 0, 0))
    high = rebar("bar y%d" % (i + 1), BAR, FOOT - 2 * COVER, at = (offset, -FOOT / 2 + COVER, -DEPTH + COVER + 1.5 * BAR), axis = (0, 1, 0))
    subtract(footing, low)
    subtract(footing, high)

plate = box("base plate", (200, 200, PLATE), at = (-100, -100, 0), material = "steel S235")
for sx in [-1, 1]:
    for sy in [-1, 1]:
        x, y = sx * ANCHOR[0], sy * ANCHOR[1]
        rod = threaded_rod("anchor %d %d" % (sx, sy), "M12", 400, at = (x, y, -300))
        subtract(footing, rod)
        hole(plate, "z+", world = (x, y, PLATE), diameter = 14, depth = PLATE, through = True)
        washer("anchor washer %d %d" % (sx, sy), "M12", at = (x, y, PLATE))
        nut("anchor nut %d %d" % (sx, sy), "M12", at = (x, y, PLATE + 2.5))

beam = box("beam", (BEAM[0], SPAN, BEAM[1]), at = (-BEAM[0] / 2, -SPAN / 2, PLATE), material = "KVH C24")
fins = [
    box("fin plate %d" % side, (FIN, 200, BEAM[1] + 10), at = (side * BEAM[0] / 2 + (side - 1) * FIN / 2, -100, PLATE), material = "steel S235")
    for side in [-1, 1]
]

# Through-bolts along x: head on the left fin, washer and nut on the right one.
for y in [-50, 50]:
    z = PLATE + BEAM[1] / 2
    left = -BEAM[0] / 2 - FIN
    right = BEAM[0] / 2 + FIN
    for part, face_x, span in [(fins[0], left + FIN, FIN), (beam, BEAM[0] / 2, BEAM[0]), (fins[1], right, FIN)]:
        hole(part, "x+", world = (face_x, y, z), diameter = 13, depth = span, through = True)
    bolt("bolt %d" % y, "M12", 120, norm = "ISO 4014", at = (left, y, z), axis = (-1, 0, 0))
    washer("bolt washer %d" % y, "M12", at = (right, y, z), axis = (1, 0, 0))
    nut("bolt nut %d" % y, "M12", at = (right + 2.5, y, z), axis = (1, 0, 0))
