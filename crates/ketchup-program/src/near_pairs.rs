//! Pairs of parts whose bounds come near each other, found through a tree of
//! their bounds, so the work follows the pairs that are near rather than
//! every pair sharing a range on one axis (review 2026-11-22 R1122ac-PG-1102,
//! capability review 1026: 5 000 parts stacked 82 mm apart in z all share
//! one x range, and a sweep along x paired each with every other, 21 s).

use std::ops::ControlFlow;

type Bounds = ([f64; 3], [f64; 3]);

/// Items a leaf of the tree holds at most; a stretch of the sweep this
/// short is read off directly.
const LEAF_SIZE: usize = 8;

/// Whether `a` and `b` come within `reach` of each other on every axis; a
/// negative reach asks them to overlap by at least as much.
fn within(a: &Bounds, b: &Bounds, reach: f64) -> bool {
    (0..3).all(|axis| b.0[axis] <= a.1[axis] + reach && a.0[axis] <= b.1[axis] + reach)
}

struct Node {
    bounds: Bounds,
    /// The latest place in the sweep of the node's items.
    last: usize,
    /// A leaf holds the places `start..end`.
    start: usize,
    end: usize,
    children: Option<[usize; 2]>,
}

/// A tree of the bounds of the items at each place of the sweep along x.
struct Tree {
    bounds: Vec<Bounds>,
    places: Vec<usize>,
    nodes: Vec<Node>,
}

impl Tree {
    fn new(bounds: Vec<Bounds>) -> Self {
        let mut places: Vec<usize> = (0..bounds.len()).collect();
        let mut nodes = Vec::new();
        if !places.is_empty() {
            build(&bounds, &mut places, 0, &mut nodes);
        }
        Self {
            bounds,
            places,
            nodes,
        }
    }

    /// Pushes every place after `after` whose bounds come within `reach` of
    /// those at `after`.
    fn near(&self, after: usize, reach: f64, found: &mut Vec<usize>) {
        let query = &self.bounds[after];
        let mut pending = if self.nodes.is_empty() {
            vec![]
        } else {
            vec![0]
        };
        while let Some(node) = pending.pop() {
            let node = &self.nodes[node];
            if node.last <= after || !within(&node.bounds, query, reach) {
                continue;
            }
            match node.children {
                Some(children) => pending.extend(children),
                None => found.extend(self.places[node.start..node.end].iter().filter(|place| {
                    **place > after && within(&self.bounds[**place], query, reach)
                })),
            }
        }
    }
}

fn enclose(bounds: &[Bounds], places: &[usize]) -> Bounds {
    places.iter().fold(
        ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]),
        |(low, high), place| {
            let (place_low, place_high) = bounds[*place];
            (
                std::array::from_fn(|axis| low[axis].min(place_low[axis])),
                std::array::from_fn(|axis| high[axis].max(place_high[axis])),
            )
        },
    )
}

/// Adds the node of `places`, which start at `start` in the tree's places,
/// split at the middle of their centres along the axis they spread most.
fn build(bounds: &[Bounds], places: &mut [usize], start: usize, nodes: &mut Vec<Node>) -> usize {
    let index = nodes.len();
    nodes.push(Node {
        bounds: enclose(bounds, places),
        last: places.iter().copied().max().unwrap_or(0),
        start,
        end: start + places.len(),
        children: None,
    });
    if places.len() > LEAF_SIZE {
        let centre = |place: usize, axis: usize| bounds[place].0[axis] + bounds[place].1[axis];
        let spread = |axis: usize| {
            let (low, high) =
                places
                    .iter()
                    .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), place| {
                        let centre = centre(*place, axis);
                        (low.min(centre), high.max(centre))
                    });
            high - low
        };
        let axis = (0..3)
            .max_by(|a, b| spread(*a).total_cmp(&spread(*b)))
            .unwrap_or(0);
        let middle = places.len() / 2;
        places.select_nth_unstable_by(middle, |a, b| centre(*a, axis).total_cmp(&centre(*b, axis)));
        let (low, high) = places.split_at_mut(middle);
        let first = build(bounds, low, start, nodes);
        let second = build(bounds, high, start + middle, nodes);
        nodes[index].children = Some([first, second]);
    }
    index
}

