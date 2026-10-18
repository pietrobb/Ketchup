//! Terminal outputs of validated graphs, kept for read-only consumers (topology
//! evidence and non-destructive pair queries) under a count and a byte budget:
//! the oldest leave first, and an output larger than the whole budget is
//! returned without being kept.

use std::collections::{BTreeMap, VecDeque};
use std::rc::Rc;

pub(super) const MAX_CACHED_GRAPH_OUTPUTS: usize = 512;
/// The worker has no memory limit of its own, so the kept outputs stop here.
pub(super) const MAX_CACHED_GRAPH_OUTPUT_BYTES: usize = 256 * 1024 * 1024;

pub(super) struct OutputCache<T> {
    max_entries: usize,
    max_bytes: usize,
    used_bytes: usize,
    order: VecDeque<String>,
    outputs: BTreeMap<String, (Rc<T>, usize)>,
}

impl<T> Default for OutputCache<T> {
    fn default() -> Self {
        Self::new(MAX_CACHED_GRAPH_OUTPUTS, MAX_CACHED_GRAPH_OUTPUT_BYTES)
    }
}

impl<T> OutputCache<T> {
    pub(super) fn new(max_entries: usize, max_bytes: usize) -> Self {
        Self {
            max_entries,
            max_bytes,
            used_bytes: 0,
            order: VecDeque::new(),
            outputs: BTreeMap::new(),
        }
    }

    pub(super) fn get(&self, key: &str) -> Option<Rc<T>> {
        self.outputs.get(key).map(|(output, _)| Rc::clone(output))
    }

    /// Keeps `output` charged `bytes`, first letting the oldest outputs go
    /// until it fits both budgets.
    pub(super) fn insert(&mut self, key: String, output: Rc<T>, bytes: usize) {
        if self.outputs.contains_key(&key) || bytes > self.max_bytes || self.max_entries == 0 {
            return;
        }
        while self.order.len() >= self.max_entries || self.used_bytes + bytes > self.max_bytes {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            if let Some((_, charged)) = self.outputs.remove(&oldest) {
                self.used_bytes -= charged;
            }
        }
        self.used_bytes += bytes;
        self.order.push_back(key.clone());
        self.outputs.insert(key, (output, bytes));
    }

    #[cfg(test)]
    pub(super) fn keys(&self) -> Vec<&str> {
        self.order.iter().map(String::as_str).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_outputs_push_the_oldest_out_before_the_count_is_reached() {
        let mut cache = OutputCache::new(512, 100);
        for key in ["a", "b", "c"] {
            cache.insert(key.to_owned(), Rc::new(()), 40);
        }
        // 120 bytes would not fit 100: "a" left, though 3 of 512 places are used.
        assert_eq!(cache.keys(), ["b", "c"]);
        assert!(cache.get("a").is_none());
        assert_eq!(cache.used_bytes, 80);
        cache.insert("d".to_owned(), Rc::new(()), 90);
        assert_eq!(cache.keys(), ["d"]);
        assert_eq!(cache.used_bytes, 90);
    }

    #[test]
    fn an_output_larger_than_the_budget_is_not_kept_and_keeps_the_others() {
        let mut cache = OutputCache::new(512, 100);
        cache.insert("a".to_owned(), Rc::new(()), 30);
        cache.insert("huge".to_owned(), Rc::new(()), 101);
        assert_eq!(cache.keys(), ["a"]);
        assert!(cache.get("huge").is_none());
    }

    #[test]
    fn the_count_still_bounds_small_outputs() {
        let mut cache = OutputCache::new(2, 1_000);
        for key in ["a", "b", "c"] {
            cache.insert(key.to_owned(), Rc::new(()), 1);
        }
        assert_eq!(cache.keys(), ["b", "c"]);
        assert_eq!(cache.used_bytes, 2);
    }
}
