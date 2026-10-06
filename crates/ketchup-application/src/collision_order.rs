//! Orders exact pair candidates so that pairs sharing solids sit next to each other.
//!
//! Every worker evaluates each solid it is asked about once and keeps it, so a
//! solid split over many workers is rebuilt in each of them. Numbering the
//! solids breadth first through the contact graph (Cuthill-McKee) keeps the
//! neighbours of a solid within a narrow band, and ranges cut from pairs sorted
//! by that numbering then share most of their solids.
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// A rank for every graph named by `pairs`; neighbours get nearby ranks.
pub(super) fn locality_ranks(
    pairs: impl Iterator<Item = (usize, usize)>,
) -> BTreeMap<usize, usize> {
    let mut neighbours = BTreeMap::<usize, BTreeSet<usize>>::new();
    for (left, right) in pairs {
        neighbours.entry(left).or_default();
        neighbours.entry(right).or_default();
        if left != right {
            neighbours.get_mut(&left).expect("inserted").insert(right);
            neighbours.get_mut(&right).expect("inserted").insert(left);
        }
    }
    let degree = |graph: &usize| neighbours[graph].len();
    let mut starts = neighbours.keys().copied().collect::<Vec<_>>();
    starts.sort_by_key(|graph| (degree(graph), *graph));
    let mut ranks = BTreeMap::new();
    let mut queue = VecDeque::new();
    for start in starts {
        if ranks.contains_key(&start) {
            continue;
        }
        ranks.insert(start, ranks.len());
        queue.push_back(start);
        while let Some(graph) = queue.pop_front() {
            let mut next = neighbours[&graph]
                .iter()
                .copied()
                .filter(|other| !ranks.contains_key(other))
                .collect::<Vec<_>>();
            next.sort_by_key(|other| (degree(other), *other));
            for other in next {
                ranks.insert(other, ranks.len());
                queue.push_back(other);
            }
        }
    }
    ranks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chain_numbered_out_of_order_gets_consecutive_ranks() {
        // 0 - 7 - 3 - 9 - 1 as pairs given in scrambled order.
        let ranks = locality_ranks([(3, 9), (0, 7), (9, 1), (7, 3), (3, 3)].into_iter());
        let order = [0, 7, 3, 9, 1].map(|graph| ranks[&graph]);
        assert_eq!(order, [0, 1, 2, 3, 4]);
    }

    #[test]
    fn separate_clusters_stay_contiguous() {
        let ranks = locality_ranks([(0, 10), (1, 11), (10, 20), (11, 21)].into_iter());
        let first = [0, 10, 20].map(|graph| ranks[&graph]);
        let second = [1, 11, 21].map(|graph| ranks[&graph]);
        assert!(
            first.iter().max() < second.iter().min() || second.iter().max() < first.iter().min()
        );
    }
}
