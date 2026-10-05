# Mini domček: prízemie, spacie podkrovie, sedlová strecha, kachle s nerezovým
# izolovaným komínom, priame schody, okná, dvere a terasa. Hrebeň beží pozdĺž
# dlhej strany (x).
#
# Ten istý dom je tu dvakrát z rovnakých čísel: ako koncept (plné steny, strecha
# a strop, tag "koncept") a ako konštrukcia drevostavby (skladby stien, stropu a
# strechy podľa parametra `system`, tag "konštrukcia" a tag každej vrstvy).
# alternatives() ich nekontroluje navzájom na kolízie; zobrazujú sa scénami
# (uložené pohľady), napr. "Koncept" skryje "konštrukcia" a naopak.

L = param("length", 6200, min = 4500, max = 10000, doc = "vonkajšia dĺžka domu (x)")
B = param("width", 4000, min = 3000, max = 6000, doc = "vonkajšia šírka domu (y)")
CLEAR = param("clear_height", 2550, min = 2500, max = 3000, doc = "svetlá výška prízemia")
PITCH = param("roof_pitch", 40, min = 30, max = 50, doc = "sklon strechy v stupňoch")
KNEE = param("knee_wall", 1000, min = 400, max = 1500, doc = "nadmurovka nad podlahou podkrovia")
SYSTEM = param("system", 0, min = 0, max = 1, doc = "skladba: 0 drevostavba 200 mm, 1 zateplená drevostavba 250 mm")

WALL_C = (226, 208, 172)
SLAB_C = (150, 150, 150)
ROOF_C = (72, 72, 78)
GLASS_C = (150, 200, 230)
WOOD_C = (150, 100, 60)
STEEL_C = (190, 192, 196)
TIMBER_C = (205, 165, 110)
WOOL_C = (238, 214, 120)
OSB_C = (196, 160, 105)
GYPSUM_C = (236, 236, 232)
FACADE_C = (120, 95, 70)

def layer(name, thickness, material, color, tags):
    return {"name": name, "thickness": thickness, "material": material, "color": color, "tags": tags}

def framed(name, thickness, material, spacing, tags, stud = 60, hanger = None, header = None):
    layer = {"name": name, "thickness": thickness, "material": material, "color": TIMBER_C, "tags": tags,
             "spacing": spacing, "stud": stud, "infill": "minerálna vlna", "infill_color": WOOL_C,
             "infill_tags": ["izolácia"]}
    if hanger != None:
        layer["hanger"] = hanger
    if header != None:
        layer["header"] = header
    return layer

# Spojovací materiál (BB-TECHNIK Banská Bystrica). Všetko je poskladané na sebe;
# nosné spoje sú len tam, kde prvok inak nemá na čom ležať (výmeny pri otvoroch).
# Ostatné spoje držia prvky proti posunu a nadvihnutiu.
# Únosnosť strmeňa nemáme z tabuľky výrobcu, preto sú jeho spoje v reporte „neoverené“
# (CONNECTOR_RATINGS v knižnici). Drevo na drevo: arunda(stropnica, výmena, "50 B").
HANGER = "strmeň vonkajší 80x120x2 typ A"
RAFTER_TIE = "krokvová spojka 30x210 VORMANN"
RIDGE_SCREW = "vrut tesársky 8x260/80 TX40 tanierová hlava"
SILL_ANCHOR = "kotva do betónu M12"
POST_BASE = "kotevná pätka stĺpika"
# Preklad nad otvorom v stene: hranol na výšku (posudok EC5 vyžaduje viac než 60 mm stĺpika).
HEADER = 200

# Skladby od interiéru von (strop: od spodku nahor). Hrúbky dávajú aj rozmery konceptu.
SYSTEMS = [
    {
        "wall": [layer("sadrokartón", 12.5, "sadrokartón", GYPSUM_C, ["sadrokartón"]),
                 layer("OSB", 15, "OSB 3", OSB_C, ["OSB"]),
                 framed("stĺpiky", 140, "KVH C24", 625, ["stĺpiky"], header = HEADER),
                 layer("fasáda", 32.5, "drevovláknitá doska", FACADE_C, ["fasáda"])],
        "roof": [layer("sadrokartón", 12.5, "sadrokartón", GYPSUM_C, ["sadrokartón"]),
                 framed("krokvy", 180, "KVH C24", 900, ["krokvy"], stud = 80),
                 layer("debnenie", 22, "OSB 3", OSB_C, ["OSB"]),
                 layer("krytina", 35.5, "plechová krytina", ROOF_C, ["krytina"])],
        "floor": [layer("sadrokartón", 12.5, "sadrokartón", GYPSUM_C, ["sadrokartón"]),
                  framed("stropnice", 200, "KVH C24", 625, ["stropnice"], stud = 80, hanger = HANGER),
                  layer("OSB", 22.5, "OSB 3", OSB_C, ["OSB", "podlaha"])],
    },
    {
        "wall": [layer("sadrokartón", 12.5, "sadrokartón", GYPSUM_C, ["sadrokartón"]),
                 layer("OSB", 15, "OSB 3", OSB_C, ["OSB"]),
                 framed("stĺpiky", 160, "KVH C24", 625, ["stĺpiky"], header = HEADER),
                 layer("fasáda", 62.5, "drevovláknitá doska", FACADE_C, ["fasáda"])],
        "roof": [layer("sadrokartón", 12.5, "sadrokartón", GYPSUM_C, ["sadrokartón"]),
                 framed("krokvy", 220, "KVH C24", 900, ["krokvy"], stud = 80),
                 layer("debnenie", 22, "OSB 3", OSB_C, ["OSB"]),
                 layer("krytina", 35.5, "plechová krytina", ROOF_C, ["krytina"])],
        "floor": [layer("sadrokartón", 12.5, "sadrokartón", GYPSUM_C, ["sadrokartón"]),
                  framed("stropnice", 200, "KVH C24", 625, ["stropnice"], stud = 80, hanger = HANGER),
                  layer("OSB", 22.5, "OSB 3", OSB_C, ["OSB", "podlaha"])],
    },
]
system = SYSTEMS[SYSTEM]

