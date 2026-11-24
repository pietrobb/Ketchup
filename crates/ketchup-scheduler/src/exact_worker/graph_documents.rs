//! The imported STEP or IGES documents a worker builds graph parts from.
//!
//! A graph naming one part of a multi-part source opened the source for
//! itself alone, so every part a worker built parsed the whole file again: on
//! a model's first check, 22 parts of one purchased STEP parsed it 214 times
//! across the workers, about 4 s each. The worker now keeps the documents of
//! the last two sources it opened, by format and the SHA-256 of their verified
//! bytes; a third source lets the one used longest ago go before it is read.

use std::rc::Rc;

pub(super) const MAX_KEPT_GRAPH_DOCUMENTS: usize = 2;

pub(super) struct KeptDocuments<T> {
    max_documents: usize,
    /// Least recently used first.
    kept: Vec<(String, Rc<T>)>,
}

impl<T> Default for KeptDocuments<T> {
    fn default() -> Self {
        Self::new(MAX_KEPT_GRAPH_DOCUMENTS)
    }
}

impl<T> KeptDocuments<T> {
    pub(super) const fn new(max_documents: usize) -> Self {
        Self {
            max_documents,
            kept: Vec::new(),
        }
    }

    /// The document kept under `key`, or the one `open` reads, kept in place
    /// of the one used longest ago when the budget is full.
    pub(super) fn get_or_open<E>(
        &mut self,
        key: &str,
        open: impl FnOnce() -> Result<T, E>,
    ) -> Result<Rc<T>, E> {
        if let Some(position) = self.kept.iter().position(|(kept, _)| kept == key) {
            let entry = self.kept.remove(position);
            let document = Rc::clone(&entry.1);
            self.kept.push(entry);
            return Ok(document);
        }
        // Released before the read, so no more than the budget is ever held.
        while !self.kept.is_empty() && self.kept.len() >= self.max_documents {
            self.kept.remove(0);
        }
        let document = Rc::new(open()?);
        if self.max_documents > 0 {
            self.kept.push((key.to_owned(), Rc::clone(&document)));
        }
        Ok(document)
    }

    #[cfg(test)]
    fn keys(&self) -> Vec<&str> {
        self.kept.iter().map(|(key, _)| key.as_str()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_third_source_lets_the_one_used_longest_ago_go() {
        let mut documents = KeptDocuments::new(2);
        let mut opens = 0;
        for key in ["a", "b", "a", "c", "a", "b"] {
            documents
                .get_or_open(key, || {
                    opens += 1;
                    Ok::<_, ()>(())
                })
                .unwrap();
        }
        // "a" was used again before "c" came, so "b" left; "b" read again pushed out "c".
        assert_eq!(documents.keys(), ["a", "b"]);
        assert_eq!(opens, 4);
    }

    #[test]
    fn a_failed_read_keeps_nothing_and_is_tried_again() {
        let mut documents = KeptDocuments::<()>::new(2);
        assert!(documents.get_or_open("a", || Err("unreadable")).is_err());
        assert!(documents.keys().is_empty());
        assert!(documents.get_or_open("a", || Ok::<_, ()>(())).is_ok());
        assert_eq!(documents.keys(), ["a"]);
    }
}
