# Malý dom pre testy: tie isté vrstvy, názvy a parameter dĺžky ako tiny-house.star,
# ale desiatky dielov namiesto stoviek. Pre testy, ktorým stačí „nejaký model“:
# vrstvy koncept/konštrukcia (alternatives), kvádre, vytiahnuté profily, otočené
# diely, odčítané otvory, valce a nástroje. Veľký dom ostáva testom, ktoré merajú
# jeho veľkosť (large_house_*).

L = param("length", 6200, min = 4500, max = 10000, doc = "vonkajšia dĺžka domu (x)")
B = param("width", 4000, min = 3000, max = 6000, doc = "vonkajšia šírka domu (y)")
H = param("height", 2600, min = 2400, max = 3200, doc = "výška stien nad doskou")

WALL = 200
SLAB = 250
STUD = 60
SPACING = 1250
PITCH = 35
RIDGE = SLAB + H + B / 2.0 * math.tan(math.radians(PITCH))

alternatives(["koncept", "konštrukcia"])

box("doska", (L, B, SLAB), material = "betón", color = (150, 150, 150))
# Ako v tiny-house.star: terasa rozširuje pohľad, aby miesto vedľa domu bolo vidno.
box("terasa", (2400, 1500, SLAB - 30), at = (450, -1500, 0), material = "drevo terasa", color = (150, 100, 60))

# --- koncept: plné steny (bočné vytiahnuté a otočené), okno a strecha ---
south = box("steny/južná", (L - 2 * WALL, WALL, H), at = (WALL, 0, SLAB),
            material = "drevostavba", color = (226, 208, 172), tags = "koncept")
box("steny/severná", (L - 2 * WALL, WALL, H), at = (WALL, B - WALL, SLAB),
    material = "drevostavba", color = (226, 208, 172), tags = "koncept")

def gable(name, x):
    part = extrude(name, profile = [[0, SLAB], [B, SLAB], [B, SLAB + H], [0, SLAB + H]],
                   distance = WALL, material = "drevostavba", color = (226, 208, 172), tags = "koncept")
    return place(part, origin = (x, 0, 0), z = (1, 0, 0), x = (0, 1, 0))

gable("steny/štít západ", 0)
gable("steny/štít východ", L - WALL)

window = box("steny/južná/okno", (1000, WALL + 2, 1200), at = (2850, -1, SLAB + 900), tool = True)
subtract(south, window, name = "okno")
box("okná/južné", (1000, 24, 1200), at = (2850, (WALL - 24) / 2.0, SLAB + 900),
    material = "izolačné sklo", color = (150, 200, 230), tags = ["okno"])

roof = extrude("strecha", profile = [[-300, SLAB + H], [B + 300, SLAB + H], [B / 2.0, RIDGE + 200]],
               distance = L + 400, material = "strešný plášť", color = (72, 72, 78), tags = "koncept")
place(roof, origin = (-200, 0, 0), z = (1, 0, 0), x = (0, 1, 0))

# --- konštrukcia: stĺpiky, pásnice a OSB južnej a severnej steny ---
count = int((L - 2 * WALL) / SPACING) + 1
for side, y in (("juh", 0), ("sever", B - WALL)):
    for k in range(count):
        x = WALL + min(k * SPACING, L - 2 * WALL - STUD)
        box("konštrukcia/stĺpik %s %d" % (side, k + 1), (STUD, 140, H - 120), at = (x, y + 30, SLAB + 60),
            material = "KVH C24", color = (205, 165, 110), tags = ["konštrukcia", "stĺpiky"])
    for name, z in (("spodná", SLAB), ("horná", SLAB + H - 60)):
        box("konštrukcia/pásnica %s %s" % (side, name), (L - 2 * WALL, 140, 60), at = (WALL, y + 30, z),
            material = "KVH C24", color = (205, 165, 110), tags = ["konštrukcia", "stĺpiky"])
    box("konštrukcia/OSB %s" % side, (L - 2 * WALL, 15, H), at = (WALL, y + (170 if side == "juh" else 15), SLAB),
        material = "OSB 3", color = (196, 160, 105), tags = ["konštrukcia", "OSB"])

# --- kachle s dymovodom: valec z elipsy, končí pod strechou ---
stove = box("kachle", (450, 500, 800), at = (L * 0.6, 1000, SLAB), material = "liatina", color = (45, 45, 45))
extrude("komín", profile = ellipse(125, 125), distance = H - 900,
        at = (L * 0.6 + 225, 1250, SLAB + 800), material = "oceľ", color = (190, 192, 196))