def total(layers):
    return sum([item["thickness"] for item in layers])

WALL = total(system["wall"])      # obvodová stena
FLOOR = total(system["floor"])    # strop prízemia = podlaha podkrovia
ROOF_T = total(system["roof"])    # strešný plášť kolmo na sklon
SLAB = 250                        # základová doska
EAVE_OVER = 500                   # presah strechy na odkvape (vodorovne)
GABLE_OVER = 400                  # presah strechy nad štítmi

a = math.radians(PITCH)
tan = math.tan(a)
ceiling = SLAB + CLEAR           # spodok stropu prízemia
attic = ceiling + FLOOR          # podlaha podkrovia
eave = attic + KNEE              # spodok strechy na vonkajšej hrane steny
ridge = eave + B / 2.0 * tan     # spodok strechy v hrebeni
lift = ROOF_T / math.cos(a)      # zvislá hrúbka strešného plášťa

alternatives(["koncept", "konštrukcia"])
# Nosné prvky musia preniesť svoju tiaž až na základ: ležať zhora na nesenom prvku
# (trám pod oboma koncami) alebo visieť na nosnom spoji (joint(..., bearing = True)).
load_path(only = ["stĺpiky", "stropnice", "krokvy"], carriers = ["OSB"], name = "nosná konštrukcia")

# Zaťaženia: vlastná tiaž konštrukcie (hustoty z knižnice), úžitkové na podlahe podkrovia,
# údržba strechy a sneh na krytine (na pôdorys). Sneh na zemi sk pre miesto stavby treba
# odčítať z mapy STN EN 1991-1-3/NA; kým nie je zadaný (0), prvky pod strechou sú neúplné.
SNOW_SK = param("snow_sk", 0, min = 0, max = 10, doc = "sneh na zemi sk v kN/m² podľa miesta stavby (0 = nezadané)")
self_weight(["konštrukcia"])
area_load("úžitkové podkrovie", kind = "imposed", kn_m2 = IMPOSED_LOADS["A"][0], on = ["podlaha"],
          source = IMPOSED_LOADS["A"][1])
area_load("údržba strechy", kind = "roof", kn_m2 = IMPOSED_LOADS["H"][0], on = ["krytina"],
          source = IMPOSED_LOADS["H"][1])
area_load("sneh", kind = "snow", kn_m2 = snow_load(SNOW_SK, PITCH), on = ["krytina"],
          source = "EN 1991-1-3 5.2(3), tab. 5.2; sk = %s kN/m²" % SNOW_SK)
# Posudok prútov podľa EC5: KVH C24, vnútorné prostredie (trieda prevádzky 1).
timber_design({"KVH C24": "C24", "BSH GL24h": "GL24h"}, service_class = 1)

# --- základ a terasa (spoločné pre koncept aj konštrukciu) ---
box("základ/doska", (L, B, SLAB), material = "betón", color = SLAB_C)
box("terasa", (2400, 1500, SLAB - 30), at = (450, -1500, 0), material = "drevo terasa", color = WOOD_C)

# Otvory: stena, názov, od, do (pozdĺž steny vo svetových x alebo y), spodok, vrch (z).
# Západný štít pri nástupe na schody a východný štít nad posteľou okno v danom
# podlaží nemajú.
OPENINGS = [
    ("south", "dvere", 800, 1700, SLAB, SLAB + 2100),
    ("south", "obývačka juh", 3500, 5300, SLAB + 750, SLAB + 2100),
    ("north", "kuchyňa sever", 4400, 5400, SLAB + 1000, SLAB + 2100),
    ("west", "podkrovie západ", 1500, 2500, attic + 500, attic + 1600),
    ("east", "prízemie východ", 1400, 2600, SLAB + 750, SLAB + 2100),
]

# --- koncept: plné steny ---
south = box("steny/južná", (L - 2 * WALL, WALL, eave + 400 - SLAB), at = (WALL, 0, SLAB),
            material = "drevostavba", color = WALL_C, tags = "koncept")
