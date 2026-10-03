# Fixed shelves on either side of a shared partition. The second row is
# explicitly staggered; validation still determines machining clearance.
partition = board("partition", (18, 400, 600))
left = board("left shelf", (300, 400, 18), at = (-300, 0, 240))
right = board("right shelf", (300, 400, 18), at = (18, 0, 240))
dowels(partition, left, count = 2, margin = 50)
dowels(partition, right, count = 2, margin = 50, offset = 11)
group("fixed shelves", [partition, left, right], grounded = True)
