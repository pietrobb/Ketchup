# Mini domček: prízemie, spacie podkrovie, sedlová strecha, komín s kachľami,
# konzolové schody, okná, dvere a terasa. Hrebeň beží pozdĺž dlhej strany (x).
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
BRICK_C = (165, 75, 55)
TIMBER_C = (205, 165, 110)
WOOL_C = (238, 214, 120)
OSB_C = (196, 160, 105)
GYPSUM_C = (236, 236, 232)
FACADE_C = (120, 95, 70)

def layer(name, thickness, material, color, tags):
    return {"name": name, "thickness": thickness, "material": material, "color": color, "tags": tags}

def framed(name, thickness, material, spacing, tags, stud = 60):
    return {"name": name, "thickness": thickness, "material": material, "color": TIMBER_C, "tags": tags,
            "spacing": spacing, "stud": stud, "infill": "minerálna vlna", "infill_color": WOOL_C,
            "infill_tags": ["izolácia"]}

# Skladby od interiéru von (strop: od spodku nahor). Hrúbky dávajú aj rozmery konceptu.
SYSTEMS = [
    {
        "wall": [layer("sadrokartón", 12.5, "sadrokartón", GYPSUM_C, ["sadrokartón"]),
                 layer("OSB", 15, "OSB 3", OSB_C, ["OSB"]),
                 framed("stĺpiky", 140, "KVH C24", 625, ["stĺpiky"]),
                 layer("fasáda", 32.5, "drevovláknitá doska", FACADE_C, ["fasáda"])],
        "roof": [layer("sadrokartón", 12.5, "sadrokartón", GYPSUM_C, ["sadrokartón"]),
                 framed("krokvy", 180, "KVH C24", 900, ["krokvy"], stud = 80),
                 layer("debnenie", 22, "OSB 3", OSB_C, ["OSB"]),
                 layer("krytina", 35.5, "plechová krytina", ROOF_C, ["krytina"])],
        "floor": [layer("sadrokartón", 12.5, "sadrokartón", GYPSUM_C, ["sadrokartón"]),
                  framed("stropnice", 165, "KVH C24", 625, ["stropnice"], stud = 80),
                  layer("OSB", 22.5, "OSB 3", OSB_C, ["OSB"])],
    },
    {
        "wall": [layer("sadrokartón", 12.5, "sadrokartón", GYPSUM_C, ["sadrokartón"]),
                 layer("OSB", 15, "OSB 3", OSB_C, ["OSB"]),
                 framed("stĺpiky", 160, "KVH C24", 625, ["stĺpiky"]),
                 layer("fasáda", 62.5, "drevovláknitá doska", FACADE_C, ["fasáda"])],
        "roof": [layer("sadrokartón", 12.5, "sadrokartón", GYPSUM_C, ["sadrokartón"]),
                 framed("krokvy", 220, "KVH C24", 900, ["krokvy"], stud = 80),
                 layer("debnenie", 22, "OSB 3", OSB_C, ["OSB"]),
                 layer("krytina", 35.5, "plechová krytina", ROOF_C, ["krytina"])],
        "floor": [layer("sadrokartón", 12.5, "sadrokartón", GYPSUM_C, ["sadrokartón"]),
                  framed("stropnice", 165, "KVH C24", 625, ["stropnice"], stud = 80),
                  layer("OSB", 22.5, "OSB 3", OSB_C, ["OSB"])],
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

# --- základ a terasa (spoločné pre koncept aj konštrukciu) ---
box("základ/doska", (L, B, SLAB), material = "betón", color = SLAB_C)
box("terasa", (2400, 1500, SLAB - 30), at = (450, -1500, 0), material = "drevo terasa", color = WOOD_C)

# Otvory: stena, názov, od, do (pozdĺž steny vo svetových x alebo y), spodok, vrch (z).
OPENINGS = [
    ("south", "dvere", 800, 1700, SLAB, SLAB + 2100),
    ("south", "obývačka juh", 3500, 5300, SLAB + 750, SLAB + 2100),
    ("north", "kuchyňa sever", 4400, 5400, SLAB + 1000, SLAB + 2100),
    ("west", "prízemie západ", 1200, 2200, SLAB + 750, SLAB + 2100),
    ("west", "podkrovie západ", 1500, 2500, attic + 500, attic + 1600),
    ("east", "prízemie východ", 1400, 2600, SLAB + 750, SLAB + 2100),
    ("east", "podkrovie východ", 1500, 2500, attic + 500, attic + 1500),
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
        material = "drevo" if door else "izolačné sklo", color = WOOD_C if door else GLASS_C)

# --- koncept: strop / podlaha podkrovia s otvorom pre schody a komín ---
attic_floor = box("podkrovie/podlaha", (L - 2 * WALL, B - 2 * WALL, FLOOR), at = (WALL, WALL, ceiling),
                  material = "drevený strop", color = WOOD_C, tags = "koncept")

RISES = 14
rise = (attic - SLAB) / RISES
RUN = 230
STAIR_W = 800
stair_x = WALL + 200
stair_y = B - WALL - STAIR_W
stair_end = stair_x + (RISES - 1) * RUN

hole_tool = box("podkrovie/podlaha/schodisko", (stair_end - WALL + 1, STAIR_W + 1, FLOOR + 2),
                at = (WALL - 1, stair_y, ceiling - 1), tool = True)
subtract(attic_floor, hole_tool, name = "otvor schodisko")

# konzolové stupne kotvené do severnej steny
for k in range(1, RISES):
    box("schody/stupeň %d" % k, (RUN, STAIR_W, 40), at = (stair_x + (k - 1) * RUN, stair_y, SLAB + k * rise - 40),
        material = "dub", color = WOOD_C)

box("podkrovie/zábradlie", (stair_end - WALL, 40, 1000), at = (WALL, stair_y - 40, attic),
    material = "drevo", color = WOOD_C)

# --- komín s kachľami ---
CH = 450
ch_x = round_to(L * 0.42, 5)
ch_y = 1200
ch_top = ridge + lift + 450
chimney = box("komín/teleso", (CH, CH, ch_top - SLAB), at = (ch_x, ch_y, SLAB), material = "tehla", color = BRICK_C)
box("komín/krycia doska", (CH + 150, CH + 150, 80), at = (ch_x - 75, ch_y - 75, ch_top),
    material = "betón", color = SLAB_C)
box("kachle", (500, 450, 800), at = (ch_x - 25, ch_y - 450, SLAB), material = "liatina", color = (45, 45, 45))
subtract(attic_floor, chimney, name = "prestup komína")

# --- koncept: sedlová strecha ---
def roof(name, profile):
    part = extrude(name, profile = profile, distance = L + 2 * GABLE_OVER, material = "strešný plášť",
                   color = ROOF_C, tags = "koncept")
    return place(part, origin = (-GABLE_OVER, 0, 0), z = (1, 0, 0), x = (0, 1, 0))

low = eave - EAVE_OVER * tan
roof_south = roof("strecha/južná", [[-EAVE_OVER, low], [B / 2.0, ridge], [B / 2.0, ridge + lift], [-EAVE_OVER, low + lift]])
roof_north = roof("strecha/severná", [[B / 2.0, ridge], [B + EAVE_OVER, low], [B + EAVE_OVER, low + lift], [B / 2.0, ridge + lift]])
subtract(roof_south, chimney, name = "prestup komína")

# --- konštrukcia: tie isté rozmery, skladby podľa systému ---
def openings_of(side, start, reverse):
    """Otvory steny v jej súradniciach (u od `start` pozdĺž steny, v od vrchu dosky)."""
    found = []
    for wall, name, lo, hi, z0, z1 in OPENINGS:
        if wall == side:
            u = start - hi if reverse else lo - start
            found.append((u, z0 - SLAB, hi - lo, z1 - z0))
    return found

T = ["konštrukcia"]
buildup("konštrukcia/stena južná", (WALL, WALL, SLAB), (1, 0, 0), (0, 0, 1), L - 2 * WALL, system["wall"],
        height = eave - SLAB, openings = openings_of("south", WALL, False), tags = T)
buildup("konštrukcia/stena severná", (L - WALL, B - WALL, SLAB), (-1, 0, 0), (0, 0, 1), L - 2 * WALL, system["wall"],
        height = eave - SLAB, openings = openings_of("north", L - WALL, True), tags = T)
GABLE_TOP = [(0, eave - SLAB), (B / 2.0, ridge - SLAB), (B, eave - SLAB)]
buildup("konštrukcia/štít západ", (WALL, B, SLAB), (0, -1, 0), (0, 0, 1), B, system["wall"],
        top = GABLE_TOP, openings = openings_of("west", B, True), tags = T)
buildup("konštrukcia/štít východ", (L - WALL, 0, SLAB), (0, 1, 0), (0, 0, 1), B, system["wall"],
        top = GABLE_TOP, openings = openings_of("east", 0, False), tags = T)

buildup("konštrukcia/strop", (WALL, WALL, ceiling), (1, 0, 0), (0, 1, 0), L - 2 * WALL, system["floor"],
        height = B - 2 * WALL, tags = T, openings = [
            (0, stair_y - WALL, stair_end - WALL, B - WALL - stair_y),
            (ch_x - WALL, ch_y - WALL, CH, CH),
        ])

# Strešné roviny: v ide po spáde od odkvapu k hrebeňu; vrstva sa končí vo zvislej
# rovine hrebeňa na svojej vnútornej ploche, obe polovice sa tak neprekrývajú.
ROOF_LENGTH = L + 2 * GABLE_OVER
def roof_top(offset):
    end = (B / 2.0 + EAVE_OVER + offset * math.sin(a)) / math.cos(a)
    return [(0, end), (ROOF_LENGTH, end)]

chimney_v = ((ch_y + EAVE_OVER) / math.cos(a), (ch_y + CH + EAVE_OVER + ROOF_T * math.sin(a)) / math.cos(a))
buildup("konštrukcia/strecha južná", (-GABLE_OVER, -EAVE_OVER, low), (1, 0, 0), (0, math.cos(a), math.sin(a)),
        ROOF_LENGTH, system["roof"], top = roof_top, tags = T,
        openings = [(ch_x + GABLE_OVER, chimney_v[0], CH, chimney_v[1] - chimney_v[0])])
buildup("konštrukcia/strecha severná", (L + GABLE_OVER, B + EAVE_OVER, low), (-1, 0, 0), (0, -math.cos(a), math.sin(a)),
        ROOF_LENGTH, system["roof"], top = roof_top, tags = T)

# --- zariadenie (spoločné) ---
box("kuchyňa/linka", (1800, 600, 900), at = (L - WALL - 1800, B - WALL - 600, SLAB), material = "lamino", color = (235, 235, 230))

bed_x = L - WALL - 100 - 2000
bed_y = B / 2.0 - 800
box("podkrovie/posteľ rám", (2000, 1600, 250), at = (bed_x, bed_y, attic), material = "drevo", color = WOOD_C)
box("podkrovie/matrac", (1950, 1550, 180), at = (bed_x + 25, bed_y + 25, attic + 250), material = "matrac", color = (240, 240, 245))
for i, y in enumerate((bed_y + 150, bed_y + 900)):
    box("podkrovie/vankúš %d" % (i + 1), (350, 550, 100), at = (bed_x + 1550, y, attic + 430), material = "textil", color = (200, 215, 235))

print("stena %s mm, strop %s mm, strecha %s mm; podkrovie: podlaha %d, hrebeň (spodok strechy) %d mm" %
      (fmt_mm(WALL), fmt_mm(FLOOR), fmt_mm(ROOF_T), attic, ridge))
