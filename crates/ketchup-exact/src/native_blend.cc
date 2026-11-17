// Blend surfaces between two surfaces and the measurement of how two surfaces
// meet along a shared edge (position, tangent plane and curvature across it).

#include "native_common.hxx"

#include <GCPnts_AbscissaPoint.hxx>
#include <GeomAPI_ProjectPointOnSurf.hxx>
#include <GeomLProp_SLProps.hxx>
#include <Geom_BSplineCurve.hxx>
#include <Geom_BSplineSurface.hxx>
#include <Geom_Surface.hxx>
#include <TColStd_Array1OfInteger.hxx>
#include <TColStd_Array1OfReal.hxx>
#include <TColStd_HArray1OfReal.hxx>
#include <TColgp_Array2OfPnt.hxx>

#include <tuple>

namespace ketchup::exact {

namespace {

// Interior samples per seam edge; the corners, where a blend also meets its
// free sides, are not part of the seam.
constexpr int SEAM_SAMPLES = 16;

// The only face of a non-solid single-face surface body, or null.
TopoDS_Face single_surface_face(const NativeOperationResult& body) {
  if (!body.valid() || body.impl().shape.IsNull()
      || body.impl().summary.solid_count != 0 || body.impl().summary.face_count != 1) {
    return {};
  }
  return TopoDS::Face(TopExp_Explorer(body.impl().shape, TopAbs_FACE).Current());
}

double distance_to(const gp_Pnt& point, const TopoDS_Shape& shape) {
  BRepExtrema_DistShapeShape distance(BRepBuilderAPI_MakeVertex(point).Vertex(), shape);
  if (!distance.IsDone() || distance.NbSolution() < 1) {
    return std::numeric_limits<double>::infinity();
  }
  return distance.Value();
}

// The surface of a face near a point: unit normal and the normal curvature
// across `tangent` (in the direction normal x tangent).
struct LocalSurface {
  gp_Dir normal;
  double cross_curvature = 0.0;
};

std::optional<LocalSurface> local_surface(
    const occ::handle<Geom_Surface>& surface, const gp_Pnt& point, const gp_Vec& tangent) {
  GeomAPI_ProjectPointOnSurf projection(point, surface);
  if (!projection.IsDone() || projection.NbPoints() == 0) {
    return std::nullopt;
  }
  double u = 0.0;
  double v = 0.0;
  projection.LowerDistanceParameters(u, v);
  GeomLProp_SLProps properties(surface, u, v, 2, tolerances().linear_mm);
  if (!properties.IsNormalDefined()) {
    return std::nullopt;
  }
  const gp_Dir normal = properties.Normal();
  const gp_Vec normal_vector(normal);
  gp_Vec across = normal_vector.Crossed(tangent);
  if (across.Magnitude() <= tolerances().rounding) {
    return std::nullopt;
  }
  across.Normalize();
  // The direction across the seam in surface parameters, then the normal
  // curvature from the first and second fundamental forms.
  const gp_Vec& su = properties.D1U();
  const gp_Vec& sv = properties.D1V();
  const double e = su.Dot(su);
  const double f = su.Dot(sv);
  const double g = sv.Dot(sv);
  const double determinant = e * g - f * f;
  if (!(std::abs(determinant) > tolerances().rounding * std::max(e * g, 1.0))) {
    return std::nullopt;
  }
  const double along_u = across.Dot(su);
  const double along_v = across.Dot(sv);
  const double a = (along_u * g - along_v * f) / determinant;
  const double b = (e * along_v - f * along_u) / determinant;
  const double first_form = e * a * a + 2.0 * f * a * b + g * b * b;
  const double second_form = properties.D2U().Dot(normal_vector) * a * a
      + 2.0 * properties.DUV().Dot(normal_vector) * a * b
      + properties.D2V().Dot(normal_vector) * b * b;
  if (!(first_form > tolerances().negligible)) {
    return std::nullopt;
  }
  return LocalSurface{normal, second_form / first_form};
}

struct SeamMeasure {
  bool found = false;
  std::string failure;
  double length = 0.0;
  double gap = 0.0;
  double angle = 0.0;
  double curvature_difference = 0.0;
  double second_curvature = 0.0;
};

// The edges of `first` that lie on `second`, sampled inside each edge.
SeamMeasure measure_seam(const TopoDS_Face& first, const TopoDS_Face& second) {
  SeamMeasure seam;
  const occ::handle<Geom_Surface> first_surface = BRep_Tool::Surface(first);
  const occ::handle<Geom_Surface> second_surface = BRep_Tool::Surface(second);
  const double on_seam = tolerances().network_surface_approximation_mm;
  TopTools_IndexedMapOfShape edges;
  TopExp::MapShapes(first, TopAbs_EDGE, edges);
  for (int index = 1; index <= edges.Extent(); ++index) {
    const TopoDS_Edge edge = TopoDS::Edge(edges(index));
    if (BRep_Tool::Degenerated(edge)) {
      continue;
    }
    const BRepAdaptor_Curve curve(edge);
    const double start = curve.FirstParameter();
    const double end = curve.LastParameter();
    if (distance_to(curve.Value((start + end) / 2.0), second) > on_seam) {
      continue;
    }
    seam.found = true;
    seam.length += GCPnts_AbscissaPoint::Length(curve);
    for (int sample = 0; sample < SEAM_SAMPLES; ++sample) {
      gp_Pnt point;
      gp_Vec tangent;
      curve.D1(start + (end - start) * (sample + 0.5) / SEAM_SAMPLES, point, tangent);
      const auto on_first = local_surface(first_surface, point, tangent);
      const auto on_second = local_surface(second_surface, point, tangent);
      if (!on_first || !on_second) {
        seam.failure = "The seam has a point where a surface has no normal";
        return seam;
      }
      const double alignment = on_first->normal.Dot(on_second->normal);
      // Opposite face normals describe the same tangent plane; the curvature
      // is then measured against the first surface's normal.
      const double second_curvature =
          alignment < 0.0 ? -on_second->cross_curvature : on_second->cross_curvature;
      seam.gap = std::max(seam.gap, distance_to(point, second));
      seam.angle = std::max(seam.angle, std::acos(std::min(1.0, std::abs(alignment))));
      seam.curvature_difference = std::max(
          seam.curvature_difference, std::abs(on_first->cross_curvature - second_curvature));
      if (std::abs(second_curvature) > std::abs(seam.second_curvature)) {
        seam.second_curvature = second_curvature;
      }
    }
  }
  if (!seam.found) {
    seam.failure = "The surfaces share no edge";
  }
  return seam;
}

// The edge of `face` whose middle lies nearest to `other`; null and a reason
// when two edges are equally near.
TopoDS_Edge facing_edge(const TopoDS_Face& face, const TopoDS_Face& other, double& gap) {
  TopTools_IndexedMapOfShape edges;
  TopExp::MapShapes(face, TopAbs_EDGE, edges);
  TopoDS_Edge nearest;
  double best = std::numeric_limits<double>::infinity();
  double runner_up = std::numeric_limits<double>::infinity();
  for (int index = 1; index <= edges.Extent(); ++index) {
    const TopoDS_Edge edge = TopoDS::Edge(edges(index));
    if (BRep_Tool::Degenerated(edge)) {
      continue;
    }
    const BRepAdaptor_Curve curve(edge);
    const double distance =
        distance_to(curve.Value((curve.FirstParameter() + curve.LastParameter()) / 2.0), other);
    if (distance < best) {
      runner_up = best;
      best = distance;
      nearest = edge;
    } else if (distance < runner_up) {
      runner_up = distance;
    }
  }
  if (!(runner_up - best > tolerances().rounding * std::max(best, 1.0))) {
    return {};
  }
  gap = best;
  return nearest;
}

// The unit direction in `face`'s tangent plane across an edge with tangent
// `tangent` at `point`, times `sign` (from `leaving_sign`); with the face's
// normal and normal curvature across the edge there.
struct EdgeFrame {
  gp_Pnt point;
  gp_Vec leaving;
  gp_Vec normal;
  double cross_curvature = 0.0;
};

std::optional<EdgeFrame> edge_frame(
    const TopoDS_Face& face, const gp_Pnt& point, const gp_Vec& tangent, double sign) {
  const auto local = local_surface(BRep_Tool::Surface(face), point, tangent);
  if (!local) {
    return std::nullopt;
  }
  gp_Vec across = gp_Vec(local->normal).Crossed(tangent);
  across.Normalize();
  return EdgeFrame{point, across * sign, gp_Vec(local->normal), local->cross_curvature};
}

// How far off the middle of an edge `leaving_sign` probes, as a share of the
// edge's length (of at least 1 mm).
constexpr double LEAVING_PROBE_SHARE = 0.001;

// +1 when normal x tangent leaves `face` across `edge`, -1 when it enters,
// probed a little off the middle of the edge; 0 when undecided.
double leaving_sign(const TopoDS_Face& face, const TopoDS_Edge& edge) {
  const BRepAdaptor_Curve curve(edge);
  gp_Pnt middle;
  gp_Vec tangent;
  curve.D1((curve.FirstParameter() + curve.LastParameter()) / 2.0, middle, tangent);
  const auto frame = edge_frame(face, middle, tangent, 1.0);
  if (!frame) {
    return 0.0;
  }
  const double step = LEAVING_PROBE_SHARE * std::max(GCPnts_AbscissaPoint::Length(curve), 1.0);
  const occ::handle<Geom_Surface> surface = BRep_Tool::Surface(face);
  for (const double sign : {1.0, -1.0}) {
    GeomAPI_ProjectPointOnSurf projection(
        middle.Translated(frame->leaving * (step * sign)), surface);
    if (!projection.IsDone() || projection.NbPoints() == 0) {
      return 0.0;
    }
    double u = 0.0;
    double v = 0.0;
    projection.LowerDistanceParameters(u, v);
    BRepClass_FaceClassifier classifier(face, gp_Pnt2d(u, v), tolerances().linear_mm);
    if (classifier.State() == TopAbs_IN) {
      return -sign;
    }
  }
  return 0.0;
}

// Cross sections of the blend along its seams; the surface interpolates them.
constexpr int BLEND_SECTIONS = 33;

// One facing edge walked by arc length, `reversed` from its last end.
struct BlendEdge {
  TopoDS_Face face;
  BRepAdaptor_Curve curve;
  double length = 0.0;
  double sign = 0.0;
  bool reversed = false;
  std::uint8_t order = 0;

