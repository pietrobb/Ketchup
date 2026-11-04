//! The drawing set previewed beside the project drawings window, a sheet at a time,
//! painted as the PDF prints it: the orthophoto beneath the site plan and the plans,
//! then the fills, lines and texts. The sheets are made on a thread as the PDF export
//! makes them, the orthophotos decoded and shrunk to their preview there too; each
//! becomes a texture once, when the preview arrives, never per frame.

use super::annotations_view::drawn_text;
use super::project_drawings::{ProjectDrawingsError, ProjectDrawingsJob};
use crate::*;
use ketchup_manufacturing::project_drawings::DrawingSet;
use ketchup_model::sheet_pdf::{Mark, Page, SheetImage};
use std::sync::mpsc::{self, Receiver, TryRecvError};

/// The longest side of an orthophoto's texture, in pixels: a sheet's image at
/// 300 dpi may exceed what a GPU texture holds, and the preview needs far less.
const MAX_TEXTURE_SIDE_PX: u32 = 2048;
/// The thinnest line the preview paints, in screen points.
const MIN_LINE_POINTS: f32 = 0.5;

/// An orthophoto of the set and its pixels for the preview's texture.
struct PreviewImage {
    image: Arc<SheetImage>,
    pixels: egui::ColorImage,
}

type MadePreview = Result<(DrawingSet, Vec<PreviewImage>), ProjectDrawingsError>;

struct PreviewTask {
    /// Set when the task is dropped, the preview closed, made again or another
    /// document opened: stops the hidden-line removal.
    cancelled: Arc<AtomicBool>,
    receiver: Receiver<MadePreview>,
}

impl Drop for PreviewTask {
    fn drop(&mut self) {
        self.cancelled
            .store(true, std::sync::atomic::Ordering::Release);
    }
}

/// The preview window and the sheets it shows.
#[derive(Default)]
pub(crate) struct SheetPreviewState {
    open: bool,
    task: Option<PreviewTask>,
    set: Option<DrawingSet>,
    /// The sheet shown, by its index in the set.
    shown: usize,
    /// A texture per orthophoto of the set, by the image it shows.
    textures: Vec<(Arc<SheetImage>, egui::TextureHandle)>,
    error: Option<String>,
}

/// The set of `job` as the PDF prints it, and the preview's pixels of each of its
/// orthophotos once.
fn made_preview(job: ProjectDrawingsJob, cancelled: &AtomicBool) -> MadePreview {
    let (set, _) = job.set(true, cancelled)?;
    let mut images: Vec<PreviewImage> = Vec::new();
    for mark in set.sheets.iter().flat_map(|sheet| &sheet.page.marks) {
        if let Mark::Image { image, .. } = mark
            && !images.iter().any(|known| Arc::ptr_eq(&known.image, image))
        {
            images.push(PreviewImage {
                image: Arc::clone(image),
                pixels: preview_pixels(image),
            });
        }
    }
    Ok((set, images))
}

/// `image`'s pixels, shrunk to at most [`MAX_TEXTURE_SIDE_PX`] on its longer side.
fn preview_pixels(image: &SheetImage) -> egui::ColorImage {
    let [width, height] = [image.width_px, image.height_px];
    let size = pixel_size(width, height);
    if image.rgb.len() != size[0] * size[1] * 3 {
        return egui::ColorImage::from_rgb([1, 1], &[255, 255, 255]);
    }
    let longest = width.max(height);
    if longest <= MAX_TEXTURE_SIDE_PX {
        return egui::ColorImage::from_rgb(size, &image.rgb);
    }
    let Some(rgb) = image::RgbImage::from_raw(width, height, image.rgb.clone()) else {
        return egui::ColorImage::from_rgb([1, 1], &[255, 255, 255]);
    };
    let shrunk = |side: u32| {
        u32::try_from(u64::from(side) * u64::from(MAX_TEXTURE_SIDE_PX) / u64::from(longest))
            .unwrap_or(MAX_TEXTURE_SIDE_PX)
            .max(1)
    };
    let small = image::imageops::thumbnail(&rgb, shrunk(width), shrunk(height));
    egui::ColorImage::from_rgb(pixel_size(small.width(), small.height()), small.as_raw())
}

/// Width and height in pixels as egui counts them.
fn pixel_size(width: u32, height: u32) -> [usize; 2] {
    [width, height].map(|side| usize::try_from(side).unwrap_or(usize::MAX))
}

