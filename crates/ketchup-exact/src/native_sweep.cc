#include "native_common.hxx"

namespace ketchup::exact {

std::unique_ptr<NativeOperationResult> sweep_planar_profile_native(
    rust::Slice<const double> profile_segments,
    rust::Slice<const double> path_segments) noexcept {
  if (!path_segments.empty() && path_segments[0] >= 10.0) {
    return sweep_spatial_profile_native_impl(profile_segments, path_segments);
  }
  return guarded([&] {
    if (profile_segments.size() < 20 || profile_segments.size() % 10 != 0
        || path_segments.size() < 20 || path_segments.size() % 10 != 0
        || path_segments.size() > 640) {
      return error_result(STATUS_INVALID_PARAMETER, "OCCT curved Sweep payload is malformed");
    }
    for (double value : profile_segments) {
      if (!std::isfinite(value)) {
        return error_result(STATUS_NON_FINITE_PARAMETER, "OCCT curved Sweep profile is non-finite");
      }
    }
    for (double value : path_segments) {
      if (!std::isfinite(value)) {
        return error_result(STATUS_NON_FINITE_PARAMETER, "OCCT curved Sweep path is non-finite");
      }
    }
    for (std::size_t offset = 0; offset + 10 < path_segments.size(); offset += 10) {
      if (path_segments[offset + 3] != path_segments[offset + 11]
          || path_segments[offset + 4] != path_segments[offset + 12]) {
        return error_result(STATUS_INVALID_PARAMETER, "OCCT curved Sweep path is disconnected");
      }
    }
    const std::size_t last_path_offset = path_segments.size() - 10;
    if (path_segments[1] == path_segments[last_path_offset + 3]
        && path_segments[2] == path_segments[last_path_offset + 4]) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT curved Sweep path must remain geometrically open");
    }

    const auto path_metrics = [&](std::size_t offset, double& length,
                                  gp_Vec& start_tangent, gp_Vec& end_tangent) {
      const double kind = path_segments[offset];
      const double start_x = path_segments[offset + 1];
      const double start_y = path_segments[offset + 2];
      const double end_x = path_segments[offset + 3];
      const double end_y = path_segments[offset + 4];
      if (kind == 0.0) {
        const gp_Vec direction(end_x - start_x, end_y - start_y, 0.0);
        length = direction.Magnitude();
        if (length <= tolerances().linear_mm) return false;
        start_tangent = direction.Normalized();
        end_tangent = start_tangent;
        return true;
      }
      if (kind == 2.0) {
        const double control_1_x = path_segments[offset + 5];
        const double control_1_y = path_segments[offset + 6];
        const double control_2_x = path_segments[offset + 7];
        const double control_2_y = path_segments[offset + 8];
        const gp_Vec chord(end_x - start_x, end_y - start_y, 0.0);
        const gp_Vec start_handle(control_1_x - start_x, control_1_y - start_y, 0.0);
        const gp_Vec middle(control_2_x - control_1_x, control_2_y - control_1_y, 0.0);
        const gp_Vec end_handle(end_x - control_2_x, end_y - control_2_y, 0.0);
        const double chord_squared = chord.SquareMagnitude();
        const double start_length = start_handle.Magnitude();
        const double end_length = end_handle.Magnitude();
        const gp_Vec control_2_from_start(control_2_x - start_x, control_2_y - start_y, 0.0);
        const double projection_1 = start_handle.Dot(chord);
        const double projection_2 = control_2_from_start.Dot(chord);
        length = start_length + middle.Magnitude() + end_length;
        if (start_length <= tolerances().linear_mm || end_length <= tolerances().linear_mm
            || projection_1 <= 0.0 || projection_2 < projection_1
            || projection_2 >= chord_squared) {
          return false;
        }
        start_tangent = start_handle.Normalized();
        end_tangent = end_handle.Normalized();
        return true;
      }
      if (kind != 1.0) return false;
      const double center_x = path_segments[offset + 5];
      const double center_y = path_segments[offset + 6];
      const double start_dx = start_x - center_x;
      const double start_dy = start_y - center_y;
      const double end_dx = end_x - center_x;
      const double end_dy = end_y - center_y;
      const double radius = std::hypot(start_dx, start_dy);
      const double end_radius = std::hypot(end_dx, end_dy);
      if (radius <= tolerances().linear_mm || std::abs(radius - end_radius) > tolerances().rounding
          || std::hypot(end_x - start_x, end_y - start_y) <= tolerances().linear_mm) {
        return false;
      }
      const bool clockwise = path_segments[offset + 9] != 0.0;
      const double start_angle = std::atan2(start_dy, start_dx);
      const double end_angle = std::atan2(end_dy, end_dx);
      double sweep = end_angle - start_angle;
      const double tau = 2.0 * std::acos(-1.0);
      if (clockwise) {
        if (sweep >= 0.0) sweep -= tau;
      } else if (sweep <= 0.0) {
        sweep += tau;
      }
      length = radius * std::abs(sweep);
      if (length <= tolerances().linear_mm) return false;
      const double sign = clockwise ? -1.0 : 1.0;
      start_tangent = gp_Vec(sign * -start_dy / radius, sign * start_dx / radius, 0.0);
      end_tangent = gp_Vec(sign * -end_dy / end_radius, sign * end_dx / end_radius, 0.0);
      return true;
    };

    const std::size_t path_segment_count = path_segments.size() / 10;
    std::vector<gp_Vec> start_tangents(path_segment_count);
    std::vector<gp_Vec> end_tangents(path_segment_count);
    std::vector<double> segment_lengths(path_segment_count);
    double path_length = 0.0;
    for (std::size_t index = 0; index < path_segment_count; ++index) {
      if (!path_metrics(
              index * 10, segment_lengths[index], start_tangents[index], end_tangents[index])) {
        return error_result(STATUS_INVALID_PARAMETER, "OCCT curved Sweep path violates its bounded segment contract");
      }
      path_length += segment_lengths[index];
      if (index > 0
          && (end_tangents[index - 1].Dot(start_tangents[index]) < 1.0 - tolerances().rounding
              || end_tangents[index - 1].Crossed(start_tangents[index]).Magnitude()
                  > tolerances().rounding)) {
        return error_result(STATUS_INVALID_PARAMETER, "OCCT curved Sweep path violates its bounded C1 contract");
      }
      const std::size_t offset = index * 10;
      const std::size_t previous = offset - 10;
      if (index > 0 && path_segments[previous] == 1.0 && path_segments[offset] == 1.0
          && path_segments[previous + 5] == path_segments[offset + 5]
          && path_segments[previous + 6] == path_segments[offset + 6]) {
        const double radius = std::hypot(
            path_segments[offset + 1] - path_segments[offset + 5],
            path_segments[offset + 2] - path_segments[offset + 6]);
        if (segment_lengths[index - 1] + segment_lengths[index]
            >= 2.0 * std::acos(-1.0) * radius - tolerances().linear_mm) {
          return error_result(STATUS_INVALID_SHAPE, "OCCT curved Sweep adjacent arcs overlap");
        }
      }
    }
    if (path_length < 0.01 || path_length > 100000.0) {
      return error_result(STATUS_INVALID_PARAMETER, "OCCT curved Sweep path length is outside its bounded contract");
    }

    const auto path_edge = [&](std::size_t offset) {
      const gp_Pnt start(path_segments[offset + 1], path_segments[offset + 2], 0.0);
      const gp_Pnt end(path_segments[offset + 3], path_segments[offset + 4], 0.0);
      if (path_segments[offset] == 0.0) {
        BRepBuilderAPI_MakeEdge builder(start, end);
        return builder.IsDone() ? builder.Edge() : TopoDS_Edge{};
      }
      if (path_segments[offset] == 2.0) {
        return cubic_bezier_edge(path_segments, offset, 0.0);
      }
      const double center_x = path_segments[offset + 5];
      const double center_y = path_segments[offset + 6];
      const double start_angle = std::atan2(start.Y() - center_y, start.X() - center_x);
      const double end_angle = std::atan2(end.Y() - center_y, end.X() - center_x);
      const double radius = start.Distance(gp_Pnt(center_x, center_y, 0.0));
      const bool counterclockwise = path_segments[offset + 9] == 0.0;
      GC_MakeArcOfCircle arc_builder(
          gp_Circ(
              gp_Ax2(gp_Pnt(center_x, center_y, 0.0), gp_Dir(0.0, 0.0, 1.0)),
              radius),
          start_angle,
          end_angle,
          counterclockwise);
      if (!arc_builder.IsDone()) return TopoDS_Edge{};
      BRepBuilderAPI_MakeEdge edge_builder(arc_builder.Value());
      return edge_builder.IsDone() ? edge_builder.Edge() : TopoDS_Edge{};
    };

    BRepBuilderAPI_MakeWire spine_builder;
    std::vector<TopoDS_Edge> path_edges;
    std::vector<Bnd_Box> path_edge_bounds;
    path_edges.reserve(path_segment_count);
    path_edge_bounds.reserve(path_segment_count);
    for (std::size_t offset = 0; offset < path_segments.size(); offset += 10) {
      const TopoDS_Edge edge = path_edge(offset);
      if (edge.IsNull()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT curved Sweep path edge is null");
      }
      path_edges.push_back(edge);
      Bnd_Box edge_bounds;
      BRepBndLib::AddOptimal(edge, edge_bounds, false, false);
      path_edge_bounds.push_back(edge_bounds);
      spine_builder.Add(edge);
    }
    for (std::size_t left = 0; left < path_edges.size(); ++left) {
      for (std::size_t right = left + 1; right < path_edges.size(); ++right) {
        if (path_edge_bounds[left].IsOut(path_edge_bounds[right])) continue;
        BRepExtrema_DistShapeShape distance(path_edges[left], path_edges[right]);
        distance.Perform();
        if (!distance.IsDone()) {
          return error_result(STATUS_INVALID_SHAPE, "OCCT curved Sweep path intersection check failed");
        }
        if (distance.Value() <= tolerances().linear_mm) {
          bool shared_endpoint_only = false;
          if (right == left + 1 && distance.NbSolution() > 0) {
            const gp_Pnt shared(
                path_segments[right * 10 + 1], path_segments[right * 10 + 2], 0.0);
            shared_endpoint_only = true;
            for (Standard_Integer solution = 1; solution <= distance.NbSolution(); ++solution) {
              if (distance.PointOnShape1(solution).Distance(shared) > tolerances().linear_mm
                  || distance.PointOnShape2(solution).Distance(shared) > tolerances().linear_mm) {
                shared_endpoint_only = false;
                break;
              }
            }
          }
          if (!shared_endpoint_only) {
            return error_result(STATUS_INVALID_SHAPE, "OCCT curved Sweep path self-intersects");
          }
        }
      }
    }
    if (!spine_builder.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT curved Sweep spine wire did not complete");
    }
    const TopoDS_Wire spine = spine_builder.Wire();
    if (!BRepCheck_Analyzer(spine).IsValid()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT curved Sweep spine wire is invalid");
    }

    const double path_start_x = path_segments[1];
    const double path_start_y = path_segments[2];
    const double section_x = start_tangents.front().Y();
    const double section_y = -start_tangents.front().X();
    const auto section_point = [&](double u, double v) {
      return gp_Pnt(
          path_start_x + section_x * u,
          path_start_y + section_y * u,
          v);
    };
    BRepBuilderAPI_MakeWire profile_builder;
    for (std::size_t offset = 0; offset < profile_segments.size(); offset += 10) {
      const double kind = profile_segments[offset];
      const gp_Pnt start = section_point(
          profile_segments[offset + 1], profile_segments[offset + 2]);
      const gp_Pnt end = section_point(
          profile_segments[offset + 3], profile_segments[offset + 4]);
      TopoDS_Edge edge;
      if (kind == 0.0) {
        BRepBuilderAPI_MakeEdge edge_builder(start, end);
        if (edge_builder.IsDone()) edge = edge_builder.Edge();
      } else if (kind == 1.0) {
        const double center_u = profile_segments[offset + 5];
        const double center_v = profile_segments[offset + 6];
        const double start_angle = std::atan2(
            profile_segments[offset + 2] - center_v,
            profile_segments[offset + 1] - center_u);
        const double end_angle = std::atan2(
            profile_segments[offset + 4] - center_v,
            profile_segments[offset + 3] - center_u);
        double sweep = end_angle - start_angle;
        const double tau = 2.0 * std::acos(-1.0);
        if (profile_segments[offset + 9] != 0.0) {
          if (sweep >= 0.0) sweep -= tau;
        } else if (sweep <= 0.0) {
          sweep += tau;
        }
        const double radius = std::hypot(
            profile_segments[offset + 1] - center_u,
            profile_segments[offset + 2] - center_v);
        const double middle_angle = start_angle + sweep / 2.0;
        const gp_Pnt middle = section_point(
            center_u + radius * std::cos(middle_angle),
            center_v + radius * std::sin(middle_angle));
        GC_MakeArcOfCircle arc_builder(start, middle, end);
        if (arc_builder.IsDone()) {
          BRepBuilderAPI_MakeEdge edge_builder(arc_builder.Value());
          if (edge_builder.IsDone()) edge = edge_builder.Edge();
        }
      } else if (kind == 2.0) {
        TColgp_Array1OfPnt poles(1, 4);
        poles.SetValue(1, start);
        poles.SetValue(2, section_point(
            profile_segments[offset + 5], profile_segments[offset + 6]));
        poles.SetValue(3, section_point(
            profile_segments[offset + 7], profile_segments[offset + 8]));
        poles.SetValue(4, end);
        occ::handle<Geom_BezierCurve> curve = new Geom_BezierCurve(poles);
        BRepBuilderAPI_MakeEdge edge_builder(curve);
        if (edge_builder.IsDone()) edge = edge_builder.Edge();
      } else {
        return error_result(STATUS_INVALID_PARAMETER, "OCCT curved Sweep profile kind is invalid");
      }
      if (edge.IsNull()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT curved Sweep profile edge is null");
      }
      profile_builder.Add(edge);
    }
    if (!profile_builder.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT curved Sweep profile wire did not complete");
    }
    const TopoDS_Wire profile = profile_builder.Wire();
    if (!BRepCheck_Analyzer(profile).IsValid()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT curved Sweep profile wire is invalid");
    }

    BRepOffsetAPI_MakePipeShell operation(spine);
    operation.SetMode(gp_Dir(0.0, 0.0, 1.0));
    operation.SetTolerance(tolerances().linear_mm, tolerances().linear_mm, tolerances().rounding);
    operation.Add(profile, false, false);
    if (!operation.IsReady()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT curved Sweep pipe is not ready");
    }
    operation.Build();
    if (!operation.IsDone() || !operation.MakeSolid()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT curved Sweep pipe did not produce a solid");
    }
    const TopoDS_Shape result = operation.Shape();
    if (result.IsNull() || !BRepCheck_Analyzer(result).IsValid()
        || count_subshapes(result, TopAbs_SOLID) != 1) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT curved Sweep result is not one valid solid");
    }
    std::vector<HistoryRecord> history;
    history.push_back(history_record(
        "sweep.start", "first_shape", "profile.wire", result, operation.FirstShape()));
    history.push_back(history_record(
        "sweep.end", "last_shape", "profile.wire", result, operation.LastShape()));
    return success_result(result, std::move(history));
  });
}