  std::optional<EdgeFrame> at(double fraction) const {
    const double along = (reversed ? 1.0 - fraction : fraction) * length;
    GCPnts_AbscissaPoint abscissa(curve, along, curve.FirstParameter());
    if (!abscissa.IsDone()) {
      return std::nullopt;
    }
    gp_Pnt point;
    gp_Vec tangent;
    curve.D1(abscissa.Parameter(), point, tangent);
    return edge_frame(face, point, tangent, sign);
  }
};

// The six control points of the quintic across the gap from `from` to `to`.
// Its end tangents are the chord's length times the direction leaving each
// face (G1 and G2) or the chord itself (G0); its end accelerations are the
// face's normal curvature across the edge times the squared chord along the
// face normal (G2) or zero. A surface through such sections shares each
// face's tangent plane (G1) and also its curvature in every direction (G2)
// along the seam.
std::array<gp_Pnt, 6> blend_section(
    const EdgeFrame& from, std::uint8_t from_order, const EdgeFrame& to, std::uint8_t to_order) {
  const gp_Vec chord(from.point, to.point);
  const double length = chord.Magnitude();
  const gp_Vec start_tangent = from_order == 0 ? chord : from.leaving * length;
  const gp_Vec end_tangent = to_order == 0 ? chord : to.leaving * -length;
  const gp_Vec start_acceleration = from_order == 2
      ? from.normal * (from.cross_curvature * length * length) : gp_Vec(0.0, 0.0, 0.0);
  const gp_Vec end_acceleration = to_order == 2
      ? to.normal * (to.cross_curvature * length * length) : gp_Vec(0.0, 0.0, 0.0);
  // C'(0) = 5 (P1 - P0), C''(0) = 20 (P2 - 2 P1 + P0), and mirrored at t = 1.
  const gp_Pnt p0 = from.point;
  const gp_Pnt p1 = p0.Translated(start_tangent / 5.0);
  const gp_Pnt p2 = p1.Translated(gp_Vec(p0, p1) + start_acceleration / 20.0);
  const gp_Pnt p5 = to.point;
  const gp_Pnt p4 = p5.Translated(end_tangent / -5.0);
  const gp_Pnt p3 = p4.Translated(gp_Vec(p5, p4) + end_acceleration / 20.0);
  return {p0, p1, p2, p3, p4, p5};
}

}  // namespace

std::unique_ptr<NativeOperationResult> blend_surfaces_native(
    const NativeOperationResult& first, std::uint8_t first_order,
    const NativeOperationResult& second, std::uint8_t second_order) noexcept {
  return guarded([&] {
    const TopoDS_Face first_face = single_surface_face(first);
    const TopoDS_Face second_face = single_surface_face(second);
    if (first_face.IsNull() || second_face.IsNull() || first_order > 2 || second_order > 2) {
      return error_result(STATUS_INVALID_PARAMETER,
                          "Surface blend joins two single-face surfaces with G0, G1 or G2");
    }
    if (first_face.IsSame(second_face)) {
      return error_result(STATUS_INVALID_PARAMETER, "Surface blend needs two different surfaces");
    }
    double first_gap = 0.0;
    double second_gap = 0.0;
    const TopoDS_Edge first_edge = facing_edge(first_face, second_face, first_gap);
    const TopoDS_Edge second_edge = facing_edge(second_face, first_face, second_gap);
    if (first_edge.IsNull() || second_edge.IsNull()) {
      return error_result(STATUS_INVALID_PARAMETER,
                          "Two edges of a surface face the other surface equally; "
                          "the edges to blend are ambiguous");
    }
    if (std::min(first_gap, second_gap) <= tolerances().network_surface_approximation_mm) {
      return error_result(STATUS_NO_GEOMETRIC_CHANGE,
                          "The surfaces already meet; there is no gap to blend");
    }
    BlendEdge from{first_face, BRepAdaptor_Curve(first_edge)};
    BlendEdge to{second_face, BRepAdaptor_Curve(second_edge)};
    for (BlendEdge* edge : {&from, &to}) {
      edge->length = GCPnts_AbscissaPoint::Length(edge->curve);
    }
    from.sign = leaving_sign(first_face, first_edge);
    to.sign = leaving_sign(second_face, second_edge);
    if (from.sign == 0.0 || to.sign == 0.0) {
      return error_result(STATUS_INVALID_SHAPE,
                          "The side a facing edge leaves its surface is undecided");
    }
    from.order = first_order;
    to.order = second_order;
    // Walk the second edge so its ends pair with the near ends of the first:
    // the blend's free sides do not cross.
    const gp_Pnt a0 = from.curve.Value(from.curve.FirstParameter());
    const gp_Pnt a1 = from.curve.Value(from.curve.LastParameter());
    const gp_Pnt b0 = to.curve.Value(to.curve.FirstParameter());
    const gp_Pnt b1 = to.curve.Value(to.curve.LastParameter());
    to.reversed = a0.Distance(b0) + a1.Distance(b1) > a0.Distance(b1) + a1.Distance(b0);

    std::array<occ::handle<TColgp_HArray1OfPnt>, 6> rows;
    for (auto& row : rows) {
      row = new TColgp_HArray1OfPnt(1, BLEND_SECTIONS);
    }
    occ::handle<TColStd_HArray1OfReal> parameters = new TColStd_HArray1OfReal(1, BLEND_SECTIONS);
    for (int section = 0; section < BLEND_SECTIONS; ++section) {
      const double fraction = static_cast<double>(section) / (BLEND_SECTIONS - 1);
      const auto start = from.at(fraction);
      const auto end = to.at(fraction);
      if (!start || !end) {
        return error_result(STATUS_INVALID_SHAPE,
                            "A facing edge has a point where its surface has no normal");
      }
      const auto poles = blend_section(*start, from.order, *end, to.order);
      for (std::size_t row = 0; row < poles.size(); ++row) {
        rows[row]->SetValue(section + 1, poles[row]);
      }
      parameters->SetValue(section + 1, fraction);
    }
    // Every row of control points becomes a cubic through its sections at the
    // same parameters, so all rows share one knot vector along the seam.
    std::vector<occ::handle<Geom_BSplineCurve>> curves;
    for (const auto& row : rows) {
      GeomAPI_Interpolate interpolation(row, parameters, false, tolerances().linear_mm);
      interpolation.Perform();
      if (!interpolation.IsDone()) {
        return error_result(STATUS_INVALID_SHAPE, "The blend's sections do not interpolate");
      }
      curves.push_back(interpolation.Curve());
    }
    const occ::handle<Geom_BSplineCurve>& model = curves.front();
    for (const auto& curve : curves) {
      if (curve->NbPoles() != model->NbPoles() || curve->NbKnots() != model->NbKnots()
          || curve->Degree() != model->Degree()) {
        return error_result(STATUS_INVALID_SHAPE, "The blend's sections interpolate unevenly");
      }
    }
    TColgp_Array2OfPnt poles(1, 6, 1, model->NbPoles());
    for (int row = 0; row < 6; ++row) {
      for (int pole = 1; pole <= model->NbPoles(); ++pole) {
        poles(row + 1, pole) = curves[row]->Pole(pole);
      }
    }
    TColStd_Array1OfReal across_knots(1, 2);
    across_knots(1) = 0.0;
    across_knots(2) = 1.0;
    TColStd_Array1OfInteger across_multiplicities(1, 2);
    across_multiplicities(1) = 6;
    across_multiplicities(2) = 6;
    const occ::handle<Geom_BSplineSurface> surface = new Geom_BSplineSurface(
        poles, across_knots, model->Knots(), across_multiplicities, model->Multiplicities(), 5,
        model->Degree());
    BRepBuilderAPI_MakeFace face_builder(surface, tolerances().linear_mm);
    if (!face_builder.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "The blend surface does not make a face");
    }
    const TopoDS_Shape result = face_builder.Face();
    if (!BRepCheck_Analyzer(result).IsValid()) {
      return error_result(STATUS_INVALID_SHAPE, "Surface blend is not one valid bounded face");
    }
    // Every claimed seam is measured along its length.
    const TopoDS_Face blend = TopoDS::Face(result);
    for (const auto& [support, order, name] : {
             std::tuple{first_face, first_order, "first"},
             std::tuple{second_face, second_order, "second"},
         }) {
      const SeamMeasure seam = measure_seam(blend, support);
      std::ostringstream diagnostic;
      diagnostic << "Surface blend at the " << name << " surface: ";
      if (!seam.found || !seam.failure.empty()) {
        diagnostic << (seam.failure.empty() ? "no seam" : seam.failure);
        return error_result(STATUS_INVALID_SHAPE, diagnostic.str());
      }
      const double allowed_curvature = tolerances().surface_blend_curvature_share
          * std::max(std::abs(seam.second_curvature), 1.0 / seam.length);
      if (seam.gap > tolerances().network_surface_approximation_mm) {
        diagnostic << "gap of " << seam.gap << " mm, more than "
                   << tolerances().network_surface_approximation_mm;
      } else if (order >= 1 && seam.angle > tolerances().surface_blend_angle_rad) {
        diagnostic << "normals differ by " << seam.angle << " rad, more than "
                   << tolerances().surface_blend_angle_rad;
      } else if (order == 2 && seam.curvature_difference > allowed_curvature) {
        diagnostic << "curvature differs by " << seam.curvature_difference
                   << " per mm, more than " << allowed_curvature;
      } else {
        continue;
      }
      return error_result(STATUS_INVALID_SHAPE, diagnostic.str());
    }
    const ShapeIndex result_index(result);
    std::vector<HistoryRecord> history;
    history.push_back(history_record(
        "surface_blend.face", "blended", "surface.pair", result_index, result));
    return success_result(result, std::move(history), false, false, {}, true);
  });
}

