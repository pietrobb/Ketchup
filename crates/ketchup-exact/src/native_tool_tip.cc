// The swept volume of an axial cutter whose end is not flat: the cone of a drill's
// point or the half sphere of a ball end mill. The tool's lowest point follows the
// motion, as the toolpath gives it. Each slice of the tool at a height is a disc,
// so a straight move sweeps the tool's cross-section through its axis along the
// move, and a circular move revolves it about the arc's axis; the tool standing
// at both ends closes the sweep.
#include "native_common.hxx"

#include <BRepBuilderAPI_MakePolygon.hxx>
#include <BRepPrimAPI_MakeCone.hxx>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <BRepPrimAPI_MakeSphere.hxx>

namespace ketchup::exact {
namespace {

constexpr std::uint8_t TIP_CONE = 1;
constexpr std::uint8_t TIP_BALL = 2;

struct TippedTool {
  double radius;
  // From the lowest point up to the end of the cutter.
  double length;
  std::uint8_t kind;
  // Axial length of the cone or half sphere.
  double tip;
};

TopoDS_Shape fuse_shapes(const TopoDS_Shape& left, const TopoDS_Shape& right) {
  if (left.IsNull() || right.IsNull()) return TopoDS_Shape{};
  BRepAlgoAPI_Fuse operation;
  configure_boolean(operation, left, right);
  operation.Build();
  if (!operation.IsDone() || operation.HasErrors()) return TopoDS_Shape{};
  operation.SimplifyResult(true, true);
  return operation.Shape();
}

// The tool with its lowest point at `point`, its shank `extra` longer.
TopoDS_Shape tool_at(const TippedTool& tool, const gp_Pnt& point, double extra) {
  const gp_Dir up(0.0, 0.0, 1.0);
  BRepPrimAPI_MakeCylinder shank(
      gp_Ax2(gp_Pnt(point.X(), point.Y(), point.Z() + tool.tip), up),
      tool.radius,
      tool.length - tool.tip + extra);
  shank.Build();
  if (!shank.IsDone()) return TopoDS_Shape{};
  TopoDS_Shape tip;
  if (tool.kind == TIP_CONE) {
    BRepPrimAPI_MakeCone cone(gp_Ax2(point, up), 0.0, tool.radius, tool.tip);
    cone.Build();
    if (!cone.IsDone()) return TopoDS_Shape{};
    tip = cone.Shape();
  } else {
    BRepPrimAPI_MakeSphere sphere(
        gp_Pnt(point.X(), point.Y(), point.Z() + tool.radius), tool.radius);
    sphere.Build();
    if (!sphere.IsDone()) return TopoDS_Shape{};
    tip = sphere.Shape();
  }
  return fuse_shapes(shank.Shape(), tip);
}

TopoDS_Edge line_edge(const gp_Pnt& from, const gp_Pnt& to) {
  BRepBuilderAPI_MakeEdge edge(from, to);
  return edge.IsDone() ? edge.Edge() : TopoDS_Edge{};
}

TopoDS_Edge arc_edge(const gp_Pnt& from, const gp_Pnt& through, const gp_Pnt& to) {
  GC_MakeArcOfCircle arc(from, through, to);
  if (!arc.IsDone()) return TopoDS_Edge{};
  BRepBuilderAPI_MakeEdge edge(arc.Value());
  return edge.IsDone() ? edge.Edge() : TopoDS_Edge{};
}

// A planar face bounded by `edges` in order, or null.
TopoDS_Face face_of(const std::vector<TopoDS_Edge>& edges) {
  BRepBuilderAPI_MakeWire wire;
  for (const TopoDS_Edge& edge : edges) {
    if (edge.IsNull()) return TopoDS_Face{};
    wire.Add(edge);
  }
  if (!wire.IsDone()) return TopoDS_Face{};
  BRepBuilderAPI_MakeFace face(wire.Wire(), true);
  return face.IsDone() ? face.Face() : TopoDS_Face{};
}

// The tool's cross-section in the vertical plane through `origin` spanned by the
// horizontal `side` and Z, as distances (u along `side`, w up) from `origin`; the
// tool's axis stands at u = `axis_u`, and the section keeps to u >= `min_u`.
TopoDS_Face section(
    const TippedTool& tool, const gp_Pnt& origin, const gp_Dir& side, double axis_u,
    double min_u) {
  const auto at = [&](double u, double w) {
    return gp_Pnt(
        origin.X() + u * side.X(), origin.Y() + u * side.Y(), origin.Z() + w);
  };
  const double r = tool.radius;
  const double h = tool.tip;
  const double top = tool.length;
  const double outer = axis_u + r;
  const double inner = std::max(axis_u - r, min_u);
  std::vector<TopoDS_Edge> edges;
  if (tool.kind == TIP_CONE) {
    // Where the cone's inner side meets u = min_u, if before its full width.
    const double inner_w = axis_u - r >= min_u ? h : h * (axis_u - min_u) / r;
    edges.push_back(line_edge(at(axis_u, 0.0), at(outer, h)));
    edges.push_back(line_edge(at(outer, h), at(outer, top)));
    edges.push_back(line_edge(at(outer, top), at(inner, top)));
    edges.push_back(line_edge(at(inner, top), at(inner, inner_w)));
    edges.push_back(line_edge(at(inner, inner_w), at(axis_u, 0.0)));
  } else {
    // The ball's profile is the circle about (axis_u, r), from straight down at
    // angle 0 to its outer side at pi/2; its inner end is where it meets u = min_u.
    const double half_pi = std::acos(0.0);
    const double inner_angle =
        axis_u - r >= min_u ? -half_pi : -std::asin((axis_u - min_u) / r);
    const auto on_ball = [&](double angle) {
      return at(axis_u + r * std::sin(angle), r - r * std::cos(angle));
    };
    const gp_Pnt start = on_ball(inner_angle);
    edges.push_back(arc_edge(start, on_ball((inner_angle + half_pi) * 0.5), at(outer, r)));
    edges.push_back(line_edge(at(outer, r), at(outer, top)));
    edges.push_back(line_edge(at(outer, top), at(inner, top)));
    edges.push_back(line_edge(at(inner, top), start));
  }
  return face_of(edges);
}

}  // namespace

std::unique_ptr<NativeOperationResult> sweep_tipped_axial_tool_native(
    const NativeSegment& motion, double radius, double axial_length, std::uint8_t tip_kind,
    double tip_length) noexcept {
  return guarded([&] {
    const bool tip_fits = tip_kind == TIP_BALL
        ? std::abs(tip_length - radius) <= tolerances().rounding
        : tip_kind == TIP_CONE && tip_length > tolerances().rounding;
    if (!segment_bounded(motion, std::numeric_limits<double>::max())
        || !std::isfinite(radius) || !std::isfinite(axial_length)
        || !std::isfinite(tip_length)
        || (motion.kind != NativeSegmentKind::Line
            && motion.kind != NativeSegmentKind::CircularArc)
        || radius <= 0.0 || !tip_fits
        || axial_length <= tip_length + tolerances().rounding) {
      return error_result(STATUS_INVALID_PARAMETER, "Tipped axial tool sweep payload is malformed");
    }
    const TippedTool tool{radius, axial_length, tip_kind, tip_kind == TIP_BALL ? radius : tip_length};
    const gp_Pnt start = spatial_point(motion.start);
    const gp_Pnt end = spatial_point(motion.end);
    TopoDS_Shape result;
    if (motion.kind == NativeSegmentKind::Line) {
      const double dx = end.X() - start.X();
      const double dy = end.Y() - start.Y();
      const double dz = end.Z() - start.Z();
      const double planar_length = std::hypot(dx, dy);
      if (planar_length <= tolerances().rounding) {
        const gp_Pnt bottom(start.X(), start.Y(), std::min(start.Z(), end.Z()));
        result = tool_at(tool, bottom, std::abs(dz));
      } else {
        if (std::abs(dz) > tolerances().rounding) {
          return error_result(
              STATUS_INVALID_PARAMETER,
              "Axial tool line sweep must be horizontal or vertical");
        }
        const gp_Dir along(dx, dy, 0.0);
        const gp_Dir side(-dy, dx, 0.0);
        const TopoDS_Face profile = section(tool, start, side, 0.0, -radius);
        if (profile.IsNull()) {
          return error_result(STATUS_INVALID_SHAPE, "OCCT tipped line sweep section failed");
        }
        BRepPrimAPI_MakePrism prism(profile, gp_Vec(along) * planar_length);
        prism.Build();
        if (!prism.IsDone()) {
          return error_result(STATUS_INVALID_SHAPE, "OCCT tipped line sweep prism failed");
        }
        result = fuse_shapes(
            fuse_shapes(prism.Shape(), tool_at(tool, start, 0.0)), tool_at(tool, end, 0.0));
      }
    } else {
      const gp_Pnt center = spatial_point(motion.center);
      if (std::abs(start.Z() - end.Z()) > tolerances().rounding
          || std::abs(start.Z() - center.Z()) > tolerances().rounding) {
        return error_result(STATUS_INVALID_PARAMETER, "Axial tool arc sweep is malformed");
      }
      const double centerline_radius = std::hypot(start.X() - center.X(), start.Y() - center.Y());
      const double end_radius = std::hypot(end.X() - center.X(), end.Y() - center.Y());
      if (centerline_radius <= tolerances().rounding
          || std::abs(centerline_radius - end_radius) > tolerances().linear_mm
          || start.Distance(end) <= tolerances().rounding) {
        return error_result(STATUS_DEGENERATE_OPERATION, "Axial tool arc sweep is degenerate");
      }
      const double tau = 2.0 * std::acos(-1.0);
      const double start_angle = std::atan2(start.Y() - center.Y(), start.X() - center.X());
      const double end_angle = std::atan2(end.Y() - center.Y(), end.X() - center.X());
      double sweep = end_angle - start_angle;
      if (motion.clockwise) {
        while (sweep >= 0.0) sweep -= tau;
      } else {
        while (sweep <= 0.0) sweep += tau;
      }
      const gp_Pnt axis_point(center.X(), center.Y(), start.Z());
      const gp_Dir radial(std::cos(start_angle), std::sin(start_angle), 0.0);
      // Inside the arc's axis the section would revolve onto itself; the tool
      // standing at the ends covers what lies there.
      const TopoDS_Face profile = section(tool, axis_point, radial, centerline_radius, 0.0);
      if (profile.IsNull()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT tipped arc sweep section failed");
      }
      const gp_Ax1 axis(axis_point, gp_Dir(0.0, 0.0, sweep < 0.0 ? -1.0 : 1.0));
      BRepPrimAPI_MakeRevol revolution(profile, axis, std::abs(sweep));
      revolution.Build();
      if (!revolution.IsDone()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT tipped arc sweep revolution failed");
      }
      result = fuse_shapes(
          fuse_shapes(revolution.Shape(), tool_at(tool, start, 0.0)), tool_at(tool, end, 0.0));
    }
    if (result.IsNull() || !BRepCheck_Analyzer(result, true).IsValid()
        || count_subshapes(result, TopAbs_SOLID) != 1) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT tipped axial tool sweep is not one valid solid");
    }
    return success_result(result, {});
  });
}

}  // namespace ketchup::exact
