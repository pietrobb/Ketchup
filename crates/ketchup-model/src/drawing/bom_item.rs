//! What a bill of materials counts as one item, the same on a drawing sheet as
//! in the fabrication bill of materials (review 2026-10-18 MF-12).

use super::{DrawingError, unresolved_instance};
use crate::document::{
    ClassificationCategoryId, ClassificationDimension, ClassificationError, DefinitionId,
    InstancePath, Snapshot,
};
use crate::validation::{FABRICATION_ROLE_DIMENSION_V1, MATERIAL_DIMENSION_V1};

/// One item of a bill of materials: copies of one definition in one hand, of
/// one material and one fabrication role. Two copies of one definition, one
/// oak and one pine, are two items, as the fabrication BOM lists them.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BomItem {
    pub definition_id: DefinitionId,
    /// The copies are placed mirrored, the opposite hand of the definition
    /// (review 2026-10-17 MF-6).
    pub mirrored: bool,
    pub material: Option<ClassificationCategoryId>,
    pub fabrication_role: Option<ClassificationCategoryId>,
}

/// Reads the item of each instance of one snapshot.
pub struct BomItems<'a> {
    snapshot: &'a Snapshot,
    material: Option<&'a ClassificationDimension>,
    fabrication_role: Option<&'a ClassificationDimension>,
}

impl<'a> BomItems<'a> {
    pub fn new(snapshot: &'a Snapshot) -> Result<Self, DrawingError> {
        let dimension = |name| {
            snapshot
                .classification_dimension_named(name)
                .map_err(DrawingError::Classification)
        };
        Ok(Self {
            snapshot,
            material: dimension(MATERIAL_DIMENSION_V1)?,
            fabrication_role: dimension(FABRICATION_ROLE_DIMENSION_V1)?,
        })
    }

    /// The item `path` is a copy of. Material and role are those of its root
    /// occurrence, which the fabrication BOM reads too.
    pub fn item(&self, path: &InstancePath) -> Result<BomItem, DrawingError> {
        let resolved = self
            .snapshot
            .resolve_instance_path(path)
            .map_err(|error| unresolved_instance(path, error))?;
        let category = |dimension: Option<&ClassificationDimension>| {
            dimension.and_then(|dimension| {
                self.snapshot
                    .occurrence_classification(path.root_occurrence(), dimension.id())
            })
        };
        Ok(BomItem {
            definition_id: resolved.definition_id,
            mirrored: resolved.mirrored(),
            material: category(self.material),
            fabrication_role: category(self.fabrication_role),
        })
    }

    /// The name of the item's material, shown in the sheet's bill of materials.
    pub fn material_name(&self, item: &BomItem) -> Result<Option<&'a str>, DrawingError> {
        let (Some(dimension), Some(category_id)) = (self.material, item.material) else {
            return Ok(None);
        };
        dimension
            .category(category_id)
            .map(|category| Some(category.name()))
            .ok_or(DrawingError::Classification(
                ClassificationError::CategoryMissing {
                    dimension_id: dimension.id(),
                    category_id,
                },
            ))
    }
}