/// What `page` is painted as with its width filling `rect`, each shape with the
/// rectangle it is clipped to, in the order of its marks: an orthophoto stays
/// beneath what is drawn after it. An image with no texture in `textures` is left out.
pub(crate) fn page_shapes(
    context: &egui::Context,
    page: &Page,
    rect: Rect,
    textures: &[(Arc<SheetImage>, egui::TextureId)],
) -> Vec<(Rect, egui::Shape)> {
    let scale = f64::from(rect.width()) / page.size_mm[0].max(1.0);
    let screen = |[x, y]: [f64; 2]| {
        let [x, y, _] = renderer::single_precision([x * scale, y * scale, 0.0]);
        rect.min + egui::vec2(x, y)
    };
    let stroke_of = |stroke: &ketchup_model::sheet_pdf::Stroke| {
        let [width, _, _] = renderer::single_precision([stroke.width_mm * scale, 0.0, 0.0]);
        let [r, g, b] = stroke.color;
        Stroke::new(width.max(MIN_LINE_POINTS), Color32::from_rgb(r, g, b))
    };
    let mut shapes = Vec::new();
    for mark in &page.marks {
        match mark {
            Mark::Fill {
                loops,
                color,
                outline,
            } => {
                let loops = loops
                    .iter()
                    .map(|points| points.iter().copied().map(screen).collect::<Vec<_>>())
                    .collect::<Vec<_>>();
                let [r, g, b] = *color;
                let mesh = even_odd_mesh(&loops, Color32::from_rgb(r, g, b));
                shapes.push((rect, egui::Shape::mesh(mesh)));
                if let Some(outline) = outline {
                    for points in loops {
                        shapes.push((rect, egui::Shape::closed_line(points, stroke_of(outline))));
                    }
                }
            }
            Mark::Line {
                points,
                closed,
                stroke,
            } => {
                let mut points = points.iter().copied().map(screen).collect::<Vec<_>>();
                let painted = stroke_of(stroke);
                if let [dash, rest @ ..] = stroke.dash_mm.as_slice() {
                    if *closed && let Some(first) = points.first().copied() {
                        points.push(first);
                    }
                    let gap = rest.first().unwrap_or(dash);
                    let [dash, gap, _] =
                        renderer::single_precision([dash * scale, gap * scale, 0.0]);
                    shapes.extend(
                        egui::Shape::dashed_line(&points, painted, dash.max(1.0), gap.max(1.0))
                            .into_iter()
                            .map(|shape| (rect, shape)),
                    );
                } else if *closed {
                    shapes.push((rect, egui::Shape::closed_line(points, painted)));
                } else {
                    shapes.push((rect, egui::Shape::line(points, painted)));
                }
            }
            Mark::Text {
                at,
                text,
                height_mm,
                anchor,
                angle_deg,
                ..
            } => {
                let drawn = drawn_text(
                    context,
                    screen(*at),
                    text.clone(),
                    height_mm * scale,
                    *anchor,
                    *angle_deg,
                );
                let shape = egui::epaint::TextShape::new(drawn.pos, drawn.galley, Color32::BLACK)
                    .with_angle(drawn.angle);
                shapes.push((rect, shape.into()));
            }
            Mark::Image {
                image,
                corners,
                clip,
            } => {
                let Some(texture) = textures
                    .iter()
                    .find(|(known, _)| Arc::ptr_eq(known, image))
                    .map(|(_, texture)| *texture)
                else {
                    continue;
                };
                let [upper_left, upper_right, lower_left] = corners.map(screen);
                let lower_right = upper_right + (lower_left - upper_left);
                let mut mesh = egui::Mesh::with_texture(texture);
                for (pos, uv) in [
                    (upper_left, [0.0, 0.0]),
                    (upper_right, [1.0, 0.0]),
                    (lower_right, [1.0, 1.0]),
                    (lower_left, [0.0, 1.0]),
                ] {
                    mesh.vertices.push(egui::epaint::Vertex {
                        pos,
                        uv: egui::pos2(uv[0], uv[1]),
                        color: Color32::WHITE,
                    });
                }
                mesh.add_triangle(0, 1, 2);
                mesh.add_triangle(0, 2, 3);
                let clip = clip.map_or(rect, |[min, max]| {
                    Rect::from_two_pos(screen(min), screen(max)).intersect(rect)
                });
                shapes.push((clip, egui::Shape::mesh(mesh)));
            }
        }
    }
    shapes
}

