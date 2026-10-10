//! Export of an evaluated model to a `.ketchup-view` package: display geometry,
//! component metadata, author notes and saved scenes, without CAD history or programs.

use crate::blender_export::{MeshGlbInstance, model_glb_export_including_hidden};
use ketchup_model::document::{
    DefinitionId, InstancePath, InstancePathStep, LocalOccurrenceKey, SavedViewId, Snapshot,
};
use ketchup_model::exact_product::MeshExportSource;
use ketchup_rejection::{Rejection, RejectionPhase};
use ketchup_view_format as view;
use std::collections::{BTreeMap, BTreeSet};

mod scenes;
pub use scenes::DesktopCameraContext;

/// Author-side data is supplied by the project/program adapter, never guessed
/// from names, colors or a world-space bounding box.
#[derive(Clone, Debug, Default)]
pub struct DefinitionMetadata {
    pub note: Option<String>,
    pub material: Option<String>,
    pub authored_size_mm: Option<[f64; 3]>,
}

/// Author-side data of one occurrence, keyed by its instance path.
#[derive(Clone, Debug, Default)]
pub struct OccurrenceMetadata {
    pub note: Option<String>,
    pub material: Option<String>,
    pub attributes: BTreeMap<String, String>,
}

/// Everything the export adds to the evaluated geometry.
#[derive(Clone, Debug, Default)]
pub struct ViewerExportData {
    pub definitions: BTreeMap<DefinitionId, DefinitionMetadata>,
    pub occurrences: BTreeMap<InstancePath, OccurrenceMetadata>,
    /// Already converted to world/mm and short-axis framing by the UI adapter.
    pub scenes: Vec<view::Scene>,
    /// Selected document views, evaluated in the explicitly supplied export
    /// viewport. Legacy saved views do not store their original viewport size.
    pub saved_views: BTreeMap<SavedViewId, DesktopCameraContext>,
    pub dimensions: Vec<view::Dimension>,
    pub start_scene: Option<u64>,
}

/// The package and a line-per-loss report of what it does not carry.
pub struct ViewerExport {
    pub package: view::Package,
    pub loss_report: String,
}

