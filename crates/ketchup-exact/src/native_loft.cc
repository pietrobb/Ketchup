#include "native_common.hxx"

namespace ketchup::exact {

std::unique_ptr<NativeOperationResult> loft_framed_profiles_native(
    rust::Slice<const double> values,
    rust::Slice<const double> guide_segments,
    std::uint8_t continuity,
    bool make_solid) noexcept {
  return guarded([&] {
    if (values.empty() || !std::isfinite(values[0])) {
      return error_result(STATUS_INVALID_PARAMETER, "OCCT framed Loft payload is malformed");
    }
    const std::size_t section_count = static_cast<std::size_t>(values[0]);
    if (section_count < 2 || section_count > 16
        || values[0] != static_cast<double>(section_count)) {
      return error_result(STATUS_INVALID_PARAMETER, "OCCT framed Loft section count is invalid");
    }
    std::size_t cursor = 1;
    double previous_elevation = -std::numeric_limits<double>::infinity();
    std::vector<TopoDS_Wire> wires;
    std::vector<TopoDS_Edge> section_edges;
    wires.reserve(section_count);
    section_edges.reserve(section_count);
    for (std::size_t section = 0; section < section_count; ++section) {
      if (cursor + 15 > values.size()) {
        return error_result(STATUS_INVALID_PARAMETER, "OCCT framed Loft section payload is truncated");
      }
      const double kind = values[cursor++];
      const double elevation = values[cursor++];
      const std::size_t count = static_cast<std::size_t>(values[cursor++]);
      const double* frame = values.data() + cursor;
      cursor += 12;
      if (!std::isfinite(kind) || !std::isfinite(elevation)
          || elevation <= previous_elevation || count == 0 || count > 64) {
        return error_result(STATUS_INVALID_PARAMETER, "OCCT framed Loft section header is invalid");
      }
      previous_elevation = elevation;
      for (std::size_t index = 0; index < 12; ++index) {
        if (!std::isfinite(frame[index]) || std::abs(frame[index]) > 1000000.0) {
          return error_result(STATUS_INVALID_PARAMETER, "OCCT framed Loft frame is invalid");
        }
      }
      const gp_Vec x_axis(frame[3], frame[4], frame[5]);
      const gp_Vec y_axis(frame[6], frame[7], frame[8]);
      const gp_Vec normal(frame[9], frame[10], frame[11]);
      if (std::abs(x_axis.Magnitude() - 1.0) > tolerances().accumulated_rounding
          || std::abs(y_axis.Magnitude() - 1.0) > tolerances().accumulated_rounding
          || std::abs(normal.Magnitude() - 1.0) > tolerances().accumulated_rounding
          || std::abs(x_axis.Dot(y_axis)) > tolerances().accumulated_rounding
          || std::abs(x_axis.Dot(normal)) > tolerances().accumulated_rounding
          || std::abs(y_axis.Dot(normal)) > tolerances().accumulated_rounding
          || x_axis.Crossed(y_axis).Dot(normal) < 1.0 - tolerances().accumulated_rounding) {
        return error_result(STATUS_INVALID_PARAMETER, "OCCT framed Loft frame is not right-handed orthonormal");
      }
      const auto world_point = [&](double x, double y) {
        return gp_Pnt(
            frame[0] + frame[3] * x + frame[6] * y + frame[9] * elevation,
            frame[1] + frame[4] * x + frame[7] * y + frame[10] * elevation,
            frame[2] + frame[5] * x + frame[8] * y + frame[11] * elevation);
      };
      BRepBuilderAPI_MakeWire wire_builder;
      TopoDS_Edge first_edge;
      if (kind == 0.0) {
        if (cursor + count * 10 > values.size() || count < 2) {
          return error_result(STATUS_INVALID_PARAMETER, "OCCT framed Loft boundary payload is invalid");
        }
        bool line_only = true;
        for (std::size_t index = 0; index < count; ++index) {
          const std::size_t offset = cursor + index * 10;
          for (std::size_t value = 0; value < 10; ++value) {
            if (!std::isfinite(values[offset + value])
                || std::abs(values[offset + value]) > 1000000.0) {
              return error_result(STATUS_INVALID_PARAMETER, "OCCT framed Loft boundary value is invalid");
            }
          }
          const double segment_kind = values[offset];
          const gp_Pnt start = world_point(values[offset + 1], values[offset + 2]);
          const gp_Pnt end = world_point(values[offset + 3], values[offset + 4]);
          TopoDS_Edge edge;
          if (segment_kind == 0.0) {
            if (values[offset + 5] != 0.0 || values[offset + 6] != 0.0
                || values[offset + 7] != 0.0 || values[offset + 8] != 0.0
                || values[offset + 9] != 0.0 || start.Distance(end) < 0.01
                || start.Distance(end) > 100000.0) {
              return error_result(STATUS_INVALID_PARAMETER, "OCCT framed Loft line is invalid");
            }
            BRepBuilderAPI_MakeEdge edge_builder(start, end);
            if (edge_builder.IsDone()) edge = edge_builder.Edge();
          } else if (segment_kind == 1.0) {
            line_only = false;
            const double center_x = values[offset + 5];
            const double center_y = values[offset + 6];
            const double radius = std::hypot(values[offset + 1] - center_x,
                                             values[offset + 2] - center_y);
            const double end_radius = std::hypot(values[offset + 3] - center_x,
                                                 values[offset + 4] - center_y);
            if (values[offset + 7] != 0.0 || values[offset + 8] != 0.0
                || (values[offset + 9] != 0.0 && values[offset + 9] != 1.0)
                || radius < 0.01 || radius > 100000.0
                || std::abs(radius - end_radius) > tolerances().rounding * std::max({radius, end_radius, 1.0})) {
              return error_result(STATUS_INVALID_PARAMETER, "OCCT framed Loft arc is invalid");
            }
            const double tau = 2.0 * std::acos(-1.0);
            const double start_angle = std::atan2(values[offset + 2] - center_y,
                                                  values[offset + 1] - center_x);
            const double end_angle = std::atan2(values[offset + 4] - center_y,
                                                values[offset + 3] - center_x);
            double sweep = end_angle - start_angle;
            if (values[offset + 9] != 0.0) {
              while (sweep >= 0.0) sweep -= tau;
            } else {
              while (sweep <= 0.0) sweep += tau;
            }
            const double middle_angle = start_angle + sweep / 2.0;
            const gp_Pnt middle = world_point(
                center_x + radius * std::cos(middle_angle),
                center_y + radius * std::sin(middle_angle));
            GC_MakeArcOfCircle arc_builder(start, middle, end);
            if (arc_builder.IsDone()) {
              BRepBuilderAPI_MakeEdge edge_builder(arc_builder.Value());
              if (edge_builder.IsDone()) edge = edge_builder.Edge();
            }
          } else if (segment_kind == 2.0) {
            line_only = false;
            if (values[offset + 9] != 0.0) {
              return error_result(STATUS_INVALID_PARAMETER, "OCCT framed Loft cubic is invalid");
            }
            const gp_Pnt control_1 = world_point(values[offset + 5], values[offset + 6]);
            const gp_Pnt control_2 = world_point(values[offset + 7], values[offset + 8]);
            const double length = start.Distance(control_1) + control_1.Distance(control_2)
                                  + control_2.Distance(end);
            if (!std::isfinite(length) || length < 0.01 || length > 100000.0) {
              return error_result(STATUS_INVALID_PARAMETER, "OCCT framed Loft cubic length is invalid");
            }
            TColgp_Array1OfPnt poles(1, 4);
            poles.SetValue(1, start);
            poles.SetValue(2, control_1);
            poles.SetValue(3, control_2);
            poles.SetValue(4, end);
            occ::handle<Geom_BezierCurve> curve = new Geom_BezierCurve(poles);
            BRepBuilderAPI_MakeEdge edge_builder(curve);
            if (edge_builder.IsDone()) edge = edge_builder.Edge();
          } else {
            return error_result(STATUS_INVALID_PARAMETER, "OCCT framed Loft boundary kind is invalid");
          }
          const std::size_t next = cursor + ((index + 1) % count) * 10;
          if (values[offset + 3] != values[next + 1]
              || values[offset + 4] != values[next + 2]) {
            return error_result(STATUS_INVALID_PARAMETER, "OCCT framed Loft boundary is open");
          }
          if (edge.IsNull()) {
            return error_result(STATUS_INVALID_SHAPE, "OCCT framed Loft boundary edge is null");
          }
          if (first_edge.IsNull()) first_edge = edge;
          wire_builder.Add(edge);
        }
        if (line_only && count < 3) {
          return error_result(STATUS_INVALID_SHAPE, "OCCT framed Loft polygon is degenerate");
        }
        cursor += count * 10;
      } else if (kind == 1.0) {
        if (count != 1 || cursor + 3 > values.size()) {
          return error_result(STATUS_INVALID_PARAMETER, "OCCT framed Loft circle payload is invalid");
        }
        const double center_x = values[cursor++];
        const double center_y = values[cursor++];
        const double radius = values[cursor++];
        if (!std::isfinite(center_x) || !std::isfinite(center_y) || !std::isfinite(radius)
            || radius < 0.01 || radius > 100000.0) {
          return error_result(STATUS_INVALID_PARAMETER, "OCCT framed Loft circle is invalid");
        }
        BRepBuilderAPI_MakeEdge edge_builder(gp_Circ(
            gp_Ax2(world_point(center_x, center_y), gp_Dir(normal), gp_Dir(x_axis)), radius));
        if (!edge_builder.IsDone()) {
          return error_result(STATUS_INVALID_SHAPE, "OCCT framed Loft circle edge is null");
        }
        first_edge = edge_builder.Edge();
        wire_builder.Add(first_edge);
      } else if (kind == 2.0) {
        if (count < 4 || cursor + count * 2 > values.size()) {
          return error_result(STATUS_INVALID_PARAMETER, "OCCT framed Loft spline payload is invalid");
        }
        occ::handle<TColgp_HArray1OfPnt> points =
            new TColgp_HArray1OfPnt(1, static_cast<Standard_Integer>(count));
        for (std::size_t point = 0; point < count; ++point) {
          const double x = values[cursor++];
          const double y = values[cursor++];
          if (!std::isfinite(x) || !std::isfinite(y)
              || std::abs(x) > 1000000.0 || std::abs(y) > 1000000.0) {
            return error_result(STATUS_INVALID_PARAMETER, "OCCT framed Loft spline point is invalid");
          }
          points->SetValue(static_cast<Standard_Integer>(point + 1), world_point(x, y));
        }
        GeomAPI_Interpolate interpolation(points, true, tolerances().rounding);
        interpolation.Perform();
        if (!interpolation.IsDone()) {
          return error_result(STATUS_INVALID_SHAPE, "OCCT framed Loft spline interpolation failed");
        }
        BRepBuilderAPI_MakeEdge edge_builder(interpolation.Curve());
        if (!edge_builder.IsDone()) {
          return error_result(STATUS_INVALID_SHAPE, "OCCT framed Loft spline edge is null");
        }
        first_edge = edge_builder.Edge();
        wire_builder.Add(first_edge);
      } else {
        return error_result(STATUS_INVALID_PARAMETER, "OCCT framed Loft profile kind is invalid");
      }
      if (!wire_builder.IsDone() || !BRepCheck_Analyzer(wire_builder.Wire()).IsValid()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT framed Loft wire is invalid");
      }
      wires.push_back(wire_builder.Wire());
      section_edges.push_back(first_edge);
    }
    if (cursor != values.size()) {
      return error_result(STATUS_INVALID_PARAMETER, "OCCT framed Loft payload has trailing values");
    }
    if (continuity > 2 || (!guide_segments.empty() && continuity == 2)) {
      return error_result(STATUS_INVALID_PARAMETER, "OCCT framed Loft continuity is unsupported");
    }
    if (!guide_segments.empty()) {
      constexpr std::size_t spatial_stride = 14;
      if (guide_segments.size() % spatial_stride != 0) {
        return error_result(STATUS_INVALID_PARAMETER, "OCCT guided Loft path payload is malformed");
      }
      BRepBuilderAPI_MakeWire spine_builder;
      gp_Pnt previous_end;
      bool has_previous = false;
      for (std::size_t offset = 0; offset < guide_segments.size(); offset += spatial_stride) {
        for (std::size_t index = 0; index < spatial_stride; ++index) {
          if (!std::isfinite(guide_segments[offset + index])) {
            return error_result(STATUS_INVALID_PARAMETER, "OCCT guided Loft path is not finite");
          }
        }
        const gp_Pnt start(
            guide_segments[offset + 1], guide_segments[offset + 2], guide_segments[offset + 3]);
        const gp_Pnt end(
            guide_segments[offset + 4], guide_segments[offset + 5], guide_segments[offset + 6]);
        if (has_previous && previous_end.Distance(start) > tolerances().linear_mm) {
          return error_result(STATUS_INVALID_PARAMETER, "OCCT guided Loft path is disconnected");
        }
        TopoDS_Edge edge;
        if (guide_segments[offset] == 10.0) {
          BRepBuilderAPI_MakeEdge builder(start, end);
          if (builder.IsDone()) edge = builder.Edge();
        } else if (guide_segments[offset] == 12.0) {
          TColgp_Array1OfPnt poles(1, 4);
          poles.SetValue(1, start);
          poles.SetValue(2, gp_Pnt(
              guide_segments[offset + 7], guide_segments[offset + 8], guide_segments[offset + 9]));
          poles.SetValue(3, gp_Pnt(
              guide_segments[offset + 10], guide_segments[offset + 11], guide_segments[offset + 12]));
          poles.SetValue(4, end);
          occ::handle<Geom_BezierCurve> curve = new Geom_BezierCurve(poles);
          BRepBuilderAPI_MakeEdge builder(curve);
          if (builder.IsDone()) edge = builder.Edge();
        } else if (guide_segments[offset] == 11.0) {
          const gp_Pnt center(
              guide_segments[offset + 7], guide_segments[offset + 8], guide_segments[offset + 9]);
          const gp_Vec normal(
              guide_segments[offset + 10], guide_segments[offset + 11], guide_segments[offset + 12]);
          const gp_Vec start_radius(center, start);
          const gp_Vec end_radius(center, end);
          if (normal.SquareMagnitude() <= tolerances().rounding * tolerances().rounding
              || start_radius.SquareMagnitude() <= tolerances().rounding * tolerances().rounding) {
            return error_result(STATUS_INVALID_PARAMETER, "OCCT guided Loft arc is degenerate");
          }
          const double signed_angle = std::atan2(
              normal.Dot(start_radius.Crossed(end_radius)), start_radius.Dot(end_radius));
          const bool clockwise = guide_segments[offset + 13] != 0.0;
          const double full_turn = 2.0 * std::acos(-1.0);
          double angle = std::fmod(clockwise ? -signed_angle : signed_angle, full_turn);
          if (angle <= 0.0) angle += full_turn;
          const gp_Vec middle_radius = start_radius.Rotated(
              gp_Ax1(center, gp_Dir(normal)), (clockwise ? -angle : angle) / 2.0);
          GC_MakeArcOfCircle arc_builder(start, center.Translated(middle_radius), end);
          if (arc_builder.IsDone()) {
            BRepBuilderAPI_MakeEdge builder(arc_builder.Value());
            if (builder.IsDone()) edge = builder.Edge();
          }
        } else {
          return error_result(STATUS_INVALID_PARAMETER, "OCCT guided Loft path kind is invalid");
        }
        if (edge.IsNull()) {
          return error_result(STATUS_INVALID_SHAPE, "OCCT guided Loft path edge is null");
        }
        spine_builder.Add(edge);
        previous_end = end;
        has_previous = true;
      }
      if (!spine_builder.IsDone() || !BRepCheck_Analyzer(spine_builder.Wire()).IsValid()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT guided Loft spine is invalid");
      }
      BRepOffsetAPI_MakePipeShell operation(spine_builder.Wire());
      operation.SetMode(false);
      operation.SetTolerance(tolerances().linear_mm, tolerances().linear_mm, tolerances().rounding);
      operation.SetForceApproxC1(continuity == 1);
      for (const TopoDS_Wire& wire : wires) operation.Add(wire, false, false);
      if (!operation.IsReady()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT guided Loft pipe is not ready");
      }
      operation.Build();
      if (!operation.IsDone() || (make_solid && !operation.MakeSolid())) {
        return error_result(
            STATUS_INVALID_SHAPE,
            make_solid
                ? "OCCT guided Loft pipe did not produce a solid"
                : "OCCT guided Loft pipe did not produce a surface");
      }
      const TopoDS_Shape result = operation.Shape();
      const std::uint32_t solid_count = count_subshapes(result, TopAbs_SOLID);
      if (result.IsNull() || !BRepCheck_Analyzer(result).IsValid()
          || (make_solid ? solid_count != 1 : solid_count != 0)) {
        return error_result(
            STATUS_INVALID_SHAPE,
            make_solid
                ? "OCCT guided Loft result is not one valid solid"
                : "OCCT guided Loft result is not a valid open surface");
      }
      std::vector<HistoryRecord> history;
      history.push_back(history_record(
          "loft.start", "first_shape", "profile.wire", result, operation.FirstShape()));
      history.push_back(history_record(
          "loft.end", "last_shape", "profile.wire", result, operation.LastShape()));
      return success_result(
          result, std::move(history), false, false, {}, !make_solid);
    }
    BRepOffsetAPI_ThruSections operation(make_solid, false, tolerances().approximation);
    operation.CheckCompatibility(true);
    operation.SetMutableInput(false);
    operation.SetContinuity(
        continuity == 0 ? GeomAbs_C0 : (continuity == 1 ? GeomAbs_C1 : GeomAbs_C2));
    for (const TopoDS_Wire& wire : wires) operation.AddWire(wire);
    operation.Build();
    if (!operation.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT framed Loft builder did not complete");
    }
    const TopoDS_Shape result = operation.Shape();
    const std::uint32_t solid_count = count_subshapes(result, TopAbs_SOLID);
    if (result.IsNull() || !BRepCheck_Analyzer(result).IsValid()
        || (make_solid ? solid_count != 1 : solid_count != 0)) {
      return error_result(
          STATUS_INVALID_SHAPE,
          make_solid
              ? "OCCT framed Loft did not produce one valid solid"
              : "OCCT framed Loft did not produce a valid open surface");
    }
    std::vector<HistoryRecord> history;
    history.push_back(history_record(
        "loft.start", "first_shape", "profile.wire", result, operation.FirstShape()));
    history.push_back(history_record(
        "loft.end", "last_shape", "profile.wire", result, operation.LastShape()));
    history.push_back(history_record(
        "loft.side", "generated_face", "profile.edge", result,
        operation.GeneratedFace(section_edges.front())));
    return success_result(
        result, std::move(history), false, false, {}, !make_solid);
  });
}

std::unique_ptr<NativeOperationResult> loft_spline_native(
    rust::Slice<const double> values) noexcept {
  return guarded([&] {
    if (values.empty() || !std::isfinite(values[0])) {
      return error_result(STATUS_INVALID_PARAMETER, "OCCT Loft payload is malformed");
    }
    const std::size_t section_count = static_cast<std::size_t>(values[0]);
    if (section_count < 2 || section_count > 16
        || values[0] != static_cast<double>(section_count)) {
      return error_result(STATUS_INVALID_PARAMETER, "OCCT Loft section count is invalid");
    }
    std::size_t cursor = 1;
    std::vector<TopoDS_Wire> wires;
    std::vector<TopoDS_Edge> section_edges;
    wires.reserve(section_count);
    section_edges.reserve(section_count);
    double previous_elevation = -std::numeric_limits<double>::infinity();
    for (std::size_t section = 0; section < section_count; ++section) {
      if (cursor + 2 > values.size() || !std::isfinite(values[cursor])) {
        return error_result(STATUS_INVALID_PARAMETER, "OCCT Loft section payload is truncated");
      }
      const std::size_t point_count = static_cast<std::size_t>(values[cursor++]);
      const double elevation = values[cursor++];
      if (point_count < 4 || point_count > 64
          || values[cursor - 2] != static_cast<double>(point_count)
          || !std::isfinite(elevation) || elevation <= previous_elevation
          || cursor + point_count * 2 > values.size()) {
        return error_result(STATUS_INVALID_PARAMETER, "OCCT Loft section is invalid");
      }
      previous_elevation = elevation;
      occ::handle<TColgp_HArray1OfPnt> points =
          new TColgp_HArray1OfPnt(1, static_cast<Standard_Integer>(point_count));
      for (std::size_t point = 0; point < point_count; ++point) {
        const double x = values[cursor++];
        const double y = values[cursor++];
        if (!std::isfinite(x) || !std::isfinite(y)) {
          return error_result(STATUS_NON_FINITE_PARAMETER, "OCCT Loft point is non-finite");
        }
        points->SetValue(
            static_cast<Standard_Integer>(point + 1), gp_Pnt(x, y, elevation));
      }
      GeomAPI_Interpolate interpolation(points, true, tolerances().rounding);
      interpolation.Perform();
      if (!interpolation.IsDone()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT spline interpolation did not complete");
      }
      BRepBuilderAPI_MakeEdge edge_builder(interpolation.Curve());
      if (!edge_builder.IsDone()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT spline edge did not complete");
      }
      const TopoDS_Edge edge = edge_builder.Edge();
      BRepBuilderAPI_MakeWire wire_builder(edge);
      if (!wire_builder.IsDone()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT spline wire did not complete");
      }
      section_edges.push_back(edge);
      wires.push_back(wire_builder.Wire());
    }
    if (cursor != values.size()) {
      return error_result(STATUS_INVALID_PARAMETER, "OCCT Loft payload has trailing values");
    }
    BRepOffsetAPI_ThruSections operation(true, false, tolerances().approximation);
    operation.CheckCompatibility(false);
    operation.SetMutableInput(false);
    for (const TopoDS_Wire& wire : wires) {
      operation.AddWire(wire);
    }
    operation.Build();
    if (!operation.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT Loft builder did not complete");
    }
    const TopoDS_Shape result = operation.Shape();
    std::vector<HistoryRecord> history;
    history.push_back(history_record(
        "loft.start", "first_shape", "profile.face", result, operation.FirstShape()));
    history.push_back(history_record(
        "loft.end", "last_shape", "profile.face", result, operation.LastShape()));
    history.push_back(history_record(
        "loft.side", "generated_face", "profile.edge.spline", result,
        operation.GeneratedFace(section_edges.front())));
    return success_result(result, std::move(history));
  });
}

std::unique_ptr<NativeOperationResult> loft_planar_profiles_native(
    rust::Slice<const double> segments,
    rust::Slice<const std::uint32_t> section_segment_counts,
    rust::Slice<const double> elevations) noexcept {
  return guarded([&] {
    if (section_segment_counts.size() < 2 || section_segment_counts.size() > 16
        || section_segment_counts.size() != elevations.size()
        || segments.empty() || segments.size() % 10 != 0) {
      return error_result(STATUS_INVALID_PARAMETER, "OCCT planar Loft payload is malformed");
    }
    std::size_t declared_segments = 0;
    double previous_elevation = -std::numeric_limits<double>::infinity();
    for (std::size_t section = 0; section < elevations.size(); ++section) {
      const std::uint32_t count = section_segment_counts[section];
      const double elevation = elevations[section];
      if (count == 0 || count > 64 || !std::isfinite(elevation)
          || std::abs(elevation) > 1000000.0 || elevation <= previous_elevation) {
        return error_result(STATUS_INVALID_PARAMETER, "OCCT planar Loft section is invalid");
      }
      declared_segments += count;
      previous_elevation = elevation;
    }
    if (declared_segments != segments.size() / 10) {
      return error_result(STATUS_INVALID_PARAMETER, "OCCT planar Loft segment counts do not match");
    }

    std::vector<TopoDS_Wire> wires;
    std::vector<TopoDS_Edge> section_edges;
    wires.reserve(section_segment_counts.size());
    section_edges.reserve(section_segment_counts.size());
    std::size_t first_segment = 0;
    const double tau = 2.0 * std::acos(-1.0);
    for (std::size_t section = 0; section < section_segment_counts.size(); ++section) {
      const std::size_t segment_count = section_segment_counts[section];
      const double elevation = elevations[section];
      BRepBuilderAPI_MakeWire wire_builder;
      bool line_only = true;
      TopoDS_Edge first_edge;
      for (std::size_t index = 0; index < segment_count; ++index) {
        const std::size_t offset = (first_segment + index) * 10;
        for (std::size_t value = 0; value < 10; ++value) {
          if (!std::isfinite(segments[offset + value])
              || std::abs(segments[offset + value]) > 1000000.0) {
            return error_result(STATUS_INVALID_PARAMETER, "OCCT planar Loft segment value is invalid");
          }
        }
        const double kind = segments[offset];
        TopoDS_Edge edge;
        if (kind == 0.0) {
          const gp_Pnt start(segments[offset + 1], segments[offset + 2], elevation);
          const gp_Pnt end(segments[offset + 3], segments[offset + 4], elevation);
          if (segments[offset + 5] != 0.0 || segments[offset + 6] != 0.0
              || segments[offset + 7] != 0.0 || segments[offset + 8] != 0.0
              || segments[offset + 9] != 0.0
              || start.Distance(end) < 0.01 || start.Distance(end) > 100000.0) {
            return error_result(STATUS_INVALID_PARAMETER, "OCCT planar Loft line is invalid");
          }
          BRepBuilderAPI_MakeEdge edge_builder(start, end);
          if (edge_builder.IsDone()) edge = edge_builder.Edge();
        } else if (kind == 1.0) {
          line_only = false;
          const gp_Pnt start(segments[offset + 1], segments[offset + 2], elevation);
          const gp_Pnt end(segments[offset + 3], segments[offset + 4], elevation);
          const double center_x = segments[offset + 5];
          const double center_y = segments[offset + 6];
          const gp_Pnt center(center_x, center_y, elevation);
          const double radius = start.Distance(center);
          const double end_radius = end.Distance(center);
          if (segments[offset + 7] != 0.0 || segments[offset + 8] != 0.0
              || (segments[offset + 9] != 0.0 && segments[offset + 9] != 1.0)
              || radius < 0.01 || radius > 100000.0
              || std::abs(radius - end_radius) > tolerances().rounding * std::max({radius, end_radius, 1.0})) {
            return error_result(STATUS_INVALID_PARAMETER, "OCCT planar Loft arc is invalid");
          }
          const double start_angle = std::atan2(start.Y() - center_y, start.X() - center_x);
          const double end_angle = std::atan2(end.Y() - center_y, end.X() - center_x);
          double sweep = end_angle - start_angle;
          if (segments[offset + 9] != 0.0) {
            while (sweep >= 0.0) sweep -= tau;
          } else {
            while (sweep <= 0.0) sweep += tau;
          }
          const double middle_angle = start_angle + sweep / 2.0;
          const gp_Pnt middle(
              center_x + radius * std::cos(middle_angle),
              center_y + radius * std::sin(middle_angle), elevation);
          GC_MakeArcOfCircle arc_builder(start, middle, end);
          if (arc_builder.IsDone()) {
            BRepBuilderAPI_MakeEdge edge_builder(arc_builder.Value());
            if (edge_builder.IsDone()) edge = edge_builder.Edge();
          }
        } else if (kind == 2.0) {
          line_only = false;
          if (segments[offset + 9] != 0.0) {
            return error_result(STATUS_INVALID_PARAMETER, "OCCT planar Loft cubic is invalid");
          }
          edge = cubic_bezier_edge(segments, offset, elevation);
        } else if (kind == 3.0 && segment_count == 1) {
          line_only = false;
          const double center_x = segments[offset + 1];
          const double center_y = segments[offset + 2];
          const double radius = segments[offset + 3];
          if (radius < 0.01 || radius > 100000.0
              || segments[offset + 4] != 0.0 || segments[offset + 5] != 0.0
              || segments[offset + 6] != 0.0 || segments[offset + 7] != 0.0
              || segments[offset + 8] != 0.0 || segments[offset + 9] != 0.0) {
            return error_result(STATUS_INVALID_PARAMETER, "OCCT planar Loft circle is invalid");
          }
          BRepBuilderAPI_MakeEdge edge_builder(gp_Circ(
              gp_Ax2(gp_Pnt(center_x, center_y, elevation), gp_Dir(0.0, 0.0, 1.0)), radius));
          if (edge_builder.IsDone()) edge = edge_builder.Edge();
        } else {
          return error_result(STATUS_INVALID_PARAMETER, "OCCT planar Loft segment kind is invalid");
        }
        if (edge.IsNull()) {
          return error_result(STATUS_INVALID_SHAPE, "OCCT planar Loft edge is null");
        }
        if (kind != 3.0) {
          const std::size_t next = (first_segment + (index + 1) % segment_count) * 10;
          if (segments[offset + 3] != segments[next + 1]
              || segments[offset + 4] != segments[next + 2]) {
            return error_result(STATUS_INVALID_PARAMETER, "OCCT planar Loft section is open");
          }
        }
        if (first_edge.IsNull()) first_edge = edge;
        wire_builder.Add(edge);
      }
      if (!wire_builder.IsDone() || (line_only && segment_count < 3)
          || !BRepCheck_Analyzer(wire_builder.Wire()).IsValid()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT planar Loft wire is invalid");
      }
      wires.push_back(wire_builder.Wire());
      section_edges.push_back(first_edge);
      first_segment += segment_count;
    }

    BRepOffsetAPI_ThruSections operation(true, false, tolerances().approximation);
    operation.CheckCompatibility(true);
    operation.SetMutableInput(false);
    for (const TopoDS_Wire& wire : wires) operation.AddWire(wire);
    operation.Build();
    if (!operation.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT planar Loft builder did not complete");
    }
    const TopoDS_Shape result = operation.Shape();
    if (result.IsNull() || !BRepCheck_Analyzer(result).IsValid()
        || count_subshapes(result, TopAbs_SOLID) != 1) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT planar Loft did not produce one valid solid");
    }
    std::vector<HistoryRecord> history;
    history.push_back(history_record(
        "loft.start", "first_shape", "profile.wire", result, operation.FirstShape()));
    history.push_back(history_record(
        "loft.end", "last_shape", "profile.wire", result, operation.LastShape()));
    history.push_back(history_record(
        "loft.side", "generated_face", "profile.edge", result,
        operation.GeneratedFace(section_edges.front())));
    return success_result(result, std::move(history));
  });
}

} // namespace ketchup::exact
