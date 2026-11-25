//! The draw order of a DWG drawing kept in its DXF (review 2026-11-25 og DXF-1002).
//!
//! AutoCAD draws a space by the sort handles of its sort table (`ACAD_SORTENTS`), an entity
//! not in the table by its own handle. acadrust's DXF writer gives the table's entries new
//! sort handles, in their order but above every handle of the drawing, so the entities in
//! the table were drawn over all the others: a hatch the architect sent to the back came out
//! on top. With every entity of the space in its table, keyed by what it was drawn by, the
//! new handles keep the drawing's whole order.

use std::collections::HashSet;

use acadrust::CadDocument;
use acadrust::objects::ObjectType;
use acadrust::types::Handle;

#[cfg(test)]
mod tests;

/// Puts every entity of a space that has a sort table into that table, with its own handle
/// as sort handle when it had no entry, in the order the space is drawn.
pub(crate) fn complete_sort_tables(document: &mut CadDocument) {
    let spaces: Vec<(Handle, Vec<Handle>)> = document
        .block_records
        .iter()
        .map(|record| (record.handle, record.entity_handles.clone()))
        .collect();
    for object in document.objects.values_mut() {
        let ObjectType::SortEntitiesTable(table) = object else {
            continue;
        };
        let Some((_, space)) = spaces
            .iter()
            .find(|(handle, _)| *handle == table.block_owner_handle)
        else {
            continue;
        };
        let in_space: HashSet<Handle> = space.iter().copied().collect();
        // The table's entries of entities the space no longer holds stay where they were.
        let mut order: Vec<(Handle, Handle)> = table
            .entries()
            .filter(|entry| !in_space.contains(&entry.entity_handle))
            .map(|entry| (entry.entity_handle, entry.sort_handle))
            .collect();
        order.extend(
            space
                .iter()
                .map(|entity| (*entity, table.effective_sort_handle(*entity))),
        );
        // Stable: of equal sort handles, the one first in the space is drawn first.
        order.sort_by_key(|(_, sort)| sort.value());
        table.clear();
        for (entity, sort) in order {
            table.add_entry(entity, sort);
        }
        table.raw_dxf_codes = None;
    }
}