/// The triangles that fill `loops` even-odd, a loop inside another a hole, as the PDF
/// fills them: between each two heights of their corners, the band is cut where their
/// edges cross it, every other piece filled.
fn even_odd_mesh(loops: &[Vec<Pos2>], color: Color32) -> egui::Mesh {
    let edges = loops
        .iter()
        .flat_map(|points| {
            (0..points.len()).map(move |index| [points[index], points[(index + 1) % points.len()]])
        })
        .filter(|[a, b]| a.y != b.y)
        .collect::<Vec<_>>();
    let mut heights = edges
        .iter()
        .flat_map(|[a, b]| [a.y, b.y])
        .collect::<Vec<_>>();
    heights.sort_by(f32::total_cmp);
    heights.dedup();
    let x_at = |[a, b]: [Pos2; 2], y: f32| a.x + (b.x - a.x) * (y - a.y) / (b.y - a.y);
    let mut mesh = egui::Mesh::default();
    for band in heights.windows(2) {
        let [top, bottom] = [band[0], band[1]];
        let middle = 0.5 * (top + bottom);
        let mut crossings = edges
            .iter()
            .filter(|[a, b]| a.y.min(b.y) < middle && middle < a.y.max(b.y))
            .map(|edge| [x_at(*edge, middle), x_at(*edge, top), x_at(*edge, bottom)])
            .collect::<Vec<_>>();
        crossings.sort_by(|a, b| a[0].total_cmp(&b[0]));
        for pair in crossings.chunks_exact(2) {
            let first = u32::try_from(mesh.vertices.len()).unwrap_or(u32::MAX);
            for corner in [
                egui::pos2(pair[0][1], top),
                egui::pos2(pair[1][1], top),
                egui::pos2(pair[1][2], bottom),
                egui::pos2(pair[0][2], bottom),
            ] {
                mesh.colored_vertex(corner, color);
            }
            mesh.add_triangle(first, first + 1, first + 2);
            mesh.add_triangle(first, first + 2, first + 3);
        }
    }
    mesh
}

impl KetchupApp {
    /// Opens the preview and makes the set again on a thread; a preview still being
    /// made is stopped.
    pub(crate) fn start_sheet_preview(&mut self, context: &egui::Context) {
        self.drawings.preview.task = None;
        self.drawings.preview.open = true;
        self.drawings.preview.error = None;
        let job = match self.project_drawings_job() {
            Ok(job) => job,
            Err(error) => {
                self.drawings.preview.error = Some(self.project_drawings_error_text(&error));
                return;
            }
        };
        let (sender, receiver) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let thread_cancelled = Arc::clone(&cancelled);
        let repaint = context.clone();
        std::thread::spawn(move || {
            if sender.send(made_preview(job, &thread_cancelled)).is_ok() {
                repaint.request_repaint();
            }
        });
        self.drawings.preview.task = Some(PreviewTask {
            cancelled,
            receiver,
        });
    }

    fn poll_sheet_preview(&mut self, context: &egui::Context) {
        let Some(task) = &self.drawings.preview.task else {
            return;
        };
        let made = match task.receiver.try_recv() {
            Ok(made) => Some(made),
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => None,
        };
        self.drawings.preview.task = None;
        self.receive_sheet_preview(context, made);
    }

    /// Waits for the preview being made: for tests, which have no frames to poll it.
    #[cfg(test)]
    pub(crate) fn finish_sheet_preview(&mut self, context: &egui::Context) {
        if let Some(task) = self.drawings.preview.task.take() {
            let made = task.receiver.recv().ok();
            self.receive_sheet_preview(context, made);
        }
    }

    /// Keeps the set `made` and makes each of its orthophotos a texture, once.
    fn receive_sheet_preview(&mut self, context: &egui::Context, made: Option<MadePreview>) {
        let ended = || std::io::Error::other("the sheet preview ended unexpectedly");
        match made.unwrap_or_else(|| Err(ProjectDrawingsError::Write(ended()))) {
            Ok((set, images)) => {
                let preview = &mut self.drawings.preview;
                preview.textures = images
                    .into_iter()
                    .enumerate()
                    .map(|(index, PreviewImage { image, pixels })| {
                        let name = format!("sheet-preview-image-{index}");
                        (
                            image,
                            context.load_texture(name, pixels, egui::TextureOptions::LINEAR),
                        )
                    })
                    .collect();
                preview.shown = preview.shown.min(set.sheets.len().saturating_sub(1));
                preview.set = Some(set);
            }
            Err(error) => {
                self.drawings.preview.error = Some(self.project_drawings_error_text(&error));
            }
        }
    }

