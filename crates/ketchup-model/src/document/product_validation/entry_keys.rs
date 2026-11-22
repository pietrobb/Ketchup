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
    /// panic. The fixtures hold groups, occurrences, an assembly recipe
    /// (format 98) and every other collection checked between them.
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
}
