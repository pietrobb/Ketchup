# A three-legged stool: a round seat, splayed legs trimmed flat at the floor
# and at the seat top, legs let into the seat and rails let into the legs.
H = param("height", 450, min = 300, max = 800, doc = "seat top above the floor")
SPLAY = param("splay", 260, min = 130, max = 400, doc = "distance of each foot from the centre")
R = param("seat_radius", 180, min = 150, max = 300)
T = param("seat_thickness", 30, min = 18, max = 60)
LEG = 36
RAIL = 22

seat = revolve("seat", profile = [(0, 0), (R, 0), (R, T), (0, T)], axis = [(0, 0), (0, 1)], at = (0, 0, H - T))
rotate(seat, axis = (1, 0, 0), angle = 90)

def on_leg(foot, top, z):
    return vec_add(foot, vec_scale(vec_sub(top, foot), z / H))

ends = []
legs = []
for i, angle in enumerate([90, 210, 330]):
    a = math.radians(angle)
    foot = (SPLAY * math.cos(a), SPLAY * math.sin(a), 0)
    top = ((R - 60) * math.cos(a), (R - 60) * math.sin(a), H)
    d = vec_sub(top, foot)
    leg = member("leg%d" % (i + 1), vec_sub(foot, vec_scale(d, 0.1)), vec_add(top, vec_scale(d, 0.1)), (LEG, LEG))
    trim(leg, (0, 0, 0), (0, 0, -1))
    trim(leg, (0, 0, H), (0, 0, 1))
    ends.append((foot, top))
    legs.append(leg)

for i in range(3):
    j = (i + 1) % 3
    z = 170 + 30 * i
    rail = member("rail%d" % (i + 1), on_leg(ends[i][0], ends[i][1], z), on_leg(ends[j][0], ends[j][1], z), (RAIL, RAIL))
    subtract(legs[i], rail)
    subtract(legs[j], rail)

for leg in legs:
    subtract(seat, leg)
