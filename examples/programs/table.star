# Parametric table for the Starlark model-state GUI trial.
WIDTH = param("width", 1200, min = 600, max = 2400)
DEPTH = param("depth", 700, min = 400, max = 1200)
HEIGHT = param("height", 720, min = 400, max = 1100)
TOP = param("top_thickness", 25, min = 15, max = 60)
LEG = param("leg_size", 70, min = 40, max = 120)
INSET = param("leg_inset", 45, min = 20, max = 150)

top = board("table/top", (WIDTH, DEPTH, TOP), at = (0, 0, HEIGHT))
legs = (
    board("table/leg-front-left", (LEG, LEG, HEIGHT), at = (INSET, INSET, 0)),
    board("table/leg-front-right", (LEG, LEG, HEIGHT), at = (WIDTH - INSET - LEG, INSET, 0)),
    board("table/leg-back-left", (LEG, LEG, HEIGHT), at = (INSET, DEPTH - INSET - LEG, 0)),
    board("table/leg-back-right", (LEG, LEG, HEIGHT), at = (WIDTH - INSET - LEG, DEPTH - INSET - LEG, 0)),
)
for leg in legs:
    dowels(top, leg, dowel = "8x40", margin = 15)
