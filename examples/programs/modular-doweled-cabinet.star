# Eight transportable modules, fixed shelves and blind dowelled plinths.
# This is a geometric example, not a load/assembly-access certification.
# Keep the requested simple joints; do not add brackets or screws silently.
width = 875
depth = 520
thickness = 18
height = 1260
right_clearance = 650
upper_cap = True

def module(name, x, z, clearance, upper):
    side_height = height - thickness if upper and upper_cap else height
    left = board(name + " left", (thickness, depth, side_height), at=(x, 0, z))
    right = board(name + " right", (thickness, depth, side_height), at=(x + width - thickness, 0, z))
    bottom = board(name + " bottom", (width - 2 * thickness, depth, thickness), at=(x + thickness, 0, z))
    cap_width = width if upper and upper_cap else width - 2 * thickness
    cap_x = x if upper and upper_cap else x + thickness
    top = board(name + " top", (cap_width, depth, thickness), at=(cap_x, 0, z + height - thickness))
    low = board(name + " low shelf", (width - 2 * thickness, depth, thickness), at=(x + thickness, 0, z + thickness + clearance))
    high = board(name + " high shelf", (width - 2 * thickness, depth, thickness), at=(x + thickness, 0, z + 1000))
    parts = [left, right, bottom, top, low, high]
    for side in [left, right]:
        for horizontal in [bottom, top, low, high]:
            dowels(side, horizontal, count=2, margin=50)
    for i, horizontal in enumerate([low, high] if upper else [bottom, low, high]):
        brace = board(name + " brace " + str(i), (width - 2 * thickness, thickness, 150), at=(x + thickness, depth - thickness, part_info(horizontal).at[2] + thickness))
        for side in [left, right]:
            dowels(side, brace, count=2, margin=35)
        dowels(brace, horizontal, count=4, margin=150)
        parts.append(brace)
    expect_gap(bottom, low, clearance)
    if not upper:
        front = board(name + " plinth front", (width - 2 * thickness, thickness, 80), at=(x + thickness, 50, 0))
        rear = board(name + " plinth rear", (width - 2 * thickness, thickness, 80), at=(x + thickness, depth - thickness, 0))
        parts.extend([front, rear])
        for rail in [front, rear]:
            dowels(rail, bottom, count=4, margin=100)
        for i, offset in enumerate([thickness, width / 2 - thickness / 2, width - 2 * thickness]):
            support = board(name + " plinth support " + str(i), (thickness, depth - 50 - 2 * thickness, 80), at=(x + offset, 50 + thickness, 0))
            for rail in [front, rear]:
                dowels(rail, support, count=2, margin=20)
            parts.append(support)
    return group(name, parts)

lower = []
upper = []
for column in range(4):
    lower.append(module("lower " + str(column), column * width, 80, right_clearance if column == 3 else 400, False))
    upper.append(module("upper " + str(column), column * width, 80 + height, 400, True))
group("cabinet", [group("lower modules", lower), group("upper modules", upper)])