trim(south, point = (0, 0, eave), normal = (0, -math.sin(a), math.cos(a)), name = "pod strechou")
north = box("steny/severná", (L - 2 * WALL, WALL, eave + 400 - SLAB), at = (WALL, B - WALL, SLAB),
            material = "drevostavba", color = WALL_C, tags = "koncept")
trim(north, point = (0, B, eave), normal = (0, math.sin(a), math.cos(a)), name = "pod strechou")

def gable(name, x):
    """Štítová stena: päťuholník v rovine (y, z), hrúbka WALL pozdĺž x."""
    part = extrude(name, profile = [[0, SLAB], [B, SLAB], [B, eave], [B / 2.0, ridge], [0, eave]],
                   distance = WALL, material = "drevostavba", color = WALL_C, tags = "koncept")
    return place(part, origin = (x, 0, 0), z = (1, 0, 0), x = (0, 1, 0))

CONCEPT_WALLS = {
    "south": (south, "y", 0),
    "north": (north, "y", B - WALL),
    "west": (gable("steny/štít západ", 0), "x", 0),
    "east": (gable("steny/štít východ", L - WALL), "x", L - WALL),
}

# Otvor vo všetkých reprezentáciách: výplň (sklo, krídlo dverí) je spoločná.
for side, name, lo, hi, z0, z1 in OPENINGS:
    wall, axis, wall_min = CONCEPT_WALLS[side]
    door = name == "dvere"
    thickness = 60 if door else 24
    if axis == "y":
        tool_at, tool_size = (lo, wall_min - 1, z0), (hi - lo, WALL + 2, z1 - z0)
        fill_at, fill_size = (lo, wall_min + (WALL - thickness) / 2.0, z0), (hi - lo, thickness, z1 - z0)
    else:
        tool_at, tool_size = (wall_min - 1, lo, z0), (WALL + 2, hi - lo, z1 - z0)
        fill_at, fill_size = (wall_min + (WALL - thickness) / 2.0, lo, z0), (thickness, hi - lo, z1 - z0)
    tool = box(part_info(wall).name + "/" + name, tool_size, at = tool_at, tool = True)
    subtract(wall, tool, name = name)
    box("dvere/krídlo" if door else "okná/" + name, fill_size, at = fill_at,
        material = "drevo" if door else "izolačné sklo", color = WOOD_C if door else GLASS_C,
        tags = [] if door else ["okno"])

# --- koncept: strop / podlaha podkrovia s otvormi pre schody a komín ---
attic_floor = box("podkrovie/podlaha", (L - 2 * WALL, B - 2 * WALL, FLOOR), at = (WALL, WALL, ceiling),
                  material = "drevený strop", color = WOOD_C, tags = "koncept")

# Priame schody na dvoch schodniciach stúpajú od západu na východ blízko stredu
# domu: pred prvým stupňom je voľná nástupná plocha a hore vychádzajú pod
# hrebeňom, kde je nad nimi aj nad výstupom dosť miesta na hlavu.
RISES = 14
rise = (attic - SLAB) / RISES
RUN = 230
STAIR_W = 800
STRINGER = 50
LANDING = 900
stair_x = WALL + LANDING
stair_y = B / 2.0 - 100
stair_end = stair_x + (RISES - 1) * RUN
hole_y = (stair_y - STRINGER, stair_y + STAIR_W + STRINGER)

hole_tool = box("podkrovie/podlaha/schodisko", (stair_end - stair_x, hole_y[1] - hole_y[0], FLOOR + 2),
                at = (stair_x, hole_y[0], ceiling - 1), tool = True)
subtract(attic_floor, hole_tool, name = "otvor schodisko")

treads = [box("schody/stupeň %d" % k, (RUN, STAIR_W, 40), at = (stair_x + (k - 1) * RUN, stair_y, SLAB + k * rise - 40),
              material = "dub", color = WOOD_C) for k in range(1, RISES)]

# Schodnica: pás nad a pod čiarou hrán stupňov, dole na doske, hore zvislým
# rezom dosadá na hranu stropu.
slope = rise / RUN
DEEP = 280
stringer = [(stair_x, SLAB), (stair_x + (DEEP - rise) / slope, SLAB), (stair_end, attic - DEEP),
            (stair_end, attic), (stair_end - 50 / slope, attic), (stair_x, SLAB + rise + 50)]
for side, y in (("južná", hole_y[0]), ("severná", stair_y + STAIR_W)):
    part = extrude("schody/schodnica %s" % side, profile = [[p[0], p[1]] for p in stringer], distance = STRINGER,
                   material = "dub", color = WOOD_C)
    place(part, origin = (0, y + STRINGER, 0), z = (0, -1, 0), x = (1, 0, 0))

