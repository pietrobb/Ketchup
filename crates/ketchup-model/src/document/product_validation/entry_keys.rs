//! Every entry of the product's maps is filed under its own id. Edits file
//! them so; a file changed outside Kečup and sealed again need not (review
//! 2026-11-22 R1122ac-PS-1100: a group filed under another id made loading
//! panic, an occurrence opened editable though nothing could find it, and the
//! window fell over at its first frame). A product read whole is checked
//! before anything looks an entry up by the id it names.

use super::*;

/// Whether each entry of the groups, occurrences, definitions, features,
/// tags and classification dimensions (with their categories) of `product`
/// names the id it is filed under.
pub(super) fn validate_entry_keys(product: &ProductModel) -> Result<(), CanonicalError> {
    filed("groups", &product.groups, |group| group.id.0, |key| key.0)?;
    filed(
        "occurrences",
        &product.occurrences,
        |occurrence| occurrence.id.0,
        |key| key.0,
    )?;
    filed(
        "definitions",
        &product.definitions,
        |definition| definition.id.0,
        |key| key.0,
    )?;
    filed(
        "features",
        &product.features,
        |feature| feature.id.0,
        |key| key.0,
    )?;
    filed("tags", &product.tags, |tag| tag.id.0, |key| key.0)?;
    filed(
        "classification dimensions",
        &product.classification_dimensions,
        |dimension| dimension.id.0,
        |key| key.0,
    )?;
    for dimension in product.classification_dimensions.values() {
        filed(
            "classification categories",
            &dimension.categories,
            |category| category.id.0,
            |key| key.0,
        )?;
    }
    Ok(())
}