    /// What the shown sheet is painted as in `rect`, as [`page_shapes`] paints it.
    pub(crate) fn sheet_preview_shapes(
        &self,
        context: &egui::Context,
        rect: Rect,
    ) -> Vec<(Rect, egui::Shape)> {
        let preview = &self.drawings.preview;
        let Some(sheet) = preview
            .set
            .as_ref()
            .and_then(|set| set.sheets.get(preview.shown))
        else {
            return Vec::new();
        };
        let textures = preview
            .textures
            .iter()
            .map(|(image, texture)| (Arc::clone(image), texture.id()))
            .collect::<Vec<_>>();
        page_shapes(context, &sheet.page, rect, &textures)
    }

    pub(crate) fn show_sheet_preview_window(&mut self, context: &egui::Context) {
        self.poll_sheet_preview(context);
        if !self.drawings.preview.open {
            return;
        }
        let mut open = true;
        let mut again = false;
        egui::Window::new(self.catalog.text("window-sheet-preview"))
            .open(&mut open)
            .default_size([720.0, 540.0])
            .resizable(true)
            .show(context, |ui| {
                ui.horizontal(|ui| {
                    let preview = &mut self.drawings.preview;
                    if let Some(set) = &preview.set {
                        let name = |index: usize| {
                            set.sheets.get(index).map_or_else(String::new, |sheet| {
                                format!("{} {}", sheet.number, sheet.title)
                            })
                        };
                        egui::ComboBox::from_id_salt("sheet-preview-sheet")
                            .selected_text(name(preview.shown))
                            .show_ui(ui, |ui| {
                                for index in 0..set.sheets.len() {
                                    ui.selectable_value(&mut preview.shown, index, name(index));
                                }
                            });
                    }
                    let making = preview.task.is_some();
                    if making {
                        ui.spinner();
                    }
                    again = ui
                        .add_enabled(
                            !making,
                            egui::Button::new(self.catalog.text("drawings-preview-again")),
                        )
                        .clicked();
                });
                if let Some(error) = &self.drawings.preview.error {
                    ui.colored_label(ui.visuals().warn_fg_color, error);
                }
                let Some(size_mm) = self
                    .drawings
                    .preview
                    .set
                    .as_ref()
                    .and_then(|set| set.sheets.get(self.drawings.preview.shown))
                    .map(|sheet| sheet.page.size_mm)
                else {
                    return;
                };
                let available = ui.available_size();
                let [width, height, _] = renderer::single_precision([size_mm[0], size_mm[1], 0.0]);
                let fit = (available.x / width).min(available.y / height).max(0.0);
                let (response, painter) =
                    ui.allocate_painter(egui::vec2(width, height) * fit, Sense::hover());
                painter.rect_filled(response.rect, 0.0, Color32::WHITE);
                for (clip, shape) in self.sheet_preview_shapes(context, response.rect) {
                    painter.with_clip_rect(clip).add(shape);
                }
            });
        if !open {
            self.drawings.preview = SheetPreviewState::default();
        } else if again {
            self.start_sheet_preview(context);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A wall cut around a room, a square with a square hole, fills only the wall,
    /// as the PDF fills it even-odd.
    #[test]
    fn a_fill_leaves_its_hole_open() {
        let square = |from: f32, to: f32| {
            vec![
                egui::pos2(from, from),
                egui::pos2(to, from),
                egui::pos2(to, to),
                egui::pos2(from, to),
            ]
        };
        let mesh = even_odd_mesh(&[square(0.0, 10.0), square(2.0, 8.0)], Color32::BLACK);
        let triangles = mesh
            .indices
            .chunks_exact(3)
            .map(|triangle| [0, 1, 2].map(|corner| mesh.vertices[triangle[corner] as usize].pos))
            .collect::<Vec<_>>();
        let area: f32 = triangles
            .iter()
            .map(|[a, b, c]| 0.5 * ((*b - *a).x * (*c - *a).y - (*b - *a).y * (*c - *a).x).abs())
            .sum();
        assert!(
            (area - 64.0).abs() < 1e-3,
            "100 less the 36 of the hole: {area}"
        );
        let filled = |point: Pos2| {
            triangles
                .iter()
                .any(|triangle| crate::screen_triangle_contains_point(*triangle, point))
        };
        assert!(filled(egui::pos2(1.0, 5.0)), "the wall");
        assert!(!filled(egui::pos2(5.0, 5.0)), "the room");
    }
}