for side, size, at in (("juh", (stair_end - stair_x, 40, 1000), (stair_x, hole_y[0] - 40, attic)),
                       ("sever", (stair_end - stair_x, 40, 1000), (stair_x, hole_y[1], attic)),
                       ("západ", (40, hole_y[1] - hole_y[0] + 80, 1000), (stair_x - 40, hole_y[0] - 40, attic))):
    box("podkrovie/zábradlie %s" % side, size, at = at, material = "drevo", color = WOOD_C)

stair_space("schody", treads, attic, landing = LANDING)

# --- kachle a nerezový izolovaný komín ---
# Z vrchu kachlí ide dymovod do izolovaného dvojplášťového komína; ten nesie
# stropný prechod na podlahe podkrovia a prechádza strechou 650 mm nad hrebeň.
# Drevo je od plášťa komína odsadené o FIRE.
STOVE = (450, 500, 800)
stove_x = round_to(L * 0.49, 5)
stove_y = 1025
FLUE_D = 250                          # vonkajší plášť izolovaného komína
PIPE_D = 150                          # dymovod
FIRE = 50                             # odstup dreva od plášťa komína
fx, fy = stove_x + STOVE[0] / 2.0, stove_y + STOVE[1] / 2.0
flue_join = ceiling - 500
flue_top = ridge + lift + 650
hole_x = (fx - FLUE_D / 2.0 - FIRE, fx + FLUE_D / 2.0 + FIRE)
hole_d = (fy - FLUE_D / 2.0 - FIRE, fy + FLUE_D / 2.0 + FIRE)

stove = box("kachle", STOVE, at = (stove_x, stove_y, SLAB), material = "liatina", color = (45, 45, 45))
service_space(stove, "x+", 1000, height = 1800, name = "kachle/miesto na prikladanie")

def round_profile(diameter):
    return ellipse(diameter / 2.0, diameter / 2.0)

extrude("komín/dymovod", profile = round_profile(PIPE_D), distance = flue_join - SLAB - STOVE[2],
        at = (fx, fy, SLAB + STOVE[2]), material = "oceľ", color = (40, 40, 40))
extrude("komín/izolovaný komín", profile = round_profile(FLUE_D), distance = flue_top - flue_join,
        at = (fx, fy, flue_join), material = "nerez, dvojplášťový izolovaný", color = STEEL_C)
PASS = hole_x[1] - hole_x[0] + 200
collar = box("komín/stropný prechod", (PASS, PASS, 3), at = (fx - PASS / 2.0, fy - PASS / 2.0, attic),
             material = "nerez", color = STEEL_C)
subtract(collar, extrude("komín/stropný prechod/otvor", profile = round_profile(FLUE_D), distance = 5,
                         at = (fx, fy, attic - 1), tool = True), name = "otvor komína")
flue_hole = box("komín/prestup", (hole_x[1] - hole_x[0], hole_d[1] - hole_d[0], flue_top - SLAB),
                at = (hole_x[0], hole_d[0], SLAB), tool = True)
subtract(attic_floor, flue_hole, name = "prestup komína")

# --- koncept: sedlová strecha ---
def roof(name, profile):
    part = extrude(name, profile = profile, distance = L + 2 * GABLE_OVER, material = "strešný plášť",
                   color = ROOF_C, tags = "koncept")
    return place(part, origin = (-GABLE_OVER, 0, 0), z = (1, 0, 0), x = (0, 1, 0))

low = eave - EAVE_OVER * tan
roof_south = roof("strecha/južná", [[-EAVE_OVER, low], [B / 2.0, ridge], [B / 2.0, ridge + lift], [-EAVE_OVER, low + lift]])
roof_north = roof("strecha/severná", [[B / 2.0, ridge], [B + EAVE_OVER, low], [B + EAVE_OVER, low + lift], [B / 2.0, ridge + lift]])
subtract(roof_south, flue_hole, name = "prestup komína")

# --- konštrukcia: tie isté rozmery, skladby podľa systému ---
def openings_of(side, start, reverse, base = SLAB, below = None):
    """Otvory steny medzi výškami base a below v jej súradniciach (u od `start`
    pozdĺž steny, v od base)."""
    found = []
    for wall, name, lo, hi, z0, z1 in OPENINGS:
        if wall == side and z0 >= base and (below == None or z1 <= below):
            u = start - hi if reverse else lo - start
            found.append((u, z0 - base, hi - lo, z1 - z0))
    return found

T = ["konštrukcia"]
KROV = T + ["krokvy"]
cos_a = math.cos(a)
sin_a = math.sin(a)

# Krov je poskladaný na sebe: krokvy sú hore osedlané na plochom vrchu hrebeňovej
# väznice a v hrebeni sa stretnú zvislým rezom, dole sú osedlané na pomúrnici.
# Väznica aj pomúrnica prechádzajú kapsami v štítoch až do presahu, takže nesú aj
# krajné krokvy v presahu. Kapsy majú pod oboma koncami stĺpik.
ROOF_LAYERS = system["roof"]
G = ROOF_LAYERS[0]["thickness"]       # podhľad pod krokvami
RAFTER = ROOF_LAYERS[1]
R = RAFTER["thickness"]               # výška krokvy kolmo na sklon
RAFTER_W = RAFTER["stud"]             # šírka krokvy
WALL_LAYERS = system["wall"]
FACADE = WALL_LAYERS[-1]["thickness"]
STUDS = WALL_LAYERS[2]["thickness"]
PLATE_H = 100                         # výška pomúrnice
RIDGE_W = 160                         # šírka hrebeňovej väznice
RIDGE_H = 280                         # výška hrebeňovej väznice