std::unique_ptr<NativeOperationResult> sweep_spatial_profile_native_impl(
    rust::Slice<const double> profile_segments,
    rust::Slice<const double> path_segments) noexcept {
  return guarded([&] {
    constexpr std::size_t spatial_stride = 14;
    const double epsilon = tolerances().rounding;
    const double minimum_segment_length = tolerances().linear_mm;
    constexpr double coordinate_limit = 1000000.0;
    const double tau = 2.0 * std::acos(-1.0);
    const auto bounded = [&](double value) {
      return std::isfinite(value) && std::abs(value) <= coordinate_limit;
    };
    if (profile_segments.size() < 20 || profile_segments.size() > 640
        || profile_segments.size() % 10 != 0
        || path_segments.size() < spatial_stride
        || path_segments.size() > 64 * spatial_stride
        || path_segments.size() % spatial_stride != 0) {
      return error_result(STATUS_INVALID_PARAMETER, "OCCT spatial Sweep payload is malformed");
    }
    for (double value : profile_segments) {
      if (!bounded(value)) {
        return error_result(
            std::isfinite(value) ? STATUS_INVALID_PARAMETER : STATUS_NON_FINITE_PARAMETER,
            "OCCT spatial Sweep profile value is outside its bounded contract");
      }
    }
    for (double value : path_segments) {
      if (!bounded(value)) {
        return error_result(
            std::isfinite(value) ? STATUS_INVALID_PARAMETER : STATUS_NON_FINITE_PARAMETER,
            "OCCT spatial Sweep path value is outside its bounded contract");
      }
    }

    const std::size_t path_count = path_segments.size() / spatial_stride;
    std::vector<double> lengths(path_count);
    std::vector<gp_Vec> start_tangents(path_count);
    std::vector<gp_Vec> end_tangents(path_count);
    const auto point_at = [&](std::size_t offset, std::size_t first) {
      return gp_Pnt(
          path_segments[offset + first],
          path_segments[offset + first + 1],
          path_segments[offset + first + 2]);
    };
    const auto positive_remainder = [&](double value) {
      const double remainder = std::fmod(value, tau);
      return remainder < 0.0 ? remainder + tau : remainder;
    };
    const auto metrics = [&](std::size_t offset, double& length,
                             gp_Vec& start_tangent, gp_Vec& end_tangent) {
      const double kind = path_segments[offset];
      const gp_Pnt start = point_at(offset, 1);
      const gp_Pnt end = point_at(offset, 4);
      if (kind == 10.0) {
        const gp_Vec direction(start, end);
        length = direction.Magnitude();
        if (!std::isfinite(length) || length <= minimum_segment_length) return false;
        start_tangent = direction.Normalized();
        end_tangent = start_tangent;
        return true;
      }
      if (kind == 12.0) {
        const gp_Pnt control_1 = point_at(offset, 7);
        const gp_Pnt control_2 = point_at(offset, 10);
        const gp_Vec chord(start, end);
        const gp_Vec first(start, control_1);
        const gp_Vec middle(control_1, control_2);
        const gp_Vec last(control_2, end);
        const gp_Vec control_2_from_start(start, control_2);
        const double first_length = first.Magnitude();
        const double last_length = last.Magnitude();
        const double projection_1 = first.Dot(chord);
        const double projection_2 = control_2_from_start.Dot(chord);
        length = first_length + middle.Magnitude() + last_length;
        if (!std::isfinite(length) || first_length <= minimum_segment_length
            || last_length <= minimum_segment_length || projection_1 <= 0.0
            || projection_2 < projection_1
            || projection_2 >= chord.SquareMagnitude()) {
          return false;
        }
        start_tangent = first.Normalized();
        end_tangent = last.Normalized();
        return true;
      }
      if (kind != 11.0 || (path_segments[offset + 13] != 0.0
                           && path_segments[offset + 13] != 1.0)) {
        return false;
      }
      const gp_Pnt center = point_at(offset, 7);
      const gp_Vec normal(
          path_segments[offset + 10],
          path_segments[offset + 11],
          path_segments[offset + 12]);
      const double normal_length = normal.Magnitude();
      if (!std::isfinite(normal_length) || std::abs(normal_length - 1.0) > epsilon) {
        return false;
      }
      const gp_Vec unit_normal = normal.Normalized();
      const gp_Vec start_radius(center, start);
      const gp_Vec end_radius(center, end);
      const double radius = start_radius.Magnitude();
      const double end_radius_length = end_radius.Magnitude();
      if (radius <= minimum_segment_length || start.Distance(end) == 0.0
          || std::abs(radius - end_radius_length) > epsilon
          || std::abs(start_radius.Dot(unit_normal)) > epsilon
          || std::abs(end_radius.Dot(unit_normal)) > epsilon) {
        return false;
      }
      const double signed_angle = std::atan2(
          unit_normal.Dot(start_radius.Crossed(end_radius)),
          start_radius.Dot(end_radius));
      const bool clockwise = path_segments[offset + 13] != 0.0;
      const double angle = positive_remainder(clockwise ? -signed_angle : signed_angle);
      length = radius * angle;
      if (!std::isfinite(length) || length <= minimum_segment_length) return false;
      const double sign = clockwise ? -1.0 : 1.0;
      start_tangent = unit_normal.Crossed(start_radius).Multiplied(sign).Normalized();
      end_tangent = unit_normal.Crossed(end_radius).Multiplied(sign).Normalized();
      return true;
    };

    double path_length = 0.0;
    for (std::size_t index = 0; index < path_count; ++index) {
      const std::size_t offset = index * spatial_stride;
      if (!metrics(offset, lengths[index], start_tangents[index], end_tangents[index])) {
        return error_result(
            STATUS_INVALID_PARAMETER,
            "OCCT spatial Sweep path violates its bounded segment contract");
      }
      path_length += lengths[index];
      if (index == 0) continue;
      const std::size_t previous = offset - spatial_stride;
      if (point_at(previous, 4).Distance(point_at(offset, 1)) != 0.0) {
        return error_result(STATUS_INVALID_PARAMETER, "OCCT spatial Sweep path is disconnected");
      }
      if (end_tangents[index - 1].Dot(start_tangents[index]) < 1.0 - epsilon
          || end_tangents[index - 1].Crossed(start_tangents[index]).Magnitude() > epsilon) {
        return error_result(
            STATUS_INVALID_PARAMETER,
            "OCCT spatial Sweep path violates its bounded C1 contract");
      }
      if (path_segments[previous] == 11.0 && path_segments[offset] == 11.0
          && point_at(previous, 7).Distance(point_at(offset, 7)) == 0.0) {
        const double radius = point_at(offset, 1).Distance(point_at(offset, 7));
        if (lengths[index - 1] + lengths[index] >= tau * radius - epsilon) {
          return error_result(STATUS_INVALID_SHAPE, "OCCT spatial Sweep adjacent arcs overlap");
        }
      }
    }
    if (!std::isfinite(path_length) || path_length < 0.01 || path_length > 100000.0) {
      return error_result(
          STATUS_INVALID_PARAMETER,
          "OCCT spatial Sweep path length is outside its bounded contract");
    }
    const bool closed =
        point_at(0, 1).Distance(point_at((path_count - 1) * spatial_stride, 4)) == 0.0;
    if (closed
        && (end_tangents.back().Dot(start_tangents.front()) < 1.0 - epsilon
            || end_tangents.back().Crossed(start_tangents.front()).Magnitude() > epsilon)) {
      return error_result(
          STATUS_INVALID_PARAMETER, "OCCT closed spatial Sweep seam violates its C1 contract");
    }

    const auto make_path_edge = [&](std::size_t offset) {
      const gp_Pnt start = point_at(offset, 1);
      const gp_Pnt end = point_at(offset, 4);
      if (path_segments[offset] == 10.0) {
        BRepBuilderAPI_MakeEdge builder(start, end);
        return builder.IsDone() ? builder.Edge() : TopoDS_Edge{};
      }
      if (path_segments[offset] == 12.0) {
        TColgp_Array1OfPnt poles(1, 4);
        poles.SetValue(1, start);
        poles.SetValue(2, point_at(offset, 7));
        poles.SetValue(3, point_at(offset, 10));
        poles.SetValue(4, end);
        occ::handle<Geom_BezierCurve> curve = new Geom_BezierCurve(poles);
        BRepBuilderAPI_MakeEdge builder(curve);
        return builder.IsDone() ? builder.Edge() : TopoDS_Edge{};
      }
      const gp_Pnt center = point_at(offset, 7);
      const gp_Vec normal(
          path_segments[offset + 10],
          path_segments[offset + 11],
          path_segments[offset + 12]);
      const gp_Vec start_radius(center, start);
      const gp_Vec end_radius(center, end);
      const double signed_angle = std::atan2(
          normal.Dot(start_radius.Crossed(end_radius)), start_radius.Dot(end_radius));
      const bool clockwise = path_segments[offset + 13] != 0.0;
      const double angle = positive_remainder(clockwise ? -signed_angle : signed_angle);
      const double rotation = clockwise ? -angle : angle;
      const gp_Vec middle_radius = start_radius.Rotated(
          gp_Ax1(center, gp_Dir(normal)), rotation / 2.0);
      const gp_Pnt middle = center.Translated(middle_radius);
      GC_MakeArcOfCircle arc_builder(start, middle, end);
      if (!arc_builder.IsDone()) return TopoDS_Edge{};
      BRepBuilderAPI_MakeEdge edge_builder(arc_builder.Value());
      return edge_builder.IsDone() ? edge_builder.Edge() : TopoDS_Edge{};
    };

    BRepBuilderAPI_MakeWire spine_builder;
    std::vector<TopoDS_Edge> path_edges;
    path_edges.reserve(path_count);
    for (std::size_t index = 0; index < path_count; ++index) {
      const TopoDS_Edge edge = make_path_edge(index * spatial_stride);
      if (edge.IsNull()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT spatial Sweep path edge is null");
      }
      path_edges.push_back(edge);
      spine_builder.Add(edge);
    }
    for (std::size_t left = 0; left < path_edges.size(); ++left) {
      for (std::size_t right = left + 1; right < path_edges.size(); ++right) {
        BRepExtrema_DistShapeShape distance(path_edges[left], path_edges[right]);
        distance.Perform();
        if (!distance.IsDone()) {
          return error_result(
              STATUS_INVALID_SHAPE, "OCCT spatial Sweep path intersection check failed");
        }
        if (distance.Value() > minimum_segment_length) continue;
        bool shared_endpoint_only = false;
        const bool adjacent = right == left + 1
            || (closed && left == 0 && right + 1 == path_edges.size());
        if (adjacent && distance.NbSolution() > 0) {
          const gp_Pnt shared = right == left + 1
              ? point_at(right * spatial_stride, 1)
              : point_at(0, 1);
          shared_endpoint_only = true;
          for (Standard_Integer solution = 1; solution <= distance.NbSolution(); ++solution) {
            if (distance.PointOnShape1(solution).Distance(shared) > minimum_segment_length
                || distance.PointOnShape2(solution).Distance(shared) > minimum_segment_length) {
              shared_endpoint_only = false;
              break;
            }
          }
        }
        if (!shared_endpoint_only) {
          return error_result(STATUS_INVALID_SHAPE, "OCCT spatial Sweep path self-intersects");
        }
      }
    }
    if (!spine_builder.IsDone() || !BRepCheck_Analyzer(spine_builder.Wire()).IsValid()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT spatial Sweep spine wire is invalid");
    }
    TopoDS_Wire spine = spine_builder.Wire();
    spine.Closed(closed);

    const gp_Pnt path_start = point_at(0, 1);
    const gp_Vec tangent = start_tangents.front();
    gp_Vec reference(0.0, 0.0, 1.0);
    if (tangent.Crossed(reference).SquareMagnitude() <= epsilon * epsilon) {
      reference = gp_Vec(0.0, 1.0, 0.0);
    }
    const gp_Vec frame_u = tangent.Crossed(reference).Normalized();
    const gp_Vec frame_v = frame_u.Crossed(tangent).Normalized();
    const auto section_point = [&](double u, double v) {
      return path_start.Translated(frame_u.Multiplied(u).Added(frame_v.Multiplied(v)));
    };

    BRepBuilderAPI_MakeWire profile_builder;
    for (std::size_t offset = 0; offset < profile_segments.size(); offset += 10) {
      const std::size_t next = (offset + 10) % profile_segments.size();
      if (profile_segments[offset + 3] != profile_segments[next + 1]
          || profile_segments[offset + 4] != profile_segments[next + 2]) {
        return error_result(STATUS_INVALID_PARAMETER, "OCCT spatial Sweep profile is open");
      }
      const double kind = profile_segments[offset];
      const gp_Pnt start = section_point(profile_segments[offset + 1], profile_segments[offset + 2]);
      const gp_Pnt end = section_point(profile_segments[offset + 3], profile_segments[offset + 4]);
      TopoDS_Edge edge;
      if (kind == 0.0) {
        BRepBuilderAPI_MakeEdge builder(start, end);
        if (builder.IsDone()) edge = builder.Edge();
      } else if (kind == 1.0) {
        const double center_u = profile_segments[offset + 5];
        const double center_v = profile_segments[offset + 6];
        const double start_angle = std::atan2(
            profile_segments[offset + 2] - center_v,
            profile_segments[offset + 1] - center_u);
        const double end_angle = std::atan2(
            profile_segments[offset + 4] - center_v,
            profile_segments[offset + 3] - center_u);
        double sweep = end_angle - start_angle;
        if (profile_segments[offset + 9] != 0.0) {
          if (sweep >= 0.0) sweep -= tau;
        } else if (sweep <= 0.0) {
          sweep += tau;
        }
        const double radius = std::hypot(
            profile_segments[offset + 1] - center_u,
            profile_segments[offset + 2] - center_v);
        const gp_Pnt middle = section_point(
            center_u + radius * std::cos(start_angle + sweep / 2.0),
            center_v + radius * std::sin(start_angle + sweep / 2.0));
        GC_MakeArcOfCircle arc_builder(start, middle, end);
        if (arc_builder.IsDone()) {
          BRepBuilderAPI_MakeEdge builder(arc_builder.Value());
          if (builder.IsDone()) edge = builder.Edge();
        }
      } else if (kind == 2.0) {
        TColgp_Array1OfPnt poles(1, 4);
        poles.SetValue(1, start);
        poles.SetValue(2, section_point(profile_segments[offset + 5], profile_segments[offset + 6]));
        poles.SetValue(3, section_point(profile_segments[offset + 7], profile_segments[offset + 8]));
        poles.SetValue(4, end);
        occ::handle<Geom_BezierCurve> curve = new Geom_BezierCurve(poles);
        BRepBuilderAPI_MakeEdge builder(curve);
        if (builder.IsDone()) edge = builder.Edge();
      } else {
        return error_result(STATUS_INVALID_PARAMETER, "OCCT spatial Sweep profile kind is invalid");
      }
      if (edge.IsNull()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT spatial Sweep profile edge is null");
      }
      profile_builder.Add(edge);
    }
    if (!profile_builder.IsDone() || !BRepCheck_Analyzer(profile_builder.Wire()).IsValid()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT spatial Sweep profile wire is invalid");
    }
    const TopoDS_Wire profile = profile_builder.Wire();

    BRepOffsetAPI_MakePipeShell operation(spine);
    // Corrected Frenet is OCCT's deterministic minimum-twist transport mode.
    operation.SetMode(false);
    operation.SetTolerance(tolerances().linear_mm, tolerances().linear_mm, tolerances().rounding);
    operation.Add(profile, false, false);
    if (!operation.IsReady()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT spatial Sweep pipe is not ready");
    }
    operation.Build();
    if (!operation.IsDone() || !operation.MakeSolid()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT spatial Sweep pipe did not produce a solid");
    }
    const TopoDS_Shape result = operation.Shape();
    if (result.IsNull() || !BRepCheck_Analyzer(result).IsValid()
        || count_subshapes(result, TopAbs_SOLID) != 1) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT spatial Sweep result is not one valid solid");
    }
    std::vector<HistoryRecord> history;
    history.push_back(history_record(
        "sweep.start", "first_shape", "profile.wire", result, operation.FirstShape()));
    history.push_back(history_record(
        "sweep.end", "last_shape", "profile.wire", result, operation.LastShape()));
    return success_result(result, std::move(history));
  });
}

} // namespace ketchup::exact
