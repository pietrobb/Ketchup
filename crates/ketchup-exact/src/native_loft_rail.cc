// A Loft stretched to a rail: the Loft along the guide alone is cut into
// stations across the guide, each station is scaled about the guide until its
// border meets the rail where the rail crosses the station's plane, and the
// result is lofted through the scaled stations. The stations are isolines of
// one surface, so they share their parametrization and start point.
#include "native_common.hxx"

#include <BRepLib_FindSurface.hxx>
#include <BRepTools_WireExplorer.hxx>
#include <GeomAPI_IntCS.hxx>
#include <GeomConvert.hxx>
#include <GeomConvert_CompCurveToBSplineCurve.hxx>
#include <Geom_BSplineCurve.hxx>

namespace ketchup::exact {

namespace {

constexpr std::size_t FIRST_STATION_COUNT = 16;
constexpr std::size_t MAX_STATION_COUNT = 64;
constexpr std::size_t RAIL_SAMPLES_PER_STATION = 4;
constexpr Standard_Integer MAX_DEGREE_ALONG_GUIDE = 5;
// A rail may scale a station by at most this factor up or down.
constexpr double MAX_STRETCH = 1000.0;
constexpr double MIN_STRETCH = 1.0 / MAX_STRETCH;

double fit_tolerance() { return tolerances().rail_loft_approximation_mm; }

// The edges of a wire in order, each as a curve running along the wire.
std::vector<occ::handle<Geom_Curve>> wire_curves(const TopoDS_Wire& wire) {
  std::vector<occ::handle<Geom_Curve>> curves;
  for (BRepTools_WireExplorer edges(wire); edges.More(); edges.Next()) {
    double start = 0.0;
    double end = 0.0;
    const occ::handle<Geom_Curve> curve = BRep_Tool::Curve(edges.Current(), start, end);
    if (curve.IsNull()) return {};
    occ::handle<Geom_TrimmedCurve> piece = new Geom_TrimmedCurve(curve, start, end);
    if (edges.Current().Orientation() == TopAbs_REVERSED) piece->Reverse();
    curves.push_back(piece);
  }
  return curves;
}

// The guide joined into one curve, so the Loft along it has one face per
// section edge and the faces' second parameter runs along the guide.
occ::handle<Geom_BSplineCurve> joined_guide(const TopoDS_Wire& guide) {
  GeomConvert_CompCurveToBSplineCurve joined;
  for (const occ::handle<Geom_Curve>& curve : wire_curves(guide)) {
    if (!joined.Add(GeomConvert::CurveToBSplineCurve(curve), tolerances().linear_mm, true)) {
      return nullptr;
    }
  }
  return joined.BSplineCurve();
}

// Where a curve crosses a plane; empty with `touches` set when it runs in it.
std::vector<gp_Pnt> plane_crossings(
    const occ::handle<Geom_Curve>& curve, const gp_Pln& plane, bool& touches) {
  GeomAPI_IntCS crossing(curve, new Geom_Plane(plane));
  std::vector<gp_Pnt> points;
  if (!crossing.IsDone()) {
    touches = true;
    return points;
  }
  touches = touches || crossing.NbSegments() > 0;
  for (Standard_Integer index = 1; index <= crossing.NbPoints(); ++index) {
    const gp_Pnt point = crossing.Point(index);
    const bool repeated = std::any_of(points.begin(), points.end(), [&](const gp_Pnt& other) {
      return other.Distance(point) <= fit_tolerance();
    });
    if (!repeated) points.push_back(point);
  }
  return points;
}

// One side face of the Loft along the guide.
struct Side {
  occ::handle<Geom_Surface> surface;
  double first_u = 0.0;
  double last_u = 0.0;
};

// The station at `v`: the sides' isolines in wire order, each a curve of its
// own that the caller may change.
std::vector<occ::handle<Geom_BSplineCurve>> isolines(
    const std::vector<Side>& sides, const std::vector<std::pair<std::size_t, bool>>& order,
    double v) {
  std::vector<occ::handle<Geom_BSplineCurve>> curves;
  for (const auto& [index, reversed] : order) {
    const Side& side = sides[index];
    occ::handle<Geom_BSplineCurve> curve = GeomConvert::CurveToBSplineCurve(
        new Geom_TrimmedCurve(side.surface->VIso(v), side.first_u, side.last_u));
    if (curve->IsPeriodic()) curve->SetNotPeriodic();
    if (reversed) curve->Reverse();
    curves.push_back(curve);
  }
  return curves;
}

// The order in which the sides' isolines run around a closed section, each
// with whether it runs backwards; empty when they do not close.
std::vector<std::pair<std::size_t, bool>> side_order(const std::vector<Side>& sides, double v) {
  std::vector<std::pair<std::size_t, bool>> identity;
  for (std::size_t index = 0; index < sides.size(); ++index) identity.emplace_back(index, false);
  const auto curves = isolines(sides, identity, v);
  std::vector<std::pair<std::size_t, bool>> order{{0, false}};
  std::vector<bool> used(sides.size(), false);
  used[0] = true;
  gp_Pnt end = curves[0]->EndPoint();
  while (order.size() < sides.size()) {
    bool found = false;
    for (std::size_t index = 0; index < curves.size() && !found; ++index) {
      if (used[index]) continue;
      for (const bool reversed : {false, true}) {
        const gp_Pnt start = reversed ? curves[index]->EndPoint() : curves[index]->StartPoint();
        if (start.Distance(end) <= fit_tolerance()) {
          order.emplace_back(index, reversed);
          used[index] = true;
          end = reversed ? curves[index]->StartPoint() : curves[index]->EndPoint();
          found = true;
          break;
        }
      }
    }
    if (!found) return {};
  }
  if (end.Distance(curves[0]->StartPoint()) > fit_tolerance()) return {};
  return order;
}

// A closed wire of curves whose ends meet within the fit tolerance: each
// shared end is moved to the middle of the two, so the edges share vertices.
std::optional<TopoDS_Wire> closed_wire(std::vector<occ::handle<Geom_BSplineCurve>>& curves) {
  for (std::size_t index = 0; index < curves.size(); ++index) {
    occ::handle<Geom_BSplineCurve>& current = curves[index];
    occ::handle<Geom_BSplineCurve>& next = curves[(index + 1) % curves.size()];
    const gp_Pnt end = current->EndPoint();
    const gp_Pnt start = next->StartPoint();
    if (end.Distance(start) > fit_tolerance()) return std::nullopt;
    const gp_Pnt middle((end.XYZ() + start.XYZ()) / 2.0);
    current->SetPole(current->NbPoles(), middle);
    next->SetPole(1, middle);
  }
  BRepBuilderAPI_MakeWire builder;
  for (const occ::handle<Geom_BSplineCurve>& curve : curves) {
    BRepBuilderAPI_MakeEdge edge(curve);
    if (!edge.IsDone()) return std::nullopt;
    builder.Add(edge.Edge());
    if (!builder.IsDone()) return std::nullopt;
  }
  if (!builder.Wire().Closed()) return std::nullopt;
  return builder.Wire();
}

// The station at `v` scaled about the guide so its border meets the rail
// where the rail crosses the station's plane, or why it cannot be.
const char* stretched_station(
    const std::vector<Side>& sides, const std::vector<std::pair<std::size_t, bool>>& order,
    double v, const occ::handle<Geom_Curve>& guide,
    const std::vector<occ::handle<Geom_Curve>>& rail, TopoDS_Wire& station) {
  auto curves = isolines(sides, order, v);
  const std::optional<TopoDS_Wire> unstretched = closed_wire(curves);
  if (!unstretched) return "OCCT Loft rail station is not a closed section";
  BRepLib_FindSurface plane_finder(*unstretched, fit_tolerance(), true);
  const occ::handle<Geom_Plane> plane_surface =
      plane_finder.Found() ? occ::down_cast<Geom_Plane>(plane_finder.Surface()) : nullptr;
  if (plane_surface.IsNull()) return "OCCT Loft rail station is not planar";
  const gp_Pln plane = plane_surface->Pln();

  bool touches = false;
  std::vector<gp_Pnt> rail_points;
  for (const occ::handle<Geom_Curve>& piece : rail) {
    for (const gp_Pnt& point : plane_crossings(piece, plane, touches)) {
      const bool repeated = std::any_of(rail_points.begin(), rail_points.end(),
                                        [&](const gp_Pnt& other) {
                                          return other.Distance(point) <= fit_tolerance();
                                        });
      if (!repeated) rail_points.push_back(point);
    }
  }
  if (touches || rail_points.size() != 1) {
    return "OCCT Loft rail does not cross every section once";
  }
  const std::vector<gp_Pnt> guide_points = plane_crossings(guide, plane, touches);
  if (touches || guide_points.empty()) return "OCCT Loft guide does not cross a section";
  const gp_Pnt on_guide = guide->Value(v);
  const gp_Pnt center = *std::min_element(
      guide_points.begin(), guide_points.end(), [&](const gp_Pnt& left, const gp_Pnt& right) {
        return left.Distance(on_guide) < right.Distance(on_guide);
      });
  const gp_Pnt on_rail = rail_points.front();
  const double rail_distance = center.Distance(on_rail);
  if (rail_distance <= fit_tolerance()) return "OCCT Loft rail meets the guide";

  // The border where the half-line from the guide towards the rail leaves it.
  Bnd_Box box;
  BRepBndLib::Add(*unstretched, box);
  const double reach = 2.0 * (rail_distance + std::sqrt(box.SquareExtent()));
  const gp_Dir towards_rail(gp_Vec(center, on_rail));
  BRepBuilderAPI_MakeEdge ray(center, center.Translated(gp_Vec(towards_rail) * reach));
  if (!ray.IsDone()) return "OCCT Loft rail direction is invalid";
  BRepExtrema_DistShapeShape border(*unstretched, ray.Edge());
  if (!border.IsDone() || border.Value() > fit_tolerance()) {
    return "OCCT Loft rail lies where no section border reaches";
  }
  double border_distance = 0.0;
  for (Standard_Integer index = 1; index <= border.NbSolution(); ++index) {
    border_distance = std::max(border_distance, center.Distance(border.PointOnShape1(index)));
  }
  const double stretch = rail_distance / border_distance;
  if (!std::isfinite(stretch) || stretch < MIN_STRETCH || stretch > MAX_STRETCH) {
    return "OCCT Loft rail stretches a section out of range";
  }
  gp_Trsf scale;
  scale.SetScale(center, stretch);
  for (occ::handle<Geom_BSplineCurve>& curve : curves) curve->Transform(scale);
  const std::optional<TopoDS_Wire> stretched = closed_wire(curves);
  if (!stretched) return "OCCT Loft rail station is not a closed section";
  station = *stretched;
  return nullptr;
}

// The largest distance from sampled rail points to the faces of `shape`.
double rail_deviation(const std::vector<occ::handle<Geom_Curve>>& rail,
                      const TopoDS_Shape& shape, std::size_t samples_per_piece) {
  TopoDS_Compound faces;
  BRep_Builder builder;
  builder.MakeCompound(faces);
  for (TopExp_Explorer face(shape, TopAbs_FACE); face.More(); face.Next()) {
    builder.Add(faces, face.Current());
  }
  double deviation = 0.0;
  for (const occ::handle<Geom_Curve>& piece : rail) {
    const double first = piece->FirstParameter();
    const double last = piece->LastParameter();
    for (std::size_t sample = 0; sample <= samples_per_piece; ++sample) {
      const double t = first + (last - first) * static_cast<double>(sample) /
                                   static_cast<double>(samples_per_piece);
      BRepExtrema_DistShapeShape distance(BRepBuilderAPI_MakeVertex(piece->Value(t)), faces);
      if (!distance.IsDone()) return std::numeric_limits<double>::infinity();
      deviation = std::max(deviation, distance.Value());
    }
  }
  return deviation;
}

}  // namespace

std::unique_ptr<NativeOperationResult> loft_stretched_to_rail(
    const TopoDS_Wire& guide, const TopoDS_Wire& rail, const std::vector<TopoDS_Wire>& wires,
    std::uint8_t continuity, bool make_solid) {
  const occ::handle<Geom_BSplineCurve> guide_curve = joined_guide(guide);
  if (guide_curve.IsNull()) {
    return error_result(STATUS_INVALID_PARAMETER, "OCCT Loft guide cannot be joined into one curve");
  }
  const std::vector<occ::handle<Geom_Curve>> rail_curves = wire_curves(rail);
  if (rail_curves.empty()) {
    return error_result(STATUS_INVALID_PARAMETER, "OCCT Loft rail has no curve");
  }
  BRepOffsetAPI_MakePipeShell along_guide(
      BRepBuilderAPI_MakeWire(BRepBuilderAPI_MakeEdge(guide_curve).Edge()).Wire());
  along_guide.SetMode(false);
  along_guide.SetTolerance(tolerances().linear_mm, tolerances().linear_mm, tolerances().rounding);
  along_guide.SetForceApproxC1(continuity == 1);
  for (const TopoDS_Wire& wire : wires) along_guide.Add(wire, false, false);
  if (!along_guide.IsReady()) {
    return error_result(STATUS_INVALID_SHAPE, "OCCT Loft along the guide is not ready");
  }
  along_guide.Build();
  if (!along_guide.IsDone()) {
    return error_result(STATUS_INVALID_SHAPE, "OCCT Loft along the guide did not complete");
  }

  std::vector<Side> sides;
  double first_v = 0.0;
  double last_v = 0.0;
  for (TopExp_Explorer face(along_guide.Shape(), TopAbs_FACE); face.More(); face.Next()) {
    Side side;
    double v_start = 0.0;
    double v_end = 0.0;
    BRepTools::UVBounds(TopoDS::Face(face.Current()), side.first_u, side.last_u, v_start, v_end);
    side.surface = BRep_Tool::Surface(TopoDS::Face(face.Current()));
    if (sides.empty()) {
      first_v = v_start;
      last_v = v_end;
    } else if (std::abs(v_start - first_v) > tolerances().rounding * std::max(1.0, std::abs(first_v))
               || std::abs(v_end - last_v) > tolerances().rounding * std::max(1.0, std::abs(last_v))) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT Loft along the guide has uneven sides");
    }
    sides.push_back(side);
  }
  if (sides.empty() || !(last_v > first_v)) {
    return error_result(STATUS_INVALID_SHAPE, "OCCT Loft along the guide has no sides");
  }
  const auto order = side_order(sides, first_v);
  if (order.empty()) {
    return error_result(STATUS_INVALID_SHAPE, "OCCT Loft along the guide has an open section");
  }