def roof_z(d, o):
    """Výška roviny strešného plášťa vo výške o kolmo nad jeho spodkom, d vodorovne
    od vonkajšieho líca obvodovej steny smerom k hrebeňu."""
    return eave + d * tan + o / cos_a

seat = min(STUDS, R / 3.0 / tan)      # osedlanie najviac do tretiny výšky krokvy
plate_top = roof_z(FACADE + seat, G)
plate_bottom = plate_top - PLATE_H
d_ridge = B / 2.0 - RIDGE_W / 2.0     # bok väznice
apex = roof_z(B / 2.0, G + R)         # hrebeň krokiev pod debnením
ridge_top = roof_z(B / 2.0, G)        # vrch väznice, na ňom sú krokvy osedlané
ridge_bottom = ridge_top - RIDGE_H

# Platformová drevostavba: steny prízemia končia hornou pásnicou, na nej leží strop
# (stropnice s obvodovým vencom a OSB) a na jeho podlahe stoja nadmurovky a štíty
# podkrovia. Fasáda ide cez obe podlažia a kryje čelo stropu. Štíty sú medzi
# fasádami pozdĺžnych stien, takže fasáda prechádza aj cez nároží.
INNER = WALL_LAYERS[:-1]              # sadrokartón, OSB, stĺpiky
FLOOR_LAYERS = system["floor"]
deck = ceiling + FLOOR_LAYERS[0]["thickness"]   # vrch pásnice prízemia = spodok stropníc

def floor_holes(x0, y0):
    return [(stair_x - x0, hole_y[0] - y0, stair_end - stair_x, hole_y[1] - hole_y[0]),
            (hole_x[0] - x0, hole_d[0] - y0, hole_x[1] - hole_x[0], hole_d[1] - hole_d[0])]

# Nadmurovka: stĺpiky pod pomúrnicou, vnútorné vrstvy pod podhľadom strechy.
def knee_top(offset):
    start = 0.0
    for k in range(len(INNER)):
        if abs(start - offset) < 0.001:
            z = plate_bottom if k == 2 else roof_z(WALL - offset - INNER[k]["thickness"], 0)
            return [(0, z - attic), (L - 2 * WALL, z - attic)]
        start += INNER[k]["thickness"]
    fail("wall layer at offset %s" % offset)

def gable_top(base):
    """Vrch štítu pod krokvami medzi fasádami pozdĺžnych stien."""
    return [(0, roof_z(FACADE, G) - base), (B / 2.0 - FACADE, roof_z(B / 2.0, G) - base),
            (B - 2 * FACADE, roof_z(FACADE, G) - base)]

def ridge_pocket(u0, base):
    return (B / 2.0 - RIDGE_W / 2.0 - u0, ridge_bottom - base, RIDGE_W, apex - ridge_bottom + 200)

def plate_pockets(u0, base, length):
    """Kapsy pre obe pomúrnice v štíte, ktorého u ide od u0 po u0 + length naprieč domom."""
    return [(d - u0, plate_bottom - base, STUDS, apex - plate_bottom)
            for d in (FACADE, length + 2 * u0 - FACADE - STUDS)]

GROUND = deck - SLAB
SIDES = [
    # stena, prízemie, podkrovie, (začiatok, smer), dĺžka, otvory od
    ("south", "prízemie/stena južná", "podkrovie/nadmurovka južná", (WALL, WALL), (1, 0, 0), L - 2 * WALL, WALL, False),
    ("north", "prízemie/stena severná", "podkrovie/nadmurovka severná", (L - WALL, B - WALL), (-1, 0, 0), L - 2 * WALL, L - WALL, True),
    ("west", "prízemie/stena západ", "štít západ", (WALL, B - FACADE), (0, -1, 0), B - 2 * FACADE, B - FACADE, True),
    ("east", "prízemie/stena východ", "štít východ", (L - WALL, FACADE), (0, 1, 0), B - 2 * FACADE, FACADE, False),
]
for side, ground, upper, (x, y), along, length, start, reverse in SIDES:
    buildup("konštrukcia/" + ground, (x, y, SLAB), along, (0, 0, 1), length, INNER, height = GROUND,
            openings = openings_of(side, start, reverse, SLAB, deck), tags = T,
            anchor = {"to": "základ/doska", "fastener": SILL_ANCHOR, "spacing": 1000})
    attic_openings = openings_of(side, start, reverse, attic)
    if side in ("south", "north"):
        buildup("konštrukcia/" + upper, (x, y, attic), along, (0, 0, 1), length, INNER, top = knee_top,
                openings = attic_openings, tags = T)
    else:
        buildup("konštrukcia/" + upper, (x, y, attic), along, (0, 0, 1), length, INNER, top = gable_top(attic),
                openings = attic_openings + [ridge_pocket(FACADE, attic)] + plate_pockets(FACADE, attic, length),
                tags = T)

