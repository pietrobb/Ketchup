#include "native_common.hxx"

namespace ketchup::exact {

std::unique_ptr<NativeOperationResult> extrude_circle_native(
    double center_x, double center_y, double radius, double height) noexcept {
  return guarded([&] {
    BRepPrimAPI_MakeCylinder operation(
        gp_Ax2(gp_Pnt(center_x, center_y, 0.0), gp_Dir(0.0, 0.0, 1.0)),
        radius,
        height);
    operation.Build();
    if (!operation.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT cylinder builder did not complete");
    }
    const TopoDS_Shape result = operation.Shape();
    TopoDS_Face bottom;
    TopoDS_Face top;
    TopoDS_Face side;
    for (TopExp_Explorer explorer(result, TopAbs_FACE); explorer.More(); explorer.Next()) {
      const TopoDS_Face candidate = TopoDS::Face(explorer.Current());
      BRepAdaptor_Surface surface(candidate);
      if (surface.GetType() == GeomAbs_Cylinder) {
        if (!side.IsNull()) {
          return error_result(STATUS_INVALID_SHAPE, "OCCT cylinder has ambiguous side identity");
        }
        side = candidate;
        continue;
      }
      BRepBuilderAPI_FindPlane plane(candidate);
      if (!plane.Found()) {
        continue;
      }
      GProp_GProps properties;
      BRepGProp::SurfaceProperties(candidate, properties);
      if (std::abs(properties.CentreOfMass().Z()) <= tolerances().rounding) {
        bottom = candidate;
      } else if (std::abs(properties.CentreOfMass().Z() - height) <= tolerances().rounding) {
        top = candidate;
      }
    }
    if (bottom.IsNull() || top.IsNull() || side.IsNull()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT cylinder face identity is incomplete");
    }
    std::vector<HistoryRecord> history;
    history.push_back(history_record(
        "extrusion.bottom", "analytic_cap", "profile.face", result, bottom));
    history.push_back(history_record(
        "extrusion.top", "analytic_cap", "profile.face", result, top));
    history.push_back(history_record(
        "extrusion.side(profile_edge=circle)",
        "analytic_generated",
        "profile.edge.circle",
        result,
        side));
    return success_result(result, std::move(history));
  });
}

std::unique_ptr<NativeOperationResult> sweep_axial_tool_native(
    const NativeSegment& motion, double radius, double axial_length) noexcept {
  return guarded([&] {
    if (!segment_bounded(motion, std::numeric_limits<double>::max())
        || !std::isfinite(radius) || !std::isfinite(axial_length)
        || (motion.kind != NativeSegmentKind::Line
            && motion.kind != NativeSegmentKind::CircularArc)
        || radius <= 0.0 || axial_length <= 0.0) {
      return error_result(STATUS_INVALID_PARAMETER, "Axial tool sweep payload is malformed");
    }
    const gp_Pnt start = spatial_point(motion.start);
    const gp_Pnt end = spatial_point(motion.end);
    const double length = axial_length;
    const auto cylinder = [&](const gp_Pnt& point) {
      BRepPrimAPI_MakeCylinder builder(
          gp_Ax2(point, gp_Dir(0.0, 0.0, 1.0)), radius, length);
      builder.Build();
      return builder.IsDone() ? builder.Shape() : TopoDS_Shape{};
    };
    const auto fuse = [](const TopoDS_Shape& left, const TopoDS_Shape& right) {
      if (left.IsNull() || right.IsNull()) return TopoDS_Shape{};
      BRepAlgoAPI_Fuse operation(left, right);
      operation.Build();
      if (!operation.IsDone() || operation.HasErrors()) return TopoDS_Shape{};
      operation.SimplifyResult(true, true);
      return operation.Shape();
    };

    TopoDS_Shape result;
    if (motion.kind == NativeSegmentKind::Line) {
      const double dx = end.X() - start.X();
      const double dy = end.Y() - start.Y();
      const double dz = end.Z() - start.Z();
      const double planar_length = std::hypot(dx, dy);
      if (planar_length <= tolerances().rounding) {
        const gp_Pnt bottom(start.X(), start.Y(), std::min(start.Z(), end.Z()));
        result = cylinder(bottom);
        if (std::abs(dz) > tolerances().rounding) {
          BRepPrimAPI_MakeCylinder builder(
              gp_Ax2(bottom, gp_Dir(0.0, 0.0, 1.0)),
              radius,
              length + std::abs(dz));
          builder.Build();
          result = builder.IsDone() ? builder.Shape() : TopoDS_Shape{};
        }
      } else {
        if (std::abs(dz) > tolerances().rounding) {
          return error_result(
              STATUS_INVALID_PARAMETER,
              "Axial tool line sweep must be horizontal or vertical");
        }
        const gp_Dir along(dx, dy, 0.0);
        const gp_Dir perpendicular(-dy, dx, 0.0);
        BRepPrimAPI_MakeBox bridge(
            gp_Ax2(
                gp_Pnt(
                    start.X() - radius * perpendicular.X(),
                    start.Y() - radius * perpendicular.Y(),
                    start.Z()),
                gp_Dir(0.0, 0.0, 1.0),
                along),
            planar_length,
            2.0 * radius,
            length);
        bridge.Build();
        if (!bridge.IsDone()) {
          return error_result(STATUS_INVALID_SHAPE, "OCCT axial line sweep bridge failed");
        }
        result = fuse(fuse(bridge.Shape(), cylinder(start)), cylinder(end));
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
      const bool clockwise = motion.clockwise;
      const double tau = 2.0 * std::acos(-1.0);
      const double start_angle = std::atan2(start.Y() - center.Y(), start.X() - center.X());
      const double end_angle = std::atan2(end.Y() - center.Y(), end.X() - center.X());
      double sweep = end_angle - start_angle;
      if (clockwise) {
        while (sweep >= 0.0) sweep -= tau;
      } else {
        while (sweep <= 0.0) sweep += tau;
      }
      const auto point = [&](double radial, double angle) {
        return gp_Pnt(
            center.X() + radial * std::cos(angle),
            center.Y() + radial * std::sin(angle),
            start.Z());
      };
      const auto arc = [&](double radial, double angle, double arc_sweep) {
        GC_MakeArcOfCircle builder(
            point(radial, angle),
            point(radial, angle + arc_sweep * 0.5),
            point(radial, angle + arc_sweep));
        if (!builder.IsDone()) return TopoDS_Edge{};
        BRepBuilderAPI_MakeEdge edge(builder.Value());
        return edge.IsDone() ? edge.Edge() : TopoDS_Edge{};
      };
      const double outer_radius = centerline_radius + radius;
      const double inner_radius = centerline_radius - radius;
      const TopoDS_Edge outer = arc(outer_radius, start_angle, sweep);
      BRepBuilderAPI_MakeWire wire;
      wire.Add(outer);
      if (inner_radius > tolerances().rounding) {
        const TopoDS_Edge end_cap = BRepBuilderAPI_MakeEdge(
            point(outer_radius, start_angle + sweep),
            point(inner_radius, start_angle + sweep)).Edge();
        const TopoDS_Edge inner = arc(inner_radius, start_angle + sweep, -sweep);
        const TopoDS_Edge start_cap = BRepBuilderAPI_MakeEdge(
            point(inner_radius, start_angle),
            point(outer_radius, start_angle)).Edge();
        if (outer.IsNull() || end_cap.IsNull() || inner.IsNull() || start_cap.IsNull()) {
          return error_result(STATUS_INVALID_SHAPE, "OCCT axial arc sweep boundary failed");
        }
        wire.Add(end_cap);
        wire.Add(inner);
        wire.Add(start_cap);
      } else {
        const gp_Pnt arc_center(center.X(), center.Y(), start.Z());
        const TopoDS_Edge end_cap = BRepBuilderAPI_MakeEdge(
            point(outer_radius, start_angle + sweep), arc_center).Edge();
        const TopoDS_Edge start_cap = BRepBuilderAPI_MakeEdge(
            arc_center, point(outer_radius, start_angle)).Edge();
        if (outer.IsNull() || end_cap.IsNull() || start_cap.IsNull()) {
          return error_result(STATUS_INVALID_SHAPE, "OCCT axial broad arc sweep boundary failed");
        }
        wire.Add(end_cap);
        wire.Add(start_cap);
      }
      if (!wire.IsDone()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT axial arc sweep wire failed");
      }
      BRepBuilderAPI_MakeFace face(wire.Wire());
      if (!face.IsDone()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT axial arc sweep face failed");
      }
      BRepPrimAPI_MakePrism prism(face.Face(), gp_Vec(0.0, 0.0, length));
      prism.Build();
      if (!prism.IsDone()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT axial arc sweep prism failed");
      }
      result = fuse(fuse(prism.Shape(), cylinder(start)), cylinder(end));
    }
    if (result.IsNull() || !BRepCheck_Analyzer(result, true).IsValid()
        || count_subshapes(result, TopAbs_SOLID) != 1) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT axial tool sweep is not one valid solid");
    }
    return success_result(result, {});
  });
}

std::unique_ptr<NativeOperationResult> extrude_mixed_profile_native(
    rust::Slice<const NativeSegment> segments, double height) noexcept {
  return guarded([&] {
    if (segments.size() < 2) {
      return error_result(STATUS_INVALID_PARAMETER, "Mixed profile segment payload is malformed");
    }
    BRepBuilderAPI_MakeWire wire_builder;
    std::vector<TopoDS_Edge> profile_edges;
    profile_edges.reserve(segments.size());
    std::size_t first_arc_index = profile_edges.capacity();
    std::size_t first_line_index = profile_edges.capacity();
    std::size_t first_cubic_index = profile_edges.capacity();
    gp_Pnt first_arc_middle;
    for (const NativeSegment& segment : segments) {
      const gp_Pnt start = planar_point(segment.start, 0.0);
      const gp_Pnt end = planar_point(segment.end, 0.0);
      TopoDS_Edge edge;
      if (segment.kind == NativeSegmentKind::Line) {
        edge = BRepBuilderAPI_MakeEdge(start, end).Edge();
        if (first_line_index == profile_edges.capacity()) {
          first_line_index = profile_edges.size();
        }
      } else if (segment.kind == NativeSegmentKind::CircularArc) {
        const double center_x = segment.center.x;
        const double center_y = segment.center.y;
        const bool clockwise = segment.clockwise;
        const double start_angle = std::atan2(start.Y() - center_y, start.X() - center_x);
        const double end_angle = std::atan2(end.Y() - center_y, end.X() - center_x);
        double sweep = end_angle - start_angle;
        const double tau = 2.0 * std::acos(-1.0);
        if (clockwise) {
          while (sweep >= 0.0) sweep -= tau;
        } else {
          while (sweep <= 0.0) sweep += tau;
        }
        const double radius = start.Distance(gp_Pnt(center_x, center_y, 0.0));
        const double middle_angle = start_angle + sweep / 2.0;
        const gp_Pnt middle(
            center_x + radius * std::cos(middle_angle),
            center_y + radius * std::sin(middle_angle),
            0.0);
        GC_MakeArcOfCircle arc_builder(start, middle, end);
        if (!arc_builder.IsDone()) {
          return error_result(STATUS_INVALID_SHAPE, "OCCT mixed profile arc builder did not complete");
        }
        edge = BRepBuilderAPI_MakeEdge(arc_builder.Value()).Edge();
        if (first_arc_index == profile_edges.capacity()) {
          first_arc_index = profile_edges.size();
          first_arc_middle = middle;
        }
      } else if (segment.kind == NativeSegmentKind::CubicBezier) {
        edge = cubic_bezier_edge(segment, 0.0);
        if (first_cubic_index == profile_edges.capacity()) {
          first_cubic_index = profile_edges.size();
        }
      } else {
        return error_result(STATUS_INVALID_PARAMETER, "Mixed profile segment kind is invalid");
      }
      if (edge.IsNull()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT mixed profile edge is null");
      }
      profile_edges.push_back(edge);
      wire_builder.Add(edge);
    }
    if (!wire_builder.IsDone()
        || (first_arc_index >= profile_edges.size()
            && first_line_index >= profile_edges.size()
            && first_cubic_index >= profile_edges.size())) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT segmented profile wire is incomplete");
    }
    BRepBuilderAPI_MakeFace face_builder(wire_builder.Wire(), true);
    if (!face_builder.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT mixed profile face builder did not complete");
    }
    const TopoDS_Face profile = face_builder.Face();
    const bool reference_is_arc = first_arc_index < profile_edges.size();
    const bool reference_is_line = first_line_index < profile_edges.size();
    const std::size_t reference_index = reference_is_arc
        ? first_arc_index
        : (reference_is_line ? first_line_index : first_cubic_index);
    TopoDS_Edge profile_reference;
    if (!reference_is_arc && !reference_is_line) {
      profile_reference = profile_edges[reference_index];
    } else {
      const gp_Pnt expected_reference_start =
          planar_point(segments[reference_index].start, 0.0);
      const gp_Pnt expected_reference_end = planar_point(segments[reference_index].end, 0.0);
      for (TopExp_Explorer explorer(profile, TopAbs_EDGE); explorer.More(); explorer.Next()) {
        const TopoDS_Edge candidate = TopoDS::Edge(explorer.Current());
        const GeomAbs_CurveType expected_type = reference_is_arc ? GeomAbs_Circle : GeomAbs_Line;
        if (BRepAdaptor_Curve(candidate).GetType() != expected_type) {
          continue;
        }
        TopoDS_Vertex first;
        TopoDS_Vertex last;
        TopExp::Vertices(candidate, first, last);
        if (first.IsNull() || last.IsNull()) {
          continue;
        }
        const gp_Pnt first_point = BRep_Tool::Pnt(first);
        const gp_Pnt last_point = BRep_Tool::Pnt(last);
        const bool endpoints_match =
            (first_point.Distance(expected_reference_start) <= tolerances().rounding
                && last_point.Distance(expected_reference_end) <= tolerances().rounding)
            || (first_point.Distance(expected_reference_end) <= tolerances().rounding
                && last_point.Distance(expected_reference_start) <= tolerances().rounding);
        // Two arcs of one circle share both endpoints (a circle drawn as two
        // halves); the arc's midpoint tells them apart.
        bool middle_matches = true;
        if (endpoints_match && reference_is_arc) {
          const BRepAdaptor_Curve curve(candidate);
          const gp_Pnt candidate_middle =
              curve.Value((curve.FirstParameter() + curve.LastParameter()) / 2.0);
          middle_matches = candidate_middle.Distance(first_arc_middle) <= tolerances().approximation;
        }
        if (endpoints_match && middle_matches) {
          if (!profile_reference.IsNull()) {
            return error_result(STATUS_INVALID_SHAPE, "OCCT segmented profile reference edge is ambiguous");
          }
          profile_reference = candidate;
        }
      }
    }
    if (profile_reference.IsNull()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT segmented profile lost its reference edge");
    }
    BRepPrimAPI_MakePrism operation(profile, gp_Vec(0.0, 0.0, height), true, false);
    if (!operation.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT mixed profile prism builder did not complete");
    }
    const TopoDS_Shape result = operation.Shape();
    std::vector<HistoryRecord> history;
    history.push_back(history_record(
        "extrusion.bottom", "first_shape", "profile.face", result, operation.FirstShape()));
    history.push_back(history_record(
        "extrusion.top", "last_shape", "profile.face", result, operation.LastShape()));
    const std::string side_role = reference_is_arc
        ? "extrusion.side(profile_edge=arc.0)"
        : (reference_is_line
            ? "extrusion.side(profile_edge=line.0)"
            : "extrusion.side(profile_edge=spline.0)");
    const std::string side_source = reference_is_arc
        ? "profile.edge.arc.0"
        : (reference_is_line ? "profile.edge.line.0" : "profile.edge.spline.0");
    HistoryRecord side_history{side_role, "generated", side_source, 0, false};
    const NCollection_List<TopoDS_Shape>& generated = operation.Generated(profile_reference);
    for (NCollection_List<TopoDS_Shape>::Iterator iterator(generated); iterator.More(); iterator.Next()) {
      const HistoryRecord candidate = history_record(
          side_role,
          "generated",
          side_source,
          result,
          iterator.Value());
      if (candidate.output_present) {
        side_history = candidate;
        break;
      }
    }
    history.push_back(std::move(side_history));
    return success_result(result, std::move(history));
  });
}