  for (std::size_t station_count = FIRST_STATION_COUNT; station_count <= MAX_STATION_COUNT;
       station_count *= 2) {
    BRepOffsetAPI_ThruSections operation(make_solid, false, fit_tolerance());
    operation.CheckCompatibility(false);
    operation.SetSmoothing(false);
    operation.SetMutableInput(false);
    operation.SetContinuity(continuity == 1 ? GeomAbs_C1 : GeomAbs_C2);
    // A low degree along the guide keeps the surface's extremes, and so its
    // bounds, where the kernel's bounding search finds them.
    operation.SetMaxDegree(MAX_DEGREE_ALONG_GUIDE);
    for (std::size_t index = 0; index <= station_count; ++index) {
      const double v = first_v + (last_v - first_v) * static_cast<double>(index) /
                                     static_cast<double>(station_count);
      TopoDS_Wire station;
      if (const char* problem =
              stretched_station(sides, order, v, guide_curve, rail_curves, station)) {
        return error_result(STATUS_INVALID_PARAMETER, problem);
      }
      operation.AddWire(station);
    }
    operation.Build();
    if (!operation.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT Loft through the rail stations did not complete");
    }
    const TopoDS_Shape result = operation.Shape();
    const std::uint32_t solid_count = count_subshapes(result, TopAbs_SOLID);
    if (result.IsNull() || !BRepCheck_Analyzer(result).IsValid()
        || (make_solid ? solid_count != 1 : solid_count != 0)) {
      return error_result(
          STATUS_INVALID_SHAPE,
          make_solid ? "OCCT Loft stretched to the rail is not one valid solid"
                     : "OCCT Loft stretched to the rail is not a valid open surface");
    }
    if (rail_deviation(rail_curves, result, station_count * RAIL_SAMPLES_PER_STATION)
        > fit_tolerance()) {
      continue;
    }
    const ShapeIndex result_index(result);
    std::vector<HistoryRecord> history;
    history.push_back(history_record(
        "loft.start", "first_shape", "profile.wire", result_index, operation.FirstShape()));
    history.push_back(history_record(
        "loft.end", "last_shape", "profile.wire", result_index, operation.LastShape()));
    return success_result(result, std::move(history), false, false, {}, !make_solid);
  }
  return error_result(STATUS_INVALID_SHAPE, "OCCT Loft does not follow its rail closely enough");
}

}  // namespace ketchup::exact
