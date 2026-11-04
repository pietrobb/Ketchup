//! An image — an orthophoto, a scanned map — read as a raster underlay (Měchenice
//! MECH-10(a)): its file kept as a blob of the document, laid out by the world file beside
//! it in the coordinates of its grid, and moved near the origin by the offset of a drawing
//! of the document in the same grid, so it lies on that drawing.

use super::underlay::ReadUnderlay;
use crate::import_source::BoxedCause;
use crate::renderer::underlay::UnderlayGeometry;
use crate::renderer::underlay::raster::RasterImage;
use crate::*;
use ketchup_application::program_isolation::UnderlayRead;
use ketchup_model::import::{ImportUnitAuthority, ImportUnitDecision};
use ketchup_model::underlay::UnderlayLines;
use ketchup_model::underlay::raster::{
    MAX_RASTER_FILE_BYTES, RasterFormat, UnderlayRaster, WorldFile, lay_out_raster,
};
use ketchup_model::underlay::snap::UnderlaySnapIndex;
use std::io::Read as _;

/// What was read of an image besides what every underlay has: where its pixels lie, its
/// file and its pixels.
pub(crate) struct ReadRaster {
    pub(crate) raster: UnderlayRaster,
    pub(crate) bytes: Vec<u8>,
    pub(crate) image: Arc<RasterImage>,
    /// The world file it was laid out by, or none: a pixel a unit from the origin.
    pub(crate) world_file: Option<PathBuf>,
    /// Whether the unit of the world file's numbers was given; else they are metres.
    unit_given: bool,
}

/// The format of an image `path` names by its extension.
pub(crate) fn raster_format(path: &Path) -> Option<RasterFormat> {
    path.extension()
        .and_then(|extension| extension.to_str())
        .and_then(RasterFormat::from_extension)
}

/// The world file beside `path`, an image of `format`, if there is one.
fn world_file_of(path: &Path, format: RasterFormat) -> Option<PathBuf> {
    format
        .world_file_extensions()
        .iter()
        .map(|extension| path.with_extension(extension))
        .find(|candidate| candidate.is_file())
}

fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, BoxedCause> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    if !u64::try_from(bytes.len()).is_ok_and(|length| length <= limit) {
        return Err(format!(
            "{} is larger than the {} MiB an image underlay takes",
            path.display(),
            limit / 1024 / 1024
        )
        .into());
    }
    Ok(bytes)
}

/// Reads the image at `path`, of `format`, as an underlay, the numbers of its world file in
/// `unit` (metres when none is given, as the grids of national surveys are), moved by
/// `geo_origin_mm` when that keeps it within reach.
pub(crate) fn read_raster_file(
    path: &Path,
    format: RasterFormat,
    unit: Option<ImportLengthUnit>,
    geo_origin_mm: Option<[f64; 2]>,
) -> Result<ReadUnderlay, ImportError> {
    let failed = |cause: BoxedCause| ImportError::failed(ImportKind::Image(format), cause);
    let bytes = read_bounded(path, MAX_RASTER_FILE_BYTES).map_err(failed)?;
    let world_file = world_file_of(path, format);
    let decision = ImportUnitDecision::new(
        unit.unwrap_or(ImportLengthUnit::Metre),
        ImportUnitAuthority::UserDeclared,
    );
    let world = world_file
        .as_deref()
        .map(|world_path| {
            let text = read_bounded(world_path, 4096)?;
            let text = String::from_utf8(text)?;
            Ok::<_, BoxedCause>(WorldFile::parse(&text, decision.millimetres_per_unit())?)
        })
        .transpose()
        .map_err(failed)?;
    let image_format = match format {
        RasterFormat::Png => image::ImageFormat::Png,
        RasterFormat::Jpeg => image::ImageFormat::Jpeg,
    };
    let (width_px, height_px) =
        image::ImageReader::with_format(std::io::Cursor::new(&bytes), image_format)
            .into_dimensions()
            .map_err(|error| failed(error.into()))?;
    let (raster, offset_mm) = lay_out_raster(
        (sha256_bytes(&bytes), format),
        [width_px, height_px],
        world,
        geo_origin_mm,
    )
    .map_err(|error| failed(error.into()))?;
    let image =
        Arc::new(RasterImage::decode(&bytes, &raster).map_err(|error| failed(error.into()))?);
    let lines = UnderlayLines::default();
    let lines_bytes = lines.encode();
    Ok(ReadUnderlay {
        read: UnderlayRead {
            layers: Vec::new(),
            hidden_layers: Vec::new(),
            units: decision,
            segments: 0,
            texts: 0,
            offset_mm,
            diagnostics: Vec::new(),
            dwg: None,
        },
        snaps: Arc::new(UnderlaySnapIndex::new(&lines)),
        geometry: Arc::new(UnderlayGeometry::new(sha256_bytes(&lines_bytes), &lines)),
        bytes: lines_bytes,
        lines,
        raster: Some(ReadRaster {
            raster,
            bytes,
            image,
            world_file,
            unit_given: unit.is_some(),
        }),
    })
}

impl KetchupApp {
    /// What the window says of an image imported from `path` as an underlay.
    pub(crate) fn raster_digest(&self, path: &Path, read: &ReadRaster) -> String {
        let placed = match &read.world_file {
            Some(world) => self.catalog.format(
                "digest-raster-world-file",
                &BTreeMap::from([("world", world.display().to_string())]),
            ),
            None => self.catalog.text("digest-raster-no-world-file"),
        };
        let [across, down] = read.raster.pixel_mm();
        let pixel = if (across - down).abs() <= f64::EPSILON * across.abs() {
            format!("{across}")
        } else {
            format!("{across} × {down}")
        };
        self.catalog.format(
            "digest-imported-raster-underlay",
            &BTreeMap::from([
                ("path", path.display().to_string()),
                ("width", read.raster.width_px.to_string()),
                ("height", read.raster.height_px.to_string()),
                ("pixel", pixel),
                ("placed", placed),
            ]),
        )
    }

    /// The drawing or image the user picks to import as an underlay.
    pub(crate) fn choose_underlay_import_path(&mut self) -> Option<PathBuf> {
        let filter_label = self.catalog.text("file-filter-underlay");
        self.dialogs
            .pick_import_path(crate::dialogs::ImportDialogRequest {
                format: ImportFormat::Dxf,
                filter_label: &filter_label,
                extensions: &["dxf", "dwg", "png", "jpg", "jpeg"],
            })
    }
}

impl ReadRaster {
    /// What was read of the image, for a bridge client.
    pub(crate) fn summary(&self) -> serde_json::Value {
        let raster = &self.raster;
        serde_json::json!({
            "format": raster.format.name(),
            "width_px": raster.width_px,
            "height_px": raster.height_px,
            "pixel_mm": raster.pixel_mm(),
            "world_file": self.world_file.is_some(),
            "world_file_path": self.world_file.as_ref().map(|path| path.display().to_string()),
            // A world file states no unit: metres unless the request named one.
            "unit_given": self.unit_given,
        })
    }
}