fn filed<K, V>(
    collection: &'static str,
    entries: &BTreeMap<K, V>,
    id: impl Fn(&V) -> u64,
    key: impl Fn(&K) -> u64,
) -> Result<(), CanonicalError> {
    match entries
        .iter()
        .map(|(filed, entry)| (key(filed), id(entry)))
        .find(|(filed, named)| filed != named)
    {
        Some((key, id)) => Err(CanonicalError::EntryUnderOtherId {
            collection,
            key,
            id,
        }),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::{load, save};

    const RENAMED: u64 = 9_999_999;

    /// The first entry of `entries` renamed by `rename`, if there is one.
    fn rename_first<K: Ord, V: Clone>(
        entries: &mut BTreeMap<K, Arc<V>>,
        rename: impl FnOnce(&mut V),
    ) -> bool {
        entries
            .values_mut()
            .next()
            .map(|entry| rename(Arc::make_mut(entry)))
            .is_some()
    }

    /// The product of `fixture` with the first entry of each collection
    /// naming another id than it is filed under, each alone, by collection.
    fn misfiled(fixture: &[u8]) -> Vec<(&'static str, ProductModel)> {
        let product = (*load(fixture).expect("the fixture opens").snapshot().product).clone();
        let mut found = Vec::new();
        let mut each = |collection, rename: &dyn Fn(&mut ProductModel) -> bool| {
            let mut changed = product.clone();
            if rename(&mut changed) {
                found.push((collection, changed));
            }
        };
        each("groups", &|p| {
            rename_first(&mut p.groups, |group| group.id = GroupId(RENAMED))
        });
        each("occurrences", &|p| {
            rename_first(&mut p.occurrences, |occurrence| {
                occurrence.id = OccurrenceId(RENAMED);
            })
        });
        each("definitions", &|p| {
            rename_first(&mut p.definitions, |definition| {
                definition.id = DefinitionId(RENAMED);
            })
        });
        each("features", &|p| {
            rename_first(&mut p.features, |feature| feature.id = FeatureId(RENAMED))
        });
        each("tags", &|p| {
            rename_first(&mut p.tags, |tag| tag.id = TagId(RENAMED))
        });
        each("classification dimensions", &|p| {
            rename_first(&mut p.classification_dimensions, |dimension| {
                dimension.id = ClassificationDimensionId(RENAMED);
            })
        });
        each("classification categories", &|p| {
            rename_first(&mut p.classification_dimensions, |dimension| {
                if let Some(category) = dimension.categories.values_mut().next() {
                    category.id = ClassificationCategoryId(RENAMED);
                }
            }) && p
                .classification_dimensions
                .values()
                .any(|dimension| !dimension.categories.is_empty())
        });
        found
    }

    /// R1122ac-PS-1100: a saved document with an entry filed under another id
    /// than it names, sealed again, is refused on opening with the collection
    /// named, never opened (editable, to fail at the first look-up) nor a
    /// panic. The fixtures, opened and saved again in the current format, hold
    /// groups, occurrences and every other collection checked between them;
    /// the formats they were written in are tested below.
    #[test]
    fn an_entry_filed_under_another_id_is_refused_on_opening() {
        let fixtures: [(&str, &[u8]); 3] = [
            (
                "format112",
                include_bytes!("../../../tests/fixtures/persistence/format112.bin"),
            ),
            (
                "release-0.3.0",
                include_bytes!("../../../tests/fixtures/persistence/release-0.3.0.bin"),
            ),
            (
                "format98",
                include_bytes!(
                    "../../../tests/fixtures/persistence/legacy/joint-records-format98.bin"
                ),
            ),
        ];
        let mut covered = BTreeSet::new();
        for (name, fixture) in fixtures {
            for (collection, product) in misfiled(fixture) {
                let snapshot = Snapshot {
                    revision_id: 0,
                    product: Arc::new(product),
                };
                let opened = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    load(&save(&snapshot)).map(|_| ())
                }));
                let Ok(Err(error)) = opened else {
                    panic!("{name}: a misfiled {collection} entry was not refused: {opened:?}");
                };
                let message = error.to_string();
                assert!(
                    message.contains(collection) && message.contains("names itself"),
                    "{name}, {collection}: {message}"
                );
                covered.insert(collection);
            }
        }
        for collection in ["groups", "occurrences", "definitions", "features", "tags"] {
            assert!(covered.contains(collection), "no fixture has {collection}");
        }
    }

    /// The stored map `field` of the product in `snapshot` (a CBOR
    /// `{revision_id, product}`), its first integer key (or `all` of them)
    /// moved by `RENAMED`, so the entry is not filed under the id it names nor
    /// found under the id other entries name it by. Whether there was such a key.
    fn misfile_stored(snapshot: &mut ciborium::Value, field: &str, all: bool) -> bool {
        fn entry<'a>(
            value: &'a mut ciborium::Value,
            name: &str,
        ) -> Option<&'a mut ciborium::Value> {
            value.as_map_mut().and_then(|entries| {
                entries
                    .iter_mut()
                    .find(|(key, _)| key.as_text() == Some(name))
                    .map(|(_, value)| value)
            })
        }
        let Some(ciborium::Value::Map(entries)) =
            entry(snapshot, "product").and_then(|product| entry(product, field))
        else {
            return false;
        };
        let mut moved = 0;
        for (key, _) in entries.iter_mut().take(if all { usize::MAX } else { 1 }) {
            if let ciborium::Value::Integer(number) = key {
                *number = (u64::try_from(*number).expect("an id") + RENAMED).into();
                moved += 1;
            }
        }
        moved > 0
    }

    /// R1122ac-PS-1100 as the review found it: the files of older formats
    /// (98, 108, 112, release 0.3.0, the assistant's), an entry filed under
    /// another id in the format they were written in and sealed again, are
    /// refused by the collection, before the format-98 assembly recipe or a
    /// migration looks an entry up, never a panic nor opened.
    #[test]
    fn an_older_format_entry_filed_under_another_id_is_refused_on_opening() {
        let fixtures: [(&str, &[u8]); 5] = [
            (
                "format98",
                include_bytes!(
                    "../../../tests/fixtures/persistence/legacy/joint-records-format98.bin"
                ),
            ),
            (
                "format108",
                include_bytes!("../../../tests/fixtures/persistence/format108.bin"),
            ),
            (
                "format112",
                include_bytes!("../../../tests/fixtures/persistence/format112.bin"),
            ),
            (
                "release-0.3.0",
                include_bytes!("../../../tests/fixtures/persistence/release-0.3.0.bin"),
            ),
            (
                "intent_entities",
                include_bytes!(
                    "../../../../ketchup-assistant/tests/fixtures/intent_entities.ketchup"
                ),
            ),
        ];
        let collections = [
            ("groups", "groups"),
            ("occurrences", "occurrences"),
            ("definitions", "definitions"),
            ("features", "features"),
            ("tags", "tags"),
            ("classification_dimensions", "classification dimensions"),
        ];
        let mut covered = BTreeSet::new();
        let mut failures = Vec::new();
        for (name, fixture) in fixtures {
            for ((field, collection), all) in collections
                .into_iter()
                .flat_map(|collection| [(collection, false), (collection, true)])
            {
                let mut found = false;
                let bytes =
                    crate::persistence::snapshot_codec::rewrite_in_container(fixture, |snapshot| {
                        found = misfile_stored(snapshot, field, all);
                    });
                if !found {
                    continue;
                }
                let opened = std::panic::catch_unwind(|| load(&bytes).map(|_| ()));
                match opened {
                    Ok(Err(error))
                        if error.to_string().contains(collection)
                            && error.to_string().contains("names itself") => {}
                    Ok(Err(error)) => {
                        failures.push(format!("{name}, {collection}, all {all}: {error}"))
                    }
                    Ok(Ok(())) => failures.push(format!("{name}, {collection}, all {all}: opened")),
                    Err(_) => failures.push(format!("{name}, {collection}, all {all}: panicked")),
                }
                covered.insert((name, field));
            }
        }
        assert!(
            failures.is_empty(),
            "not refused by the collection:\n{}",
            failures.join("\n")
        );
        for (name, field) in [
            ("format98", "occurrences"),
            ("format108", "groups"),
            ("format112", "groups"),
            ("format112", "occurrences"),
            ("release-0.3.0", "occurrences"),
            ("intent_entities", "groups"),
        ] {
            assert!(covered.contains(&(name, field)), "{name} has no {field}");
        }
    }
}