std::unique_ptr<NativeOperationResult> extrude_planar_region_native(
    rust::Slice<const NativeSegment> segments,
    rust::Slice<const std::uint32_t> loop_segment_counts,
    double height) noexcept {
  return guarded([&] {
    if (loop_segment_counts.size() < 2 || loop_segment_counts.size() > 65
        || segments.empty()
        || !std::isfinite(height) || height <= 0.0) {
      return error_result(STATUS_INVALID_PARAMETER, "Planar region payload is malformed");
    }
    std::size_t declared_segments = 0;
    for (const std::uint32_t count : loop_segment_counts) {
      declared_segments += count;
    }
    if (declared_segments != segments.size() || declared_segments > 4096) {
      return error_result(STATUS_INVALID_PARAMETER, "Planar region segment counts do not match");
    }

    auto build_wire = [&](std::size_t first_segment, std::size_t segment_count) {
      BRepBuilderAPI_MakeWire wire_builder;
      for (std::size_t index = 0; index < segment_count; ++index) {
        const NativeSegment& segment = segments[first_segment + index];
        TopoDS_Edge edge;
        if (segment.kind == NativeSegmentKind::Line) {
          const gp_Pnt start = planar_point(segment.start, 0.0);
          const gp_Pnt end = planar_point(segment.end, 0.0);
          if (start.Distance(end) <= tolerances().linear_mm) {
            return TopoDS_Wire{};
          }
          BRepBuilderAPI_MakeEdge edge_builder(start, end);
          if (!edge_builder.IsDone()) {
            return TopoDS_Wire{};
          }
          edge = edge_builder.Edge();
        } else if (segment.kind == NativeSegmentKind::CircularArc) {
          const gp_Pnt start = planar_point(segment.start, 0.0);
          const gp_Pnt end = planar_point(segment.end, 0.0);
          const double center_x = segment.center.x;
          const double center_y = segment.center.y;
          const bool clockwise = segment.clockwise;
          const double start_angle = std::atan2(start.Y() - center_y, start.X() - center_x);
          const double end_angle = std::atan2(end.Y() - center_y, end.X() - center_x);
          double sweep = end_angle - start_angle;
          const double tau = 2.0 * std::acos(-1.0);
          if (clockwise) {
            while (sweep >= 0.0) sweep -= tau;
          } else {
            while (sweep <= 0.0) sweep += tau;
          }
          const double radius = start.Distance(gp_Pnt(center_x, center_y, 0.0));
          const double middle_angle = start_angle + sweep / 2.0;
          const gp_Pnt middle(
              center_x + radius * std::cos(middle_angle),
              center_y + radius * std::sin(middle_angle),
              0.0);
          GC_MakeArcOfCircle arc_builder(start, middle, end);
          if (!arc_builder.IsDone()) {
            return TopoDS_Wire{};
          }
          BRepBuilderAPI_MakeEdge edge_builder(arc_builder.Value());
          if (!edge_builder.IsDone()) {
            return TopoDS_Wire{};
          }
          edge = edge_builder.Edge();
        } else if (segment.kind == NativeSegmentKind::CubicBezier) {
          edge = cubic_bezier_edge(segment, 0.0);
        } else if (segment.kind == NativeSegmentKind::Circle && segment_count == 1) {
          const double center_x = segment.center.x;
          const double center_y = segment.center.y;
          const double radius = segment.radius;
          if (!std::isfinite(center_x) || !std::isfinite(center_y)
              || !std::isfinite(radius) || radius <= tolerances().linear_mm) {
            return TopoDS_Wire{};
          }
          BRepBuilderAPI_MakeEdge edge_builder(
              gp_Circ(gp_Ax2(gp_Pnt(center_x, center_y, 0.0), gp_Dir(0.0, 0.0, 1.0)), radius));
          if (!edge_builder.IsDone()) {
            return TopoDS_Wire{};
          }
          edge = edge_builder.Edge();
          if (segment.clockwise) {
            edge.Reverse();
          }
        } else {
          return TopoDS_Wire{};
        }
        if (edge.IsNull()) {
          return TopoDS_Wire{};
        }
        wire_builder.Add(edge);
      }
      if (!wire_builder.IsDone()) {
        return TopoDS_Wire{};
      }
      return wire_builder.Wire();
    };

    std::size_t first_segment = 0;
    TopoDS_Wire outer = build_wire(first_segment, loop_segment_counts[0]);
    if (outer.IsNull()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT planar region outer wire is invalid");
    }
    first_segment += loop_segment_counts[0];
    BRepBuilderAPI_MakeFace face_builder(outer, true);
    for (std::size_t index = 1; index < loop_segment_counts.size(); ++index) {
      TopoDS_Wire hole = build_wire(first_segment, loop_segment_counts[index]);
      if (hole.IsNull()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT planar region hole wire is invalid");
      }
      face_builder.Add(hole);
      first_segment += loop_segment_counts[index];
    }
    face_builder.Build();
    if (!face_builder.IsDone() || !BRepCheck_Analyzer(face_builder.Face()).IsValid()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT planar region face is invalid");
    }
    const TopoDS_Face profile = face_builder.Face();
    BRepPrimAPI_MakePrism operation(profile, gp_Vec(0.0, 0.0, height), true, false);
    if (!operation.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT planar region prism builder did not complete");
    }
    const TopoDS_Shape result = operation.Shape();
    if (result.IsNull() || !BRepCheck_Analyzer(result).IsValid()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT planar region prism is invalid");
    }
    std::vector<HistoryRecord> history;
    history.push_back(history_record(
        "extrusion.bottom", "first_shape", "profile.face", result, operation.FirstShape()));
    history.push_back(history_record(
        "extrusion.top", "last_shape", "profile.face", result, operation.LastShape()));
    return success_result(result, std::move(history));
  });
}

