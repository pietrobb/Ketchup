//! The draw order of the fills drawn into a drawing (review 2026-11-24 og DXF-900). AutoCAD
//! and ezdxf draw a space by the sort handles of its `ACAD_SORTENTS` table (a SORTENTSTABLE
//! in the extension dictionary of the space's block record), and an entity not in it by its
//! own handle. A fill written first in ENTITIES has the highest handle, so without an entry
//! it is drawn over every line of the drawing.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use super::{Drawing, Entity, ObjectEdits, hex, take_handle};

/// Model space's SORTENTSTABLE: its bytes, where its entries start, and the sort handle of
/// each entity it names.
struct SortTable {
    bytes: Range<usize>,
    entries_at: usize,
    keys: BTreeMap<u64, u64>,
}

impl Drawing<'_> {
    /// Puts `fills`, the handles of the fills drawn (lower than those of the entities added
    /// over them), under every entity of model space in its draw order, keeping the order of
    /// the drawing's own: the sort handles of the fills and of those entities, sorted, are
    /// given out again in that order, and each entity whose sort handle is not its own is an
    /// entry. Model space's sort table is rewritten, or one written with its extension
    /// dictionary. `removed` are the handles taken out. A drawing before DXF 14, or without
    /// the OBJECTS section or a model space block record to hold a table, keeps its fills
    /// first in the file only.
    pub(super) fn fills_under(
        &self,
        fills: &[u64],
        removed: &BTreeSet<u64>,
        handles: &mut Option<u64>,
        edits: &mut ObjectEdits,
    ) {
        let (Some(objects_end), Some(space)) = (self.objects_end, self.model_space) else {
            return;
        };
        if fills.is_empty() || !self.lightweight {
            return;
        }
        let space = self.record_at(space);
        let Some(space_handle) = self.own_handle(&space) else {
            return;
        };
        let table = self.sort_table(space_handle, objects_end);
        let records = self.block_record_handles();
        let mut own: Vec<(u64, usize, u64)> = self
            .entities
            .iter()
            .enumerate()
            .filter_map(|(at, entity)| {
                let handle = entity.handle.filter(|handle| !removed.contains(handle))?;
                let key = table.as_ref().and_then(|table| table.keys.get(&handle));
                self.in_space(entity, space_handle, &records)
                    .then(|| (key.copied().unwrap_or(handle), at, handle))
            })
            .collect();
        own.sort_unstable();
        let mut keys: Vec<u64> = own
            .iter()
            .map(|(key, ..)| *key)
            .chain(fills.iter().copied())
            .collect();
        keys.sort_unstable();
        let order = fills
            .iter()
            .copied()
            .chain(own.iter().map(|(.., handle)| *handle));
        let mut entries = Vec::new();
        for (handle, key) in order.zip(keys) {
            if handle != key {
                self.push_pair(&mut entries, 331, format!("{handle:X}").as_bytes());
                self.push_pair(&mut entries, 5, format!("{key:X}").as_bytes());
            }
        }
        if let Some(table) = table {
            // Rewritten whole: what the removal took out of it is out of it here too.
            edits.retain(|(range, _)| range.start != table.bytes.start);
            let mut with = self.source[table.bytes.start..table.entries_at].to_vec();
            with.extend(entries);
            edits.push((table.bytes, with));
            return;
        }
        if entries.is_empty() {
            return;
        }
        let space_hex = format!("{space_handle:X}").into_bytes();
        let mut added = Vec::new();
        let (dictionary, table) = match self.extension_dictionary(&space) {
            Some(dictionary) => {
                let Some(record) = self.object_of(dictionary, objects_end) else {
                    return;
                };
                if self.entry_of(&record, b"ACAD_SORTENTS").is_some() {
                    // Names a table not of model space or not there: left as it is.
                    return;
                }
                let Some(table) = take_handle(handles) else {
                    return;
                };
                let mut entry = Vec::new();
                self.push_pair(&mut entry, 3, b"ACAD_SORTENTS");
                self.push_pair(&mut entry, 360, format!("{table:X}").as_bytes());
                let end = self.pairs[record.end - 1].end();
                edits.push((end..end, entry));
                (dictionary, table)
            }
            None => {
                let (Some(dictionary), Some(table)) = (take_handle(handles), take_handle(handles))
                else {
                    return;
                };
                let dictionary_hex = format!("{dictionary:X}").into_bytes();
                // In the block record before its owner, as the extension dictionary is.
                let mut group = Vec::new();
                self.push_pair(&mut group, 102, b"{ACAD_XDICTIONARY");
                self.push_pair(&mut group, 360, &dictionary_hex);
                self.push_pair(&mut group, 102, b"}");
                let at = self.owner_pair(&space).map_or_else(
                    || self.pairs[space.start + 1].end(),
                    |owner| self.pairs[owner].start(),
                );
                edits.push((at..at, group));
                self.push_pair(&mut added, 0, b"DICTIONARY");
                self.push_pair(&mut added, 5, &dictionary_hex);
                self.reactor(&mut added, &space_hex);
                self.push_pair(&mut added, 100, b"AcDbDictionary");
                self.push_pair(&mut added, 280, b"1");
                self.push_pair(&mut added, 281, b"1");
                self.push_pair(&mut added, 3, b"ACAD_SORTENTS");
                self.push_pair(&mut added, 360, format!("{table:X}").as_bytes());
                (dictionary, table)
            }
        };
        self.push_pair(&mut added, 0, b"SORTENTSTABLE");
        self.push_pair(&mut added, 5, format!("{table:X}").as_bytes());
        self.reactor(&mut added, format!("{dictionary:X}").as_bytes());
        self.push_pair(&mut added, 100, b"AcDbSortentsTable");
        self.push_pair(&mut added, 330, &space_hex);
        added.extend(entries);
        let at = self.pairs[objects_end].start();
        edits.push((at..at, added));
        self.class_of_sort_tables(edits);
    }

    /// The CLASS record of SORTENTSTABLE, when the drawing has none (review 2026-11-25 og
    /// DXF-1004): AutoCAD and ezdxf list each class of object a drawing holds. Put in its
    /// CLASSES section, or in one written before TABLES; with an instance count when the
    /// drawing's classes have one.
    fn class_of_sort_tables(&self, edits: &mut ObjectEdits) {
        let named = |index: usize, code: i32, name: &[u8]| {
            self.pairs[index].code == code && self.value(index).eq_ignore_ascii_case(name)
        };
        let section = |name: &[u8]| {
            (0..self.pairs.len().saturating_sub(1))
                .find(|index| named(*index, 0, b"SECTION") && named(index + 1, 2, name))
        };
        let mut class = Vec::new();
        let (at, counted, new_section) = if let Some(start) = section(b"CLASSES") {
            let Some(end) = (start + 2..self.pairs.len()).find(|index| named(*index, 0, b"ENDSEC"))
            else {
                return;
            };
            if (start + 2..end).any(|index| named(index, 1, b"SORTENTSTABLE")) {
                return;
            }
            let counted = (start + 2..end).any(|index| self.pairs[index].code == 91);
            (self.pairs[end].start(), counted, false)
        } else {
            let Some(tables) = section(b"TABLES") else {
                return;
            };
            self.push_pair(&mut class, 0, b"SECTION");
            self.push_pair(&mut class, 2, b"CLASSES");
            (self.pairs[tables].start(), false, true)
        };
        self.push_pair(&mut class, 0, b"CLASS");
        self.push_pair(&mut class, 1, b"SORTENTSTABLE");
        self.push_pair(&mut class, 2, b"AcDbSortentsTable");
        self.push_pair(&mut class, 3, b"ObjectDBX Classes");
        self.push_pair(&mut class, 90, b"0");
        if counted {
            self.push_pair(&mut class, 91, b"1");
        }
        self.push_pair(&mut class, 280, b"0");
        self.push_pair(&mut class, 281, b"0");
        if new_section {
            self.push_pair(&mut class, 0, b"ENDSEC");
        }
        edits.push((at..at, class));
    }

    /// The reactor group naming `owner`, and the owner.
    fn reactor(&self, out: &mut Vec<u8>, owner: &[u8]) {
        self.push_pair(out, 102, b"{ACAD_REACTORS");
        self.push_pair(out, 330, owner);
        self.push_pair(out, 102, b"}");
        self.push_pair(out, 330, owner);
    }

    /// The pairs of the record whose pair of code 0 is `start`.
    fn record_at(&self, start: usize) -> Range<usize> {
        let end = (start + 1..self.pairs.len())
            .find(|index| self.pairs[*index].code == 0)
            .unwrap_or(self.pairs.len());
        start..end
    }

    /// The handle of the record of pairs `record`.
    fn own_handle(&self, record: &Range<usize>) -> Option<u64> {
        record
            .clone()
            .find(|index| matches!(self.pairs[*index].code, 5 | 105))
            .and_then(|index| hex(self.value(index)))
    }

    /// The pair of the owner (330) of `record`, outside its 102 groups.
    fn owner_pair(&self, record: &Range<usize>) -> Option<usize> {
        let mut in_group = false;
        record.clone().find(|index| {
            match self.pairs[*index].code {
                102 => in_group = self.value(*index).starts_with(b"{"),
                330 => return !in_group,
                _ => {}
            }
            false
        })
    }

    /// The extension dictionary of `record`.
    fn extension_dictionary(&self, record: &Range<usize>) -> Option<u64> {
        let mut in_dictionary = false;
        record.clone().find_map(|index| {
            match self.pairs[index].code {
                102 => in_dictionary = self.value(index).eq_ignore_ascii_case(b"{ACAD_XDICTIONARY"),
                360 if in_dictionary => return hex(self.value(index)),
                _ => {}
            }
            None
        })
    }

    /// The pairs of the record of the OBJECTS section of handle `handle`.
    fn object_of(&self, handle: u64, objects_end: usize) -> Option<Range<usize>> {
        self.objects.iter().enumerate().find_map(|(at, &start)| {
            let record = start..self.objects.get(at + 1).copied().unwrap_or(objects_end);
            (self.own_handle(&record) == Some(handle)).then_some(record)
        })
    }

    /// The pair naming the entry `key` of the dictionary `record`.
    fn entry_of(&self, record: &Range<usize>, key: &[u8]) -> Option<usize> {
        record.clone().find_map(|index| {
            (self.pairs[index].code == 3 && self.value(index).eq_ignore_ascii_case(key))
                .then_some(index + 1)
                .filter(|next| *next < record.end && matches!(self.pairs[*next].code, 350 | 360))
        })
    }

    /// The SORTENTSTABLE of the space of block record `space`.
    fn sort_table(&self, space: u64, objects_end: usize) -> Option<SortTable> {
        self.objects.iter().enumerate().find_map(|(at, &start)| {
            if !self.value(start).eq_ignore_ascii_case(b"SORTENTSTABLE") {
                return None;
            }
            let record = start..self.objects.get(at + 1).copied().unwrap_or(objects_end);
            let marker = record.clone().find(|index| {
                self.pairs[*index].code == 100
                    && self
                        .value(*index)
                        .eq_ignore_ascii_case(b"AcDbSortentsTable")
            })?;
            let block = (marker + 1..record.end).find(|index| self.pairs[*index].code == 330)?;
            if hex(self.value(block)) != Some(space) {
                return None;
            }
            let mut keys = BTreeMap::new();
            for index in block + 1..record.end - 1 {
                if self.pairs[index].code == 331 && self.pairs[index + 1].code == 5 {
                    let entity = hex(self.value(index));
                    keys.extend(entity.zip(hex(self.value(index + 1))));
                }
            }
            Some(SortTable {
                bytes: self.pairs[record.start].start()..self.pairs[record.end - 1].end(),
                entries_at: self.pairs[block].end(),
                keys,
            })
        })
    }

    /// Whether `entity` is of the space of block record `space`: owned by it, or not owned
    /// by another of the block records `records` and not in paper space. An owner that is no
    /// block record is the group or hatch an export before R1125og-DXF-1000 gave its entities.
    fn in_space(&self, entity: &Entity, space: u64, records: &BTreeSet<u64>) -> bool {
        let head = Self::head(entity);
        match self
            .owner_pair(&head)
            .and_then(|owner| hex(self.value(owner)))
        {
            Some(owner) if owner == space => true,
            Some(owner) if records.contains(&owner) => false,
            _ => !self.in_paper_space(&head),
        }
    }

    /// The owner (330) of the entities added to model space (R1125og-DXF-1000): the handle
    /// of model space's block record, otherwise the owner of its first entity outside the
    /// 102 groups, where a reactor names a group, an associative hatch or a dimension.
    pub(super) fn space_owner(&self) -> Option<Vec<u8>> {
        let record = self.model_space.map(|space| self.record_at(space));
        if let Some(at) = record.and_then(|record| {
            record
                .clone()
                .find(|index| matches!(self.pairs[*index].code, 5 | 105))
        }) {
            return Some(self.value(at).to_vec());
        }
        self.entities.iter().find_map(|entity| {
            let head = Self::head(entity);
            if self.in_paper_space(&head) {
                return None;
            }
            self.owner_pair(&head).map(|at| self.value(at).to_vec())
        })
    }

    /// The handles of the drawing's block records.
    fn block_record_handles(&self) -> BTreeSet<u64> {
        let Some((end, _)) = self.block_records else {
            return BTreeSet::new();
        };
        let is = |index: usize, name: &[u8]| {
            self.pairs[index].code == 0 && self.value(index).eq_ignore_ascii_case(name)
        };
        let start = (0..end)
            .rev()
            .find(|index| is(*index, b"TABLE"))
            .unwrap_or(0);
        (start..end)
            .filter(|index| is(*index, b"BLOCK_RECORD"))
            .filter_map(|index| self.own_handle(&self.record_at(index)))
            .collect()
    }

    /// The pairs of `entity`'s own record, before those of its vertices or attributes.
    fn head(entity: &Entity) -> Range<usize> {
        entity.pairs.start..entity.records.get(1).map_or(entity.pairs.end, |r| r.0)
    }

    /// Whether the record `head` is marked as in paper space (67 is 1).
    fn in_paper_space(&self, head: &Range<usize>) -> bool {
        head.clone()
            .any(|index| self.pairs[index].code == 67 && self.value(index) == b"1")
    }
}
