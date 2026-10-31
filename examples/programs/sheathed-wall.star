# A sheathed timber-frame wall with a ledger on its inside face carrying a joist, cut
# from declared stock and priced from the program's own price list.
# The wall is a buildup: KVH C24 studs at 625 behind OSB laid from upright 1250 x 2500
# boards ("sheet"), their joints on stud centres, the grain up. The joist hangs on the
# ledger on an SAE380 joist hanger from the connector catalogue (connector(); the joint
# check lists its ETA rating, the hardware the hanger and the nails it is rated with). bar_stock and sheet_stock declare what the timber and the
# boards are cut from (report section cutting); price_list the unit prices the quote
# sums (report section quote: Kečup has no prices of its own, and adds no VAT).
LENGTH = param("wall_length", 3000, min = 1250, max = 5000, doc = "length of the wall along x")
HEIGHT = 2500
LEDGER = (120, 240)
JOIST = (60, 200)
SPAN = 3000

buildup("wall", (0, 0, 0), (1, 0, 0), (0, 0, 1), LENGTH, height = HEIGHT,
    layers = [
        {"name": "frame", "thickness": 140, "material": "KVH C24", "spacing": 625},
        {"name": "osb", "thickness": 15, "material": "OSB", "sheet": (1250, 2500), "grain": "v"},
    ])

# The layers stack from the inside face (y = 0) outward, towards -y.
ledger = box("ledger", (LENGTH, LEDGER[0], LEDGER[1]), at = (0, 0, HEIGHT - 500),
             material = "KVH C24", grounded = True)
joist = box("joist", (JOIST[0], SPAN, JOIST[1]),
            at = (LENGTH / 2 - JOIST[0] / 2, LEDGER[0], HEIGHT - 500 + LEDGER[1] - JOIST[1]),
            material = "KVH C24")
connector(joist, ledger, "SAE380")

bar_stock("KVH C24", [3000, 4000, 5000], kerf = 4)
sheet_stock("OSB", [(2500, 1250)], grain = True)
price_list({
    "KVH C24": (480, "m3"),
    "OSB": (14.9, "piece"),
    "SAE380": (4.2, "piece"),
    "square twist nail 3.75x30": (0.04, "piece"),
}, currency = "EUR", source = "example prices, not an offer")