std::unique_ptr<NativeOperationResult> revolve_planar_region_native(
    rust::Slice<const NativeSegment> segments,
    rust::Slice<const std::uint32_t> loop_segment_counts,
    double axis_start_x, double axis_start_y,
    double axis_end_x, double axis_end_y,
    double angle_degrees) noexcept {
  return guarded([&] {
    if (loop_segment_counts.size() < 2 || loop_segment_counts.size() > 65
        || segments.empty()
        || !std::isfinite(axis_start_x) || !std::isfinite(axis_start_y)
        || !std::isfinite(axis_end_x) || !std::isfinite(axis_end_y)
        || !std::isfinite(angle_degrees) || angle_degrees <= 0.0 || angle_degrees > 360.0) {
      return error_result(STATUS_INVALID_PARAMETER, "Planar region revolve payload is malformed");
    }
    std::size_t declared_segments = 0;
    for (const std::uint32_t count : loop_segment_counts) {
      declared_segments += count;
    }
    if (declared_segments != segments.size() || declared_segments > 4096) {
      return error_result(STATUS_INVALID_PARAMETER, "Planar region revolve segment counts do not match");
    }
    const gp_Vec axis_vector(
        axis_end_x - axis_start_x,
        axis_end_y - axis_start_y,
        0.0);
    if (axis_vector.Magnitude() <= tolerances().negligible) {
      return error_result(STATUS_INVALID_PARAMETER, "Planar region revolve axis is degenerate");
    }

    auto build_wire = [&](std::size_t first_segment, std::size_t segment_count) {
      BRepBuilderAPI_MakeWire wire_builder;
      for (std::size_t index = 0; index < segment_count; ++index) {
        const NativeSegment& segment = segments[first_segment + index];
        TopoDS_Edge edge;
        if (segment.kind == NativeSegmentKind::Line) {
          const gp_Pnt start = planar_point(segment.start, 0.0);
          const gp_Pnt end = planar_point(segment.end, 0.0);
          if (start.Distance(end) <= tolerances().linear_mm) {
            return TopoDS_Wire{};
          }
          BRepBuilderAPI_MakeEdge edge_builder(start, end);
          if (!edge_builder.IsDone()) {
            return TopoDS_Wire{};
          }
          edge = edge_builder.Edge();
        } else if (segment.kind == NativeSegmentKind::CircularArc) {
          const gp_Pnt start = planar_point(segment.start, 0.0);
          const gp_Pnt end = planar_point(segment.end, 0.0);
          const double center_x = segment.center.x;
          const double center_y = segment.center.y;
          const bool clockwise = segment.clockwise;
          const double start_angle = std::atan2(start.Y() - center_y, start.X() - center_x);
          const double end_angle = std::atan2(end.Y() - center_y, end.X() - center_x);
          double sweep = end_angle - start_angle;
          const double tau = 2.0 * std::acos(-1.0);
          if (clockwise) {
            while (sweep >= 0.0) sweep -= tau;
          } else {
            while (sweep <= 0.0) sweep += tau;
          }
          const double radius = start.Distance(gp_Pnt(center_x, center_y, 0.0));
          const gp_Pnt middle(
              center_x + radius * std::cos(start_angle + sweep / 2.0),
              center_y + radius * std::sin(start_angle + sweep / 2.0),
              0.0);
          GC_MakeArcOfCircle arc_builder(start, middle, end);
          if (!arc_builder.IsDone()) {
            return TopoDS_Wire{};
          }
          BRepBuilderAPI_MakeEdge edge_builder(arc_builder.Value());
          if (!edge_builder.IsDone()) {
            return TopoDS_Wire{};
          }
          edge = edge_builder.Edge();
        } else if (segment.kind == NativeSegmentKind::CubicBezier) {
          edge = cubic_bezier_edge(segment, 0.0);
        } else if (segment.kind == NativeSegmentKind::Circle && segment_count == 1) {
          const double center_x = segment.center.x;
          const double center_y = segment.center.y;
          const double radius = segment.radius;
          if (!std::isfinite(center_x) || !std::isfinite(center_y)
              || !std::isfinite(radius) || radius <= tolerances().linear_mm) {
            return TopoDS_Wire{};
          }
          BRepBuilderAPI_MakeEdge edge_builder(
              gp_Circ(gp_Ax2(gp_Pnt(center_x, center_y, 0.0), gp_Dir(0.0, 0.0, 1.0)), radius));
          if (!edge_builder.IsDone()) {
            return TopoDS_Wire{};
          }
          edge = edge_builder.Edge();
          if (segment.clockwise) {
            edge.Reverse();
          }
        } else {
          return TopoDS_Wire{};
        }
        if (edge.IsNull()) {
          return TopoDS_Wire{};
        }
        wire_builder.Add(edge);
      }
      if (!wire_builder.IsDone()) {
        return TopoDS_Wire{};
      }
      return wire_builder.Wire();
    };

    std::size_t first_segment = 0;
    TopoDS_Wire outer = build_wire(first_segment, loop_segment_counts[0]);
    if (outer.IsNull()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT planar region revolve outer wire is invalid");
    }
    first_segment += loop_segment_counts[0];
    BRepBuilderAPI_MakeFace face_builder(outer, true);
    for (std::size_t index = 1; index < loop_segment_counts.size(); ++index) {
      TopoDS_Wire hole = build_wire(first_segment, loop_segment_counts[index]);
      if (hole.IsNull()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT planar region revolve hole wire is invalid");
      }
      face_builder.Add(hole);
      first_segment += loop_segment_counts[index];
    }
    face_builder.Build();
    if (!face_builder.IsDone() || !BRepCheck_Analyzer(face_builder.Face()).IsValid()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT planar region revolve face is invalid");
    }
    const TopoDS_Face profile = face_builder.Face();
    const double angle_radians = angle_degrees * std::acos(-1.0) / 180.0;
    BRepPrimAPI_MakeRevol operation(
        profile,
        gp_Ax1(gp_Pnt(axis_start_x, axis_start_y, 0.0), gp_Dir(axis_vector)),
        angle_radians,
        true);
    if (!operation.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT planar region revolve builder did not complete");
    }
    const TopoDS_Shape result = operation.Shape();
    if (result.IsNull() || !BRepCheck_Analyzer(result).IsValid()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT planar region revolve is invalid");
    }
    std::vector<HistoryRecord> history;
    if (angle_degrees < 360.0) {
      history.push_back(history_record(
          "revolve.start", "first_shape", "profile.face", result, operation.FirstShape()));
      history.push_back(history_record(
          "revolve.end", "last_shape", "profile.face", result, operation.LastShape()));
    }
    return success_result(result, std::move(history));
  });
}