FACADE_LAYERS = WALL_LAYERS[-1:]
eave_line = roof_z(0, G) - SLAB
for side, (x, y), along, length, start, reverse in (
        ("south", (FACADE, FACADE), (1, 0, 0), L - 2 * FACADE, FACADE, False),
        ("north", (L - FACADE, B - FACADE), (-1, 0, 0), L - 2 * FACADE, L - FACADE, True)):
    buildup("konštrukcia/fasáda %s" % ("južná" if side == "south" else "severná"), (x, y, SLAB), along, (0, 0, 1),
            length, FACADE_LAYERS, height = eave_line, openings = openings_of(side, start, reverse), tags = T)
for side, name, (x, y), along, start, reverse in (("west", "západ", (FACADE, B), (0, -1, 0), B, True),
                                                  ("east", "východ", (L - FACADE, 0), (0, 1, 0), 0, False)):
    buildup("konštrukcia/fasáda %s" % name, (x, y, SLAB), along, (0, 0, 1), B, FACADE_LAYERS,
            top = [(0, eave_line), (B / 2.0, roof_z(B / 2.0, G) - SLAB), (B, eave_line)],
            openings = openings_of(side, start, reverse) + [ridge_pocket(0, SLAB)] + plate_pockets(0, SLAB, B), tags = T)

# Strop: stropnice ležia na hornej pásnici stien prízemia, obvodový veniec (krajné
# stropnice a čelné fošne) stojí na jej vonkajšej časti. Podhľad je len v interiéri.
# Výmeny pozdĺž otvoru schodiska sú dlhé ako schody a nesú skrátené stropnice,
# preto nevisia len z boku na krajných stropniciach: každý ich koniec spolu s
# krajnou stropnicou leží zhora na stĺpiku postavenom na základovej doske.
POST = 80
stair_posts = [(name, x - POST, y) for name, x in (("západ", stair_x), ("východ", stair_end))
               for y in (hole_y[0] - POST, hole_y[1])]
for name, x, y in stair_posts:
    post = box("konštrukcia/strop/stĺpik pod výmenou %s %s" % ("juh" if y < hole_y[0] else "sever", name),
               (2 * POST, POST, deck - SLAB), at = (x, y, SLAB), material = "KVH C24", color = TIMBER_C,
               tags = T + ["stĺpiky"])
    joint(post, "základ/doska", kind = "anchor", fastener = POST_BASE, fasteners = [(x + POST, y + POST / 2.0, SLAB)])
buildup("konštrukcia/podhľad prízemia", (WALL, WALL, ceiling), (1, 0, 0), (0, 1, 0), L - 2 * WALL, FLOOR_LAYERS[:1],
        height = B - 2 * WALL, openings = floor_holes(WALL, WALL) +
        [(x - WALL, y - WALL, 2 * POST, POST) for _, x, y in stair_posts], tags = T)
buildup("konštrukcia/strop", (FACADE, FACADE, deck), (1, 0, 0), (0, 1, 0), L - 2 * FACADE, FLOOR_LAYERS[1:],
        height = B - 2 * FACADE, openings = floor_holes(FACADE, FACADE), tags = T)

ROOF_LENGTH = L + 2 * GABLE_OVER

def on_side(points, north):
    """Obrys (d, z) jednej strešnej polovice ako (y, z); severná je zrkadlová."""
    if north:
        return [[B - p[0], p[1]] for p in reversed(points)]
    return [[p[0], p[1]] for p in points]

def along_x(name, points, x0, x1, material, color, tags):
    """Hranol s obrysom v rovine (y, z) od x0 po x1."""
    part = extrude(name, profile = points, distance = x1 - x0, material = material, color = color, tags = tags)
    return place(part, origin = (x0, 0, 0), z = (1, 0, 0), x = (0, 1, 0))

def slab_between(d0, d1, o0, o1):
    """Obrys (d, z) pásu strešného plášťa od o0 po o1 so zvislými koncami v d0 a d1."""
    return [(d0, roof_z(d0, o0)), (d1, roof_z(d1, o0)), (d1, roof_z(d1, o1)), (d0, roof_z(d0, o1))]

def rafter_outline(start, end, seated):
    """Krokva od pätky na odkvape (start None, rez kolmo na sklon) alebo od zvislého
    rezu v start po zvislý rez v end, osedlaná na pomúrnici, ak ju prekrýva. Krokva
    končiaca pri väznici je na nej osedlaná a končí zvislým rezom v hrebeni."""
    if start == None:
        bottom = [(-EAVE_OVER - G * sin_a, low + G * cos_a)]
        top = [(-EAVE_OVER - (G + R) * sin_a, low + (G + R) * cos_a)]
    else:
        bottom = [(start, roof_z(start, G))]
        top = [(start, roof_z(start, G + R))]
    if seated:
        bottom += [(FACADE, roof_z(FACADE, G)), (FACADE, plate_top), (FACADE + seat, plate_top)]
    if abs(end - d_ridge) < 0.001:
        return bottom + [(d_ridge, roof_z(d_ridge, G)), (d_ridge, ridge_top), (B / 2.0, ridge_top), (B / 2.0, apex)] + top
    return bottom + [(end, roof_z(end, G)), (end, roof_z(end, G + R))] + top