NativeSeam measure_seam_native(
    const NativeOperationResult& first, const NativeOperationResult& second) noexcept {
  NativeSeam result{};
  result.status = STATUS_INVALID_SHAPE;
  try {
    const TopoDS_Face first_face = single_surface_face(first);
    const TopoDS_Face second_face = single_surface_face(second);
    if (first_face.IsNull() || second_face.IsNull()) {
      result.status = STATUS_INVALID_PARAMETER;
      result.diagnostic = "Seam measurement takes two single-face surfaces";
      return result;
    }
    const SeamMeasure seam = measure_seam(first_face, second_face);
    if (!seam.found) {
      result.status = STATUS_INVALID_PARAMETER;
      result.diagnostic = seam.failure;
      return result;
    }
    if (!seam.failure.empty()) {
      result.diagnostic = seam.failure;
      return result;
    }
    result.length_mm = seam.length;
    result.gap_mm = seam.gap;
    result.normal_angle_rad = seam.angle;
    result.curvature_difference_per_mm = seam.curvature_difference;
    result.second_curvature_per_mm = seam.second_curvature;
    result.status = STATUS_OK;
  } catch (const Standard_Failure& error) {
    result.status = STATUS_BACKEND_EXCEPTION;
    result.diagnostic = standard_failure_message(error);
  } catch (const std::exception& error) {
    result.status = STATUS_BACKEND_EXCEPTION;
    result.diagnostic = error.what();
  }
  return result;
}

}  // namespace ketchup::exact