std::unique_ptr<NativeOperationResult> revolve_general_profile_native(
    rust::Slice<const NativeSegment> segments,
    double axis_start_x, double axis_start_y,
    double axis_end_x, double axis_end_y,
    double angle_degrees) noexcept {
  return guarded([&] {
    if (segments.size() < 2
        || !std::isfinite(axis_start_x) || !std::isfinite(axis_start_y)
        || !std::isfinite(axis_end_x) || !std::isfinite(axis_end_y)
        || !std::isfinite(angle_degrees) || angle_degrees <= 0.0 || angle_degrees > 360.0) {
      return error_result(STATUS_INVALID_PARAMETER, "General revolve payload is malformed");
    }
    const gp_Vec axis_vector(
        axis_end_x - axis_start_x,
        axis_end_y - axis_start_y,
        0.0);
    if (axis_vector.Magnitude() <= tolerances().negligible) {
      return error_result(STATUS_INVALID_PARAMETER, "General revolve axis is degenerate");
    }

    BRepBuilderAPI_MakeWire wire_builder;
    std::vector<TopoDS_Edge> profile_edges;
    profile_edges.reserve(segments.size());
    for (const NativeSegment& segment : segments) {
      const gp_Pnt start = planar_point(segment.start, 0.0);
      const gp_Pnt end = planar_point(segment.end, 0.0);
      TopoDS_Edge edge;
      if (segment.kind == NativeSegmentKind::Line) {
        edge = BRepBuilderAPI_MakeEdge(start, end).Edge();
      } else if (segment.kind == NativeSegmentKind::CircularArc) {
        const double center_x = segment.center.x;
        const double center_y = segment.center.y;
        const bool clockwise = segment.clockwise;
        const double start_angle = std::atan2(start.Y() - center_y, start.X() - center_x);
        const double end_angle = std::atan2(end.Y() - center_y, end.X() - center_x);
        double sweep = end_angle - start_angle;
        const double tau = 2.0 * std::acos(-1.0);
        if (clockwise) {
          while (sweep >= 0.0) sweep -= tau;
        } else {
          while (sweep <= 0.0) sweep += tau;
        }
        const double radius = start.Distance(gp_Pnt(center_x, center_y, 0.0));
        const double middle_angle = start_angle + sweep / 2.0;
        const gp_Pnt middle(
            center_x + radius * std::cos(middle_angle),
            center_y + radius * std::sin(middle_angle),
            0.0);
        GC_MakeArcOfCircle arc_builder(start, middle, end);
        if (!arc_builder.IsDone()) {
          return error_result(STATUS_INVALID_SHAPE, "OCCT general revolve arc builder did not complete");
        }
        edge = BRepBuilderAPI_MakeEdge(arc_builder.Value()).Edge();
      } else {
        return error_result(STATUS_INVALID_PARAMETER, "General revolve segment kind is invalid");
      }
      if (edge.IsNull()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT general revolve edge is null");
      }
      profile_edges.push_back(edge);
      wire_builder.Add(edge);
    }
    if (!wire_builder.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT general revolve wire builder did not complete");
    }
    BRepBuilderAPI_MakeFace face_builder(wire_builder.Wire(), true);
    if (!face_builder.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT general revolve profile face builder did not complete");
    }
    const TopoDS_Face profile = face_builder.Face();
    std::vector<TopoDS_Edge> operation_edges;
    operation_edges.reserve(2);
    for (std::size_t source_index = 0; source_index < 2; ++source_index) {
      const NativeSegment& segment = segments[source_index];
      const NativeSegmentKind kind = segment.kind;
      const gp_Pnt expected_start = planar_point(segment.start, 0.0);
      const gp_Pnt expected_end = planar_point(segment.end, 0.0);
      gp_Pnt expected_middle(
          (expected_start.X() + expected_end.X()) / 2.0,
          (expected_start.Y() + expected_end.Y()) / 2.0,
          0.0);
      if (kind == NativeSegmentKind::CircularArc) {
        const double center_x = segment.center.x;
        const double center_y = segment.center.y;
        const bool clockwise = segment.clockwise;
        const double start_angle = std::atan2(
            expected_start.Y() - center_y, expected_start.X() - center_x);
        const double end_angle = std::atan2(
            expected_end.Y() - center_y, expected_end.X() - center_x);
        double sweep = end_angle - start_angle;
        const double tau = 2.0 * std::acos(-1.0);
        if (clockwise) {
          while (sweep >= 0.0) sweep -= tau;
        } else {
          while (sweep <= 0.0) sweep += tau;
        }
        const double radius = expected_start.Distance(gp_Pnt(center_x, center_y, 0.0));
        expected_middle = gp_Pnt(
            center_x + radius * std::cos(start_angle + sweep / 2.0),
            center_y + radius * std::sin(start_angle + sweep / 2.0),
            0.0);
      }
      TopoDS_Edge matched;
      for (TopExp_Explorer explorer(profile, TopAbs_EDGE); explorer.More(); explorer.Next()) {
        const TopoDS_Edge candidate = TopoDS::Edge(explorer.Current());
        BRepAdaptor_Curve curve(candidate);
        if ((kind == NativeSegmentKind::Line && curve.GetType() != GeomAbs_Line)
            || (kind == NativeSegmentKind::CircularArc && curve.GetType() != GeomAbs_Circle)) {
          continue;
        }
        TopoDS_Vertex first;
        TopoDS_Vertex last;
        TopExp::Vertices(candidate, first, last);
        if (first.IsNull() || last.IsNull()) {
          continue;
        }
        const gp_Pnt first_point = BRep_Tool::Pnt(first);
        const gp_Pnt last_point = BRep_Tool::Pnt(last);
        const bool endpoints_match =
            (first_point.Distance(expected_start) <= tolerances().rounding
                && last_point.Distance(expected_end) <= tolerances().rounding)
            || (first_point.Distance(expected_end) <= tolerances().rounding
                && last_point.Distance(expected_start) <= tolerances().rounding);
        if (!endpoints_match) {
          continue;
        }
        if (kind == NativeSegmentKind::CircularArc) {
          const gp_Pnt candidate_middle = curve.Value(
              (curve.FirstParameter() + curve.LastParameter()) / 2.0);
          if (candidate_middle.Distance(expected_middle) > tolerances().accumulated_rounding) {
            continue;
          }
        }
        matched = candidate;
        break;
      }
      if (matched.IsNull()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT general revolve profile edge identity was lost");
      }
      operation_edges.push_back(matched);
    }
    const double angle_radians = angle_degrees * std::acos(-1.0) / 180.0;
    BRepPrimAPI_MakeRevol operation(
        profile,
        gp_Ax1(gp_Pnt(axis_start_x, axis_start_y, 0.0), gp_Dir(axis_vector)),
        angle_radians,
        true);
    if (!operation.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT general revolve builder did not complete");
    }
    const TopoDS_Shape result = operation.Shape();
    std::vector<HistoryRecord> history;
    const char* side_roles[] = {"revolve.side.0", "revolve.side.1"};
    for (std::size_t index = 0; index < 2; ++index) {
      const std::string source = std::string("profile.edge.") + std::to_string(index);
      HistoryRecord record{side_roles[index], "generated", source, 0, false};
      const NCollection_List<TopoDS_Shape>& generated = operation.Generated(operation_edges[index]);
      for (NCollection_List<TopoDS_Shape>::Iterator iterator(generated); iterator.More(); iterator.Next()) {
        const HistoryRecord candidate = history_record(
            side_roles[index], "generated", source, result, iterator.Value());
        if (candidate.output_present) {
          record = candidate;
          break;
        }
      }
      history.push_back(std::move(record));
    }
    if (angle_degrees < 360.0) {
      history.push_back(history_record(
          "revolve.start", "first_shape", "profile.face", result, operation.FirstShape()));
      history.push_back(history_record(
          "revolve.end", "last_shape", "profile.face", result, operation.LastShape()));
    }
    return success_result(result, std::move(history));
  });
}

} // namespace ketchup::exact