# Krokvy: krajné v presahu štítov, nad každým štítom a medzi štítmi rovnomerne
# najviac po RAFTER["spacing"].
on_gables = [(WALL - RAFTER_W) / 2.0, L - (WALL + RAFTER_W) / 2.0]
bays = int((on_gables[1] - on_gables[0]) / RAFTER["spacing"] + 0.999)
RAFTERS = ([-GABLE_OVER, on_gables[0]] +
           [on_gables[0] + k * (on_gables[1] - on_gables[0]) / bays for k in range(1, bays)] +
           [on_gables[1], L + GABLE_OVER - RAFTER_W])

# Prestup komína v južnej polovici: výmeny pod a nad komínom ohraničia otvor
# medzi susednými krokvami a nesú krokvy, ktoré otvor preruší.
cut = [x for x in RAFTERS if x < hole_x[1] and hole_x[0] < x + RAFTER_W]
trim_x = (max([x + RAFTER_W for x in RAFTERS if x + RAFTER_W <= hole_x[0]]),
          min([x for x in RAFTERS if x >= hole_x[1]]))

ridge_beam = along_x("konštrukcia/hrebeňová väznica",
                     [[d_ridge, ridge_bottom], [B - d_ridge, ridge_bottom], [B - d_ridge, ridge_top], [d_ridge, ridge_top]],
                     -GABLE_OVER, L + GABLE_OVER, "BSH GL24h", TIMBER_C, KROV)

def middle(a, b):
    """Stred plochy, ktorou sa a a b dotýkajú."""
    touch = contact(a, b)
    if touch == None:
        fail("%s sa nedotýka %s" % (part_info(a).name, part_info(b).name))
    return vec_scale(vec_add(touch.min, touch.max), 0.5)

def roof_frame(side, north):
    name = "konštrukcia/strecha %s" % side
    plate = box("konštrukcia/pomúrnica %s" % side, (ROOF_LENGTH, STUDS, PLATE_H),
                at = (-GABLE_OVER, B - FACADE - STUDS if north else FACADE, plate_bottom),
                material = RAFTER["material"], color = TIMBER_C, tags = KROV)
    rafters = []
    for k in range(len(RAFTERS)):
        x = RAFTERS[k]
        if not north and x in cut:
            pieces = [rafter_outline(None, hole_d[0] - RAFTER_W, True),
                      rafter_outline(hole_d[1] + RAFTER_W, d_ridge, False)]
        else:
            pieces = [rafter_outline(None, d_ridge, True)]
        for j in range(len(pieces)):
            label = "krokva %d" % (k + 1) if len(pieces) == 1 else "krokva %d%s" % (k + 1, "ab"[j])
            rafter = along_x("%s/%s" % (name, label), on_side(pieces[j], north), x, x + RAFTER_W,
                             RAFTER["material"], TIMBER_C, KROV)
            rafters.append(rafter)
            if j == 0:
                # Krokvové spojky z oboch strán držia krokvu na pomúrnici proti posunu a nadvihnutiu.
                m = middle(rafter, plate)
                for sign, hand in ((-1, "ľavá"), (1, "pravá")):
                    joint(rafter, plate, kind = "krokvová spojka " + hand, fastener = RAFTER_TIE + " " + hand,
                          fasteners = [(m[0] + sign * RAFTER_W / 2.0, m[1], m[2])])
            if j == len(pieces) - 1:
                joint(rafter, ridge_beam, kind = "vrut", fastener = RIDGE_SCREW, fasteners = [middle(rafter, ridge_beam)])
    if not north:
        for j, (d0, d1) in enumerate([(hole_d[0] - RAFTER_W, hole_d[0]), (hole_d[1], hole_d[1] + RAFTER_W)]):
            trimmer = along_x("%s/výmena %s" % (name, ("dolná", "horná")[j]), on_side(slab_between(d0, d1, G, G + R), north),
                              trim_x[0], trim_x[1], RAFTER["material"], TIMBER_C, KROV)
            # Výmena visí v strmeňoch na susedných krokvách a nesie prerušené krokvy.
            # Koniec výmeny pri boku krokvy: krokva nesie výmenu; koniec krokvy pri výmene: výmena nesie krokvu.
            for rafter in rafters:
                touch = contact(trimmer, rafter)
                if touch != None:
                    carried, carrier = (trimmer, rafter) if abs(touch.normal[0]) > 0.7 else (rafter, trimmer)
                    joint(carried, carrier, kind = "hanger", fastener = HANGER, bearing = True,
                          fasteners = [middle(trimmer, rafter)], rating = connector_rating_of(HANGER))
    count = 0
    for k in range(len(RAFTERS) - 1):
        x0, x1 = max(RAFTERS[k] + RAFTER_W, WALL), min(RAFTERS[k + 1], L - WALL)
        if x1 - x0 < 1:
            continue
        spans = [(WALL, d_ridge)]
        if not north and x0 >= trim_x[0] - 0.001 and x1 <= trim_x[1] + 0.001:
            spans = [(WALL, hole_d[0] - RAFTER_W), (hole_d[1] + RAFTER_W, d_ridge)]
        for d0, d1 in spans:
            count += 1
            along_x("%s/izolácia %d" % (name, count), on_side(slab_between(d0, d1, G, G + R), north), x0, x1,
                    RAFTER["infill"], RAFTER["infill_color"], T + RAFTER["infill_tags"])