/// Visits each pair of `items` whose `bounds` come [`within`] `reach` of each
/// other, the one with the lower x bound first, in the order a sweep along x
/// meets them, until `visit` breaks.
pub(crate) fn near_pairs(
    bounds: &[Bounds],
    mut items: Vec<usize>,
    reach: f64,
    mut visit: impl FnMut(usize, usize) -> ControlFlow<()>,
) {
    items.sort_by(|left, right| bounds[*left].0[0].total_cmp(&bounds[*right].0[0]));
    let tree = Tree::new(items.iter().map(|item| bounds[*item]).collect());
    let starts: Vec<f64> = tree.bounds.iter().map(|(low, _)| low[0]).collect();
    let mut later = Vec::new();
    let mut marked = vec![false; items.len()];
    // Whether most of the last stretch a sweep along x passed was near: a
    // crowd is read off its stretch, cheaper than the tree.
    let mut crowded = false;
    for (place, &left) in items.iter().enumerate() {
        let query = &tree.bounds[place];
        let end = starts.partition_point(|start| *start <= query.1[0] + reach);
        let stretch = place + 1..end.max(place + 1);
        later.clear();
        if crowded || stretch.len() <= LEAF_SIZE {
            later.extend(
                stretch
                    .clone()
                    .filter(|at| within(query, &tree.bounds[*at], reach)),
            );
        } else {
            tree.near(place, reach, &mut later);
            in_sweep_order(&mut later, &mut marked, &stretch);
        }
        crowded = !later.is_empty() && later.len() * 4 >= stretch.len();
        for at in &later {
            if visit(left, items[*at]).is_break() {
                return;
            }
        }
    }
}

/// Sorts `places`, all within `stretch`: when few, by sorting, else by
/// marking them and reading them off the stretch.
fn in_sweep_order(places: &mut Vec<usize>, marked: &mut [bool], stretch: &std::ops::Range<usize>) {
    if places.is_empty() {
        return;
    }
    if places.len() * (usize::try_from(places.len().ilog2()).unwrap_or(64) + 1) < stretch.len() {
        places.sort_unstable();
        return;
    }
    for at in places.iter() {
        marked[*at] = true;
    }
    places.clear();
    for at in stretch.clone() {
        if std::mem::take(&mut marked[at]) {
            places.push(at);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pairs a sweep along x checking every axis visits, in its order.
    fn swept(bounds: &[Bounds], reach: f64) -> Vec<(usize, usize)> {
        let mut order: Vec<usize> = (0..bounds.len()).collect();
        order.sort_by(|left, right| bounds[*left].0[0].total_cmp(&bounds[*right].0[0]));
        let mut pairs = Vec::new();
        for (position, &left) in order.iter().enumerate() {
            for &right in &order[position + 1..] {
                if within(&bounds[left], &bounds[right], reach) {
                    pairs.push((left, right));
                }
            }
        }
        pairs
    }

    fn visited(bounds: &[Bounds], reach: f64) -> Vec<(usize, usize)> {
        let mut pairs = Vec::new();
        near_pairs(bounds, (0..bounds.len()).collect(), reach, |a, b| {
            pairs.push((a, b));
            ControlFlow::Continue(())
        });
        pairs
    }

    #[test]
    fn the_tree_visits_what_a_full_sweep_does_in_its_order() {
        // Boxes of mixed sizes, some at one spot, some touching, some apart.
        let mut seed = 7_u64;
        let mut next = move || {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            f64::from(u32::try_from((seed >> 33) % 1000).unwrap_or(0))
        };
        let bounds: Vec<Bounds> = (0..400)
            .map(|item| {
                let low: [f64; 3] = if item % 10 == 0 {
                    [0.0; 3]
                } else {
                    std::array::from_fn(|_| next())
                };
                let size: [f64; 3] = std::array::from_fn(|_| 1.0 + next() / 20.0);
                (low, std::array::from_fn(|axis| low[axis] + size[axis]))
            })
            .collect();
        for reach in [-0.1, 0.0, 0.1, 20.0] {
            assert_eq!(
                visited(&bounds, reach),
                swept(&bounds, reach),
                "reach {reach}"
            );
        }
    }

    #[test]
    fn parts_stacked_along_z_pair_only_with_their_neighbours() {
        let bounds: Vec<Bounds> = (0..5_000)
            .map(|item| {
                let z = f64::from(item) * 100.0;
                ([0.0, 0.0, z], [100.0, 100.0, z + 18.0])
            })
            .collect();
        assert!(visited(&bounds, 20.0).is_empty());
        let mut checked = 0_usize;
        near_pairs(&bounds, (0..bounds.len()).collect(), 82.0, |a, b| {
            checked += 1;
            assert_eq!(a.abs_diff(b), 1);
            ControlFlow::Continue(())
        });
        assert_eq!(checked, 4_999);
    }

    #[test]
    fn a_break_stops_the_visits() {
        let bounds: Vec<Bounds> = vec![([0.0; 3], [1.0; 3]); 50];
        let mut seen = 0;
        near_pairs(&bounds, (0..50).collect(), 0.0, |_, _| {
            seen += 1;
            if seen == 3 {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        });
        assert_eq!(seen, 3);
    }
}