/// Export already evaluated bodies; this does not execute programs, recompute
/// CAD, or mutate the source. Bounds describe the exported bodies in definition
/// axes. Empty assembly ancestors have no invented dimensions or materials.
pub fn model_viewer_export(
    snapshot: &Snapshot,
    instances: &[MeshGlbInstance<'_>],
    data: &ViewerExportData,
) -> Result<ViewerExport, Rejection> {
    let export = model_glb_export_including_hidden(snapshot, instances).map_err(|error| {
        invalid(
            "geometry",
            "The current model geometry could not be exported.",
        )
        .caused_by(error)
    })?;
    let bounds = definition_bounds(instances)?;
    let mut definitions = BTreeSet::new();
    let mut occurrences = Vec::with_capacity(export.occurrence_nodes.len());
    for (path, &glb_node) in &export.occurrence_nodes {
        let occurrence = source_occurrence(snapshot, path, glb_node)?;
        definitions.insert(DefinitionId(occurrence.definition_id));
        for step in &occurrence.path.steps {
            let owner = match step {
                view::PathStep::Group {
                    owner_definition_id,
                    ..
                }
                | view::PathStep::Occurrence {
                    owner_definition_id,
                    ..
                } => *owner_definition_id,
            };
            definitions.insert(DefinitionId(owner));
        }
        let mut occurrence = occurrence;
        if let Some(metadata) = data.occurrences.get(path) {
            occurrence.note.clone_from(&metadata.note);
            occurrence.material.clone_from(&metadata.material);
            occurrence.attributes.clone_from(&metadata.attributes);
        }
        occurrences.push(occurrence);
    }
    let definitions = definitions
        .into_iter()
        .map(|id| {
            let definition = snapshot.definition(id).ok_or_else(|| {
                invalid(
                    "definitions",
                    "An exported component definition is missing.",
                )
            })?;
            let metadata = data.definitions.get(&id);
            let local_dimensions =
                metadata
                    .and_then(|metadata| metadata.authored_size_mm)
                    .map(|size_mm| view::LocalDimensions::AuthoredAxes { size_mm })
                    .or_else(|| {
                        bounds.get(&id).map(|&[min_mm, max_mm]| {
                            view::LocalDimensions::LocalBounds { min_mm, max_mm }
                        })
                    });
            Ok(view::Definition {
                id: id.0,
                name: definition.name().to_owned(),
                note: metadata.and_then(|metadata| metadata.note.clone()),
                material: metadata.and_then(|metadata| metadata.material.clone()),
                color_srgb: None,
                local_dimensions,
            })
        })
        .collect::<Result<Vec<_>, Rejection>>()?;
    let mut scenes = data.scenes.clone();
    let mut loss_report = export.loss_report;
    loss_report.push_str("viewer_edges=tessellated CAD face boundaries; canonical mesh boundaries and creases (normal dot < 0.95); not exact topological curves\n");
    for (id, context) in &data.saved_views {
        let saved = snapshot.saved_view(*id).ok_or_else(|| {
            invalid(
                "scenes",
                "A selected saved view is missing from this document.",
            )
        })?;
        let (scene, omitted) = scenes::saved_scene(snapshot, saved, context, &occurrences)?;
        if !omitted.is_empty() {
            use std::fmt::Write;
            writeln!(
                loss_report,
                "viewer_scene {} omitted_display_flags={}",
                id.0,
                omitted.join(",")
            )
            .expect("writing to a String cannot fail");
        }
        scenes.push(scene);
    }
    let package = view::Package {
        manifest: view::Manifest {
            source: view::Source {
                document_id: snapshot.document_id().0,
                revision: snapshot.revision_id(),
                canonical_digest: snapshot.canonical_digest(),
            },
            definitions,
            occurrences,
            scenes,
            dimensions: data.dimensions.clone(),
            start_scene: data.start_scene,
        },
        glb: export.glb,
    };
    // Use the same bounded writer/validator as the saved file, including the
    // metadata byte limit, without building a second copy of the geometry.
    package.write(std::io::sink())?;
    Ok(ViewerExport {
        package,
        loss_report,
    })
}

/// The manifest entry of one exported occurrence, with its name and color.
fn source_occurrence(
    snapshot: &Snapshot,
    path: &InstancePath,
    glb_node: usize,
) -> Result<view::Occurrence, Rejection> {
    let root = snapshot.occurrence(path.root_occurrence()).ok_or_else(|| {
        invalid(
            "occurrences.path",
            "The exported root occurrence is missing.",
        )
    })?;
    let mut definition_id = root.definition_id();
    let mut name = root.name();
    let mut color = root.color();
    let mut steps = Vec::with_capacity(path.steps().len());
    for step in path.steps() {
        match step {
            InstancePathStep::Group(id) => steps.push(view::PathStep::Group {
                owner_definition_id: definition_id.0,
                local_id: id.0,
            }),
            InstancePathStep::Occurrence(id) => {
                steps.push(view::PathStep::Occurrence {
                    owner_definition_id: definition_id.0,
                    local_id: id.0,
                });
                let local = snapshot
                    .local_occurrence(LocalOccurrenceKey {
                        definition_id,
                        local_id: *id,
                    })
                    .ok_or_else(|| {
                        invalid(
                            "occurrences.path",
                            "An exported nested occurrence is missing.",
                        )
                    })?;
                definition_id = local.definition_id();
                name = local.name();
                color = color.or(local.color());
            }
        }
    }
    Ok(view::Occurrence {
        name: name.to_owned(),
        path: view::InstancePath {
            root_occurrence_id: root.id().0,
            steps,
        },
        definition_id: definition_id.0,
        glb_node,
        note: None,
        material: None,
        color_srgb: color,
        attributes: BTreeMap::new(),
    })
}

/// Bounds of each definition's bodies in its own axes.
fn definition_bounds(
    instances: &[MeshGlbInstance<'_>],
) -> Result<BTreeMap<DefinitionId, [[f64; 3]; 2]>, Rejection> {
    let mut bounds = BTreeMap::<DefinitionId, [[f64; 3]; 2]>::new();
    let mut seen = BTreeSet::new();
    for instance in instances {
        let source = instance.source;
        let id = source.definition_id();
        if !seen.insert((id, source.producer_feature_id())) {
            continue;
        }
        let body_bounds = match source {
            MeshExportSource::Exact(package) => package.bounds_mm(),
            MeshExportSource::Canonical { mesh, .. } => {
                let first = *mesh.vertices_mm.first().ok_or_else(|| {
                    invalid("geometry.bounds", "An exported body has no vertices.")
                })?;
                let mut bounds = [first, first];
                for point in &mesh.vertices_mm {
                    include_bounds(&mut bounds, [*point, *point]);
                }
                bounds
            }
        };
        bounds
            .entry(id)
            .and_modify(|bounds| include_bounds(bounds, body_bounds))
            .or_insert(body_bounds);
    }
    Ok(bounds)
}

fn include_bounds(bounds: &mut [[f64; 3]; 2], other: [[f64; 3]; 2]) {
    for axis in 0..3 {
        bounds[0][axis] = bounds[0][axis].min(other[0][axis]);
        bounds[1][axis] = bounds[1][axis].max(other[1][axis]);
    }
}

fn invalid(target: &str, reason: &str) -> Rejection {
    Rejection::new("viewer.export.invalid", RejectionPhase::Request)
        .target(target)
        .reason(reason)
        .fix_hint("Refresh the model geometry and export only metadata and scenes belonging to these components.")
}