roof_frame("južná", False)
roof_frame("severná", True)

# Plášť: podhľad pod krokvami len v interiéri, debnenie a krytina na krokvách.
# Vrstva v je po spáde; debnenie a krytina končia vnútornou plochou v rovine hrebeňa.
def chimney_hole(x0, d0, o0, thickness):
    v0 = (hole_d[0] - d0 + o0 * sin_a) / cos_a
    v1 = (hole_d[1] - d0 + (o0 + thickness) * sin_a) / cos_a
    return (hole_x[0] - x0, v0, hole_x[1] - hole_x[0], v1 - v0)

def lining(side, north):
    sign = -1 if north else 1
    start = (WALL if not north else L - WALL, B - WALL if north else WALL, roof_z(WALL, 0))
    length = (B / 2.0 - RIDGE_W / 2.0 - WALL) / cos_a
    buildup("konštrukcia/strecha %s/podhľad" % side, start, (sign, 0, 0), (0, sign * cos_a, sin_a),
            L - 2 * WALL, [ROOF_LAYERS[0]], top = [(0, length), (L - 2 * WALL, length)], tags = T,
            openings = [] if north else [chimney_hole(WALL, WALL, 0, G)])

def cladding(side, north):
    sign = -1 if north else 1
    o0 = G + R
    edge = (-EAVE_OVER - o0 * sin_a, low + o0 * cos_a)
    start = (L + GABLE_OVER if north else -GABLE_OVER, B - edge[0] if north else edge[0], edge[1])
    sheets = ROOF_LAYERS[2:]
    def top(offset):
        end = (B / 2.0 + EAVE_OVER + (o0 + offset) * sin_a) / cos_a
        return [(0, end), (ROOF_LENGTH, end)]
    buildup("konštrukcia/strecha %s" % side, start, (sign, 0, 0), (0, sign * cos_a, sin_a),
            ROOF_LENGTH, sheets, top = top, tags = T,
            openings = [] if north else [chimney_hole(-GABLE_OVER, -EAVE_OVER, o0, ROOF_T - o0)])

for side, north in (("južná", False), ("severná", True)):
    lining(side, north)
    cladding(side, north)

# Hrebenáč prekrýva styk krytiny oboch polovíc.
CAP = 200
cap = [(B / 2.0 - CAP, roof_z(B / 2.0 - CAP, ROOF_T)), (B / 2.0, roof_z(B / 2.0, ROOF_T))]
cap_bottom = cap + [(B - p[0], p[1]) for p in reversed(cap[:-1])]
along_x("konštrukcia/hrebenáč",
        [[p[0], p[1]] for p in cap_bottom] + [[p[0], p[1] + 3 / cos_a] for p in reversed(cap_bottom)],
        -GABLE_OVER, L + GABLE_OVER, "plechová krytina", ROOF_C, T + ["krytina"])

# --- zariadenie (spoločné) ---
box("kuchyňa/linka", (1800, 600, 900), at = (L - WALL - 1800, B - WALL - 600, SLAB), material = "lamino", color = (235, 235, 230))

# Posteľ v podkroví vedľa otvoru schodiska; nad ňou ani pri nej nesmie byť okno.
BED_W = 1400
bed_x = L - WALL - 100 - 2000
bed_y = hole_y[0] - 40 - 100 - BED_W
bed = box("podkrovie/posteľ rám", (2000, BED_W, 250), at = (bed_x, bed_y, attic), material = "drevo", color = WOOD_C)
box("podkrovie/matrac", (1950, BED_W - 50, 180), at = (bed_x + 25, bed_y + 25, attic + 250), material = "matrac", color = (240, 240, 245))
for i, y in enumerate((bed_y + 100, bed_y + BED_W - 650)):
    box("podkrovie/vankúš %d" % (i + 1), (350, 550, 100), at = (bed_x + 1550, y, attic + 430), material = "textil", color = (200, 215, 235))
no_window_over(bed, tag = "okno")

print("stena %s mm, strop %s mm, strecha %s mm; podkrovie: podlaha %d, hrebeň (spodok strechy) %d mm" %
      (fmt_mm(WALL), fmt_mm(FLOOR), fmt_mm(ROOF_T), attic, ridge))
