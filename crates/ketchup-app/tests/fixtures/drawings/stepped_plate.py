"""Writes stepped-plate-1to2.dxf with ezdxf 1.4: a part drawing made outside Kecup.

A plate 120 x 80, 15 thick, with a boss 40 x 80 standing 25 above it at its right end
(40 high there), a 30 hole through the plate at (40, 40). Drawn at 1:2 on an A4 sheet:
plan above, front view below, a border and title block, centre lines, hidden edges and
real DIMENSION entities, on Slovak layer names. Volume of the part:
120*80*15 + 40*80*25 - pi*15^2*15 = 213 397.1 mm3.

Run: python stepped_plate.py (writes next to this file).
"""

import pathlib

import ezdxf

S = 0.5  # 1:2
doc = ezdxf.new("R2010", setup=True)
doc.header["$INSUNITS"] = 4
for name, color, linetype in [
    ("VIDITELNE", 7, "Continuous"),
    ("SKRYTE", 1, "DASHED"),
    ("OSY", 3, "CENTER"),
    ("KOTY", 2, "Continuous"),
    ("RAMEC", 5, "Continuous"),
]:
    doc.layers.add(name, color=color, linetype=linetype)
msp = doc.modelspace()


def at(origin, x, y):
    return (origin[0] + x * S, origin[1] + y * S)


# Border and title block (A4 landscape, 297 x 210).
msp.add_lwpolyline([(0, 0), (297, 0), (297, 210), (0, 210)], close=True, dxfattribs={"layer": "RAMEC"})
msp.add_lwpolyline([(10, 10), (287, 10), (287, 200), (10, 200)], close=True, dxfattribs={"layer": "RAMEC"})
msp.add_lwpolyline([(187, 10), (287, 10), (287, 40), (187, 40)], close=True, dxfattribs={"layer": "RAMEC"})
msp.add_line((187, 25), (287, 25), dxfattribs={"layer": "RAMEC"})
msp.add_text("DOSKA S NALITKOM", height=4, dxfattribs={"layer": "RAMEC"}).set_placement((190, 30))
msp.add_text("M 1:2   S235   c.v. 0042", height=3, dxfattribs={"layer": "RAMEC"}).set_placement((190, 15))

# Plan, origin at (60, 120) on the sheet.
plan = (60, 120)
msp.add_lwpolyline([at(plan, *p) for p in [(0, 0), (120, 0), (120, 80), (0, 80)]], close=True, dxfattribs={"layer": "VIDITELNE"})
msp.add_line(at(plan, 80, 0), at(plan, 80, 80), dxfattribs={"layer": "VIDITELNE"})
msp.add_circle(at(plan, 40, 40), 15 * S, dxfattribs={"layer": "VIDITELNE"})
msp.add_line(at(plan, 20, 40), at(plan, 60, 40), dxfattribs={"layer": "OSY"})
msp.add_line(at(plan, 40, 20), at(plan, 40, 60), dxfattribs={"layer": "OSY"})

# Front view, origin at (60, 70): same X as the plan.
front = (60, 70)
msp.add_lwpolyline(
    [at(front, *p) for p in [(0, 0), (120, 0), (120, 40), (80, 40), (80, 15), (0, 15)]],
    close=True,
    dxfattribs={"layer": "VIDITELNE"},
)
for x in (25, 55):
    msp.add_line(at(front, x, 0), at(front, x, 15), dxfattribs={"layer": "SKRYTE"})
msp.add_line(at(front, 40, -5), at(front, 40, 20), dxfattribs={"layer": "OSY"})


def dim(p1, p2, base, angle=0):
    # The drawing is at 1:2: dimlfac makes the text show the part's size.
    msp.add_linear_dim(
        base=base, p1=p1, p2=p2, angle=angle, override={"dimlfac": 1 / S}, dxfattribs={"layer": "KOTY"}
    ).render()


dim(at(plan, 0, 80), at(plan, 120, 80), at(plan, 0, 92))
dim(at(plan, 120, 0), at(plan, 120, 80), at(plan, 132, 0), angle=90)
dim(at(front, 0, 0), at(front, 0, 15), at(front, -12, 0), angle=90)
dim(at(front, 120, 0), at(front, 120, 40), at(front, 132, 0), angle=90)
dim(at(front, 0, 0), at(front, 80, 0), at(front, 0, -12))
msp.add_diameter_dim(
    center=at(plan, 40, 40), radius=15 * S, angle=45, override={"dimlfac": 1 / S}, dxfattribs={"layer": "KOTY"}
).render()

doc.saveas(pathlib.Path(__file__).with_name("stepped-plate-1to2.dxf"))
