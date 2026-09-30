#include "native_common.hxx"

namespace ketchup::exact {

std::unique_ptr<NativeOperationResult> make_box_native(
    double origin_x, double origin_y, double origin_z,
    double size_x, double size_y, double size_z) noexcept {
  return guarded([&] {
    BRepPrimAPI_MakeBox operation(gp_Pnt(origin_x, origin_y, origin_z), size_x, size_y, size_z);
    const TopoDS_Shape result = operation.Shape();
    if (!operation.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT box builder did not complete");
    }
    std::vector<HistoryRecord> history;
    for (const auto& [role, face] : std::array<std::pair<const char*, TopoDS_Face>, 6>{
             std::pair{"box.face.bottom", operation.BottomFace()},
             std::pair{"box.face.top", operation.TopFace()},
             std::pair{"box.face.left", operation.LeftFace()},
             std::pair{"box.face.right", operation.RightFace()},
             std::pair{"box.face.front", operation.FrontFace()},
             std::pair{"box.face.back", operation.BackFace()},
         }) {
      history.push_back(history_record(role, "generated", role, result, face));
    }
    return success_result(result, std::move(history));
  });
}

std::unique_ptr<NativeOperationResult> offset_rectangle_native(
    double min_x, double min_y, double max_x, double max_y,
    double distance) noexcept {
  return guarded([&] {
    const gp_Pnt south_west(min_x, min_y, 0.0);
    const gp_Pnt south_east(max_x, min_y, 0.0);
    const gp_Pnt north_east(max_x, max_y, 0.0);
    const gp_Pnt north_west(min_x, max_y, 0.0);
    BRepBuilderAPI_MakeWire source_builder;
    source_builder.Add(BRepBuilderAPI_MakeEdge(south_west, south_east).Edge());
    source_builder.Add(BRepBuilderAPI_MakeEdge(south_east, north_east).Edge());
    source_builder.Add(BRepBuilderAPI_MakeEdge(north_east, north_west).Edge());
    source_builder.Add(BRepBuilderAPI_MakeEdge(north_west, south_west).Edge());
    if (!source_builder.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT offset source wire did not complete");
    }

    BRepOffsetAPI_MakeOffset operation(source_builder.Wire(), GeomAbs_Intersection, false);
    operation.Perform(distance);
    if (!operation.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT planar offset did not complete");
    }
    TopoDS_Wire offset_wire;
    for (TopExp_Explorer explorer(operation.Shape(), TopAbs_WIRE); explorer.More(); explorer.Next()) {
      if (!offset_wire.IsNull()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT planar offset produced multiple wires");
      }
      offset_wire = TopoDS::Wire(explorer.Current());
    }
    if (offset_wire.IsNull()) {
      return error_result(STATUS_NULL_RESULT, "OCCT planar offset produced no wire");
    }
    BRepBuilderAPI_MakeFace face_builder(offset_wire, true);
    if (!face_builder.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT offset face builder did not complete");
    }
    const TopoDS_Face result = face_builder.Face();
    std::vector<HistoryRecord> history;
    history.push_back(history_record(
        "planar_offset.face", "offset_generated", "profile.face", result, result));
    return success_result(result, std::move(history), false, true);
  });
}

std::unique_ptr<NativeOperationResult> offset_planar_profile_native(
    rust::Slice<const double> segments, double distance) noexcept {
  return guarded([&] {
    if (segments.size() < 20 || segments.size() % 10 != 0 || segments.size() > 640
        || !std::isfinite(distance)
        || std::abs(distance) > 100000.0) {
      return error_result(STATUS_INVALID_PARAMETER, "Planar offset payload is malformed");
    }
    BRepBuilderAPI_MakeWire source_builder;
    bool line_only = true;
    for (std::size_t offset = 0; offset < segments.size(); offset += 10) {
      for (std::size_t index = 0; index < 10; ++index) {
        if (!std::isfinite(segments[offset + index])
            || std::abs(segments[offset + index]) > 1000000.0) {
          return error_result(STATUS_INVALID_PARAMETER, "Planar offset segment value is invalid");
        }
      }
      const double kind = segments[offset];
      const gp_Pnt start(segments[offset + 1], segments[offset + 2], 0.0);
      const gp_Pnt end(segments[offset + 3], segments[offset + 4], 0.0);
      const std::size_t next = (offset + 10) % segments.size();
      if (segments[offset + 3] != segments[next + 1]
          || segments[offset + 4] != segments[next + 2]) {
        return error_result(STATUS_INVALID_PARAMETER, "Planar offset source wire is open");
      }
      TopoDS_Edge edge;
      if (kind == 0.0) {
        if (segments[offset + 5] != 0.0 || segments[offset + 6] != 0.0
            || segments[offset + 7] != 0.0 || segments[offset + 8] != 0.0
            || segments[offset + 9] != 0.0) {
          return error_result(STATUS_INVALID_PARAMETER, "Planar offset line payload is malformed");
        }
        const double length = start.Distance(end);
        if (!std::isfinite(length) || length < 0.01 || length > 100000.0) {
          return error_result(STATUS_INVALID_PARAMETER, "Planar offset line length is invalid");
        }
        BRepBuilderAPI_MakeEdge edge_builder(start, end);
        if (!edge_builder.IsDone()) {
          return error_result(STATUS_INVALID_SHAPE, "OCCT planar offset line builder did not complete");
        }
        edge = edge_builder.Edge();
      } else if (kind == 1.0) {
        line_only = false;
        if (segments[offset + 7] != 0.0 || segments[offset + 8] != 0.0
            || (segments[offset + 9] != 0.0 && segments[offset + 9] != 1.0)) {
          return error_result(STATUS_INVALID_PARAMETER, "Planar offset arc payload is malformed");
        }
        const double center_x = segments[offset + 5];
        const double center_y = segments[offset + 6];
        const bool clockwise = segments[offset + 9] != 0.0;
        const gp_Pnt center(center_x, center_y, 0.0);
        const double radius = start.Distance(center);
        const double end_radius = end.Distance(center);
        if (!std::isfinite(radius) || radius < 0.01 || radius > 100000.0
            || std::abs(radius - end_radius) > tolerances().rounding * std::max({radius, end_radius, 1.0})
            || std::abs(center_x) + radius > 1000000.0
            || std::abs(center_y) + radius > 1000000.0) {
          return error_result(STATUS_INVALID_PARAMETER, "Planar offset arc radius is invalid");
        }
        const double start_angle = std::atan2(start.Y() - center_y, start.X() - center_x);
        const double end_angle = std::atan2(end.Y() - center_y, end.X() - center_x);
        double sweep = end_angle - start_angle;
        const double tau = 2.0 * std::acos(-1.0);
        if (clockwise) {
          while (sweep >= 0.0) sweep -= tau;
        } else {
          while (sweep <= 0.0) sweep += tau;
        }
        const double middle_angle = start_angle + sweep / 2.0;
        const gp_Pnt middle(
            center_x + radius * std::cos(middle_angle),
            center_y + radius * std::sin(middle_angle),
            0.0);
        GC_MakeArcOfCircle arc_builder(start, middle, end);
        if (!arc_builder.IsDone()) {
          return error_result(STATUS_INVALID_SHAPE, "OCCT planar offset arc builder did not complete");
        }
        BRepBuilderAPI_MakeEdge edge_builder(arc_builder.Value());
        if (!edge_builder.IsDone()) {
          return error_result(STATUS_INVALID_SHAPE, "OCCT planar offset arc edge did not complete");
        }
        edge = edge_builder.Edge();
      } else if (kind == 2.0) {
        line_only = false;
        if (segments[offset + 9] != 0.0) {
          return error_result(STATUS_INVALID_PARAMETER, "Planar offset cubic payload is malformed");
        }
        const gp_Pnt control_1(segments[offset + 5], segments[offset + 6], 0.0);
        const gp_Pnt control_2(segments[offset + 7], segments[offset + 8], 0.0);
        const double control_polygon_length =
            start.Distance(control_1) + control_1.Distance(control_2) + control_2.Distance(end);
        if (!std::isfinite(control_polygon_length) || control_polygon_length < 0.01
            || control_polygon_length > 100000.0) {
          return error_result(STATUS_INVALID_PARAMETER, "Planar offset cubic length is invalid");
        }
        edge = cubic_bezier_edge(segments, offset, 0.0);
      } else {
        return error_result(STATUS_INVALID_PARAMETER, "Planar offset segment kind is unsupported");
      }
      if (edge.IsNull()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT planar offset source edge is null");
      }
      source_builder.Add(edge);
    }
    if (!source_builder.IsDone() || (line_only && segments.size() < 30)) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT planar offset source wire did not complete");
    }
    BRepBuilderAPI_MakeFace source_face_builder(source_builder.Wire(), true);
    if (!source_face_builder.IsDone()
        || !BRepCheck_Analyzer(source_face_builder.Face()).IsValid()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT planar offset source face is invalid");
    }
    if (distance == 0.0) {
      const TopoDS_Face result = source_face_builder.Face();
      std::vector<HistoryRecord> history;
      history.push_back(history_record(
          "planar_surface.face", "generated", "profile.face", result, result));
      return success_result(result, std::move(history), false, true);
    }

    BRepOffsetAPI_MakeOffset operation(source_builder.Wire(), GeomAbs_Intersection, false);
    operation.Perform(distance);
    if (!operation.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT planar offset did not complete");
    }
    TopoDS_Wire offset_wire;
    for (TopExp_Explorer explorer(operation.Shape(), TopAbs_WIRE); explorer.More(); explorer.Next()) {
      if (!offset_wire.IsNull()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT planar offset produced multiple wires");
      }
      offset_wire = TopoDS::Wire(explorer.Current());
    }
    if (offset_wire.IsNull()) {
      return error_result(STATUS_NULL_RESULT, "OCCT planar offset produced no wire");
    }
    BRepBuilderAPI_MakeFace face_builder(offset_wire, true);
    if (!face_builder.IsDone() || !BRepCheck_Analyzer(face_builder.Face()).IsValid()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT planar offset face is invalid");
    }
    const TopoDS_Face result = face_builder.Face();
    std::vector<HistoryRecord> history;
    history.push_back(history_record(
        "planar_offset.face", "offset_generated", "profile.face", result, result));
    return success_result(result, std::move(history), false, true);
  });
}

std::unique_ptr<NativeOperationResult> planar_surface_profile_native(
    rust::Slice<const double> segments) noexcept {
  return offset_planar_profile_native(segments, 0.0);
}

std::unique_ptr<NativeOperationResult> trim_surface_native(
    const NativeOperationResult& target,
    const NativeOperationResult& cutter) noexcept {
  return guarded([&] {
    if (!target.valid() || !cutter.valid()
        || target.impl().shape.IsNull() || cutter.impl().shape.IsNull()
        || target.impl().summary.solid_count != 0 || cutter.impl().summary.solid_count != 0
        || target.impl().summary.face_count == 0 || cutter.impl().summary.face_count == 0) {
      return error_result(STATUS_INVALID_PARAMETER, "Surface trim requires valid non-solid target and cutter surfaces");
    }
    BRepAlgoAPI_Common operation(target.impl().shape, cutter.impl().shape);
    operation.SetNonDestructive(true);
    operation.Build();
    if (!operation.IsDone() || operation.HasErrors() || operation.HasWarnings()
        || operation.Shape().IsNull()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT surface trim did not produce an unambiguous intersection");
    }
    const TopoDS_Shape result = operation.Shape();
    if (!BRepCheck_Analyzer(result, true).IsValid()
        || count_subshapes(result, TopAbs_SOLID) != 0
        || count_subshapes(result, TopAbs_FACE) != 1
        || count_subshapes(result, TopAbs_WIRE) != 1) {
      return error_result(STATUS_INVALID_SHAPE, "Surface trim must produce one valid bounded non-solid face");
    }
    GProp_GProps target_properties;
    GProp_GProps result_properties;
    BRepGProp::SurfaceProperties(target.impl().shape, target_properties);
    BRepGProp::SurfaceProperties(result, result_properties);
    const double target_area = target_properties.Mass();
    const double result_area = result_properties.Mass();
    const double tolerance = tolerances().rounding * std::max({target_area, result_area, 1.0});
    if (!std::isfinite(target_area) || !std::isfinite(result_area)
        || result_area <= tolerance || result_area >= target_area - tolerance) {
      return error_result(STATUS_NO_GEOMETRIC_CHANGE, "Surface trim requires a finite proper subset of the target");
    }
    std::vector<HistoryRecord> history;
    append_propagated_history(history, operation, result, target.impl());
    history.push_back(history_record(
        "surface_trim.face", "trimmed", "surface.target", result, result));
    return success_result(result, std::move(history), false, true);
  });
}

std::unique_ptr<NativeOperationResult> extend_planar_surface_native(
    const NativeOperationResult& target, double distance) noexcept {
  return guarded([&] {
    if (!target.valid() || target.impl().shape.IsNull()
        || target.impl().summary.solid_count != 0
        || target.impl().summary.face_count != 1
        || target.impl().summary.wire_count != 1
        || !std::isfinite(distance) || distance < 0.01 || distance > 100000.0) {
      return error_result(STATUS_INVALID_PARAMETER, "Planar surface extend payload is outside the bounded envelope");
    }
    const TopoDS_Face source = face_at_ordinal(target.impl().shape, 0);
    if (source.IsNull()) {
      return error_result(STATUS_INVALID_SHAPE, "Planar surface extend source face is absent");
    }
    const BRepAdaptor_Surface surface(source);
    if (surface.GetType() != GeomAbs_Plane) {
      return error_result(STATUS_INVALID_PARAMETER, "Surface extend currently requires one planar face");
    }
    BRepOffsetAPI_MakeOffset operation(source, GeomAbs_Intersection, false);
    operation.Perform(distance);
    if (!operation.IsDone() || operation.Shape().IsNull()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT planar surface boundary extension did not complete");
    }
    TopoDS_Wire offset_wire;
    for (TopExp_Explorer wires(operation.Shape(), TopAbs_WIRE); wires.More(); wires.Next()) {
      if (!offset_wire.IsNull()) {
        return error_result(STATUS_INVALID_SHAPE, "Surface extend produced an ambiguous multi-wire boundary");
      }
      offset_wire = TopoDS::Wire(wires.Current());
    }
    if (offset_wire.IsNull()) {
      return error_result(STATUS_NULL_RESULT, "Surface extend produced no bounded wire");
    }
    BRepBuilderAPI_MakeFace face_builder(surface.Plane(), offset_wire, true);
    if (!face_builder.IsDone() || face_builder.Face().IsNull()
        || !BRepCheck_Analyzer(face_builder.Face(), true).IsValid()) {
      return error_result(STATUS_INVALID_SHAPE, "Surface extend produced an invalid planar face");
    }
    const TopoDS_Face result = face_builder.Face();
    GProp_GProps source_properties;
    GProp_GProps result_properties;
    BRepGProp::SurfaceProperties(source, source_properties);
    BRepGProp::SurfaceProperties(result, result_properties);
    const double source_area = source_properties.Mass();
    const double result_area = result_properties.Mass();
    const double tolerance = tolerances().rounding * std::max({source_area, result_area, 1.0});
    if (!std::isfinite(source_area) || !std::isfinite(result_area)
        || result_area <= source_area + tolerance) {
      return error_result(STATUS_NO_GEOMETRIC_CHANGE, "Surface extend did not increase the bounded face area");
    }
    std::vector<HistoryRecord> history;
    history.push_back(history_record(
        "surface_extend.face", "extended", "surface.target", result, result));
    return success_result(result, std::move(history), false, true);
  });
}

std::unique_ptr<NativeOperationResult> combine_surfaces_native(
    const NativeOperationResult& base,
    const NativeOperationResult& added) noexcept {
  return guarded([&] {
    if (!base.valid() || !added.valid()
        || base.impl().shape.IsNull() || added.impl().shape.IsNull()
        || base.impl().summary.solid_count != 0 || added.impl().summary.solid_count != 0
        || base.impl().summary.face_count == 0 || added.impl().summary.face_count == 0) {
      return error_result(
          STATUS_INVALID_PARAMETER,
          "Surface compound requires valid non-solid surface inputs");
    }
    const std::uint64_t face_count = static_cast<std::uint64_t>(base.impl().summary.face_count)
        + static_cast<std::uint64_t>(added.impl().summary.face_count);
    if (face_count > 256) {
      return error_result(STATUS_INVALID_PARAMETER, "Surface compound exceeds the face limit");
    }
    BRep_Builder builder;
    TopoDS_Compound compound;
    builder.MakeCompound(compound);
    builder.Add(compound, base.impl().shape);
    builder.Add(compound, added.impl().shape);
    return success_result(compound, {}, false, false, {}, true);
  });
}

std::unique_ptr<NativeOperationResult> knit_surface_compound_native(
    const NativeOperationResult& surfaces,
    double tolerance, bool make_solid) noexcept {
  return guarded([&] {
    if (!surfaces.valid() || surfaces.impl().shape.IsNull()
        || surfaces.impl().summary.solid_count != 0
        || surfaces.impl().summary.face_count < 2
        || surfaces.impl().summary.face_count > 256
        || !std::isfinite(tolerance)
        || tolerance < tolerances().linear_mm || tolerance > 10.0) {
      return error_result(
          STATUS_INVALID_PARAMETER,
          "Surface knit requires 2..256 non-solid faces and a tolerance from the linear tolerance to 10 mm");
    }

    BRepBuilderAPI_Sewing sewing(tolerance, true, true, true, false);
    for (TopExp_Explorer faces(surfaces.impl().shape, TopAbs_FACE); faces.More(); faces.Next()) {
      sewing.Add(faces.Current());
    }
    sewing.Perform();
    const TopoDS_Shape sewed = sewing.SewedShape();
    if (sewed.IsNull() || !BRepCheck_Analyzer(sewed, true).IsValid()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT surface sewing produced no valid result");
    }
    if (sewing.NbMultipleEdges() != 0) {
      return error_result(
          STATUS_INVALID_SHAPE,
          "Surface knit is non-manifold: multiple_edges="
              + std::to_string(sewing.NbMultipleEdges()));
    }
    if (sewing.NbContigousEdges() == 0) {
      return error_result(
          STATUS_NO_GEOMETRIC_CHANGE,
          "Surface knit found no shared boundaries within tolerance="
              + std::to_string(tolerance));
    }
    if (count_subshapes(sewed, TopAbs_SHELL) != 1) {
      return error_result(
          STATUS_INVALID_SHAPE,
          "Surface knit must produce one connected shell; shells="
              + std::to_string(count_subshapes(sewed, TopAbs_SHELL)));
    }

    TopoDS_Shape result = sewed;
    if (make_solid) {
      if (sewing.NbFreeEdges() != 0) {
        return error_result(
            STATUS_INVALID_SHAPE,
            "Surface knit cannot create a solid from an open shell: free_edges="
                + std::to_string(sewing.NbFreeEdges())
                + ", tolerance_mm=" + std::to_string(tolerance));
      }
      TopoDS_Shell shell;
      for (TopExp_Explorer shells(sewed, TopAbs_SHELL); shells.More(); shells.Next()) {
        shell = TopoDS::Shell(shells.Current());
      }
      if (shell.IsNull()) {
        return error_result(STATUS_INVALID_SHAPE, "Surface knit produced no shell for solid conversion");
      }
      BRepBuilderAPI_MakeSolid solid_builder(shell);
      if (!solid_builder.IsDone()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT surface knit solid builder did not complete");
      }
      TopoDS_Solid solid = solid_builder.Solid();
      if (solid.IsNull() || !BRepLib::OrientClosedSolid(solid)
          || !BRepCheck_Analyzer(solid, true).IsValid()
          || count_subshapes(solid, TopAbs_SOLID) != 1) {
        return error_result(
            STATUS_INVALID_SHAPE,
            "Surface knit shell is not a valid closed watertight solid");
      }
      result = solid;
    }

    std::vector<HistoryRecord> history;
    for (TopExp_Explorer faces(result, TopAbs_FACE); faces.More(); faces.Next()) {
      history.push_back(history_record(
          "surface_knit.face", "sewn", "surface.input.face", result, faces.Current()));
    }
    return success_result(result, std::move(history), false, false, {}, !make_solid);
  });
}

std::unique_ptr<NativeOperationResult> thicken_surface_native(
    const NativeOperationResult& surface,
    double thickness, std::uint8_t direction) noexcept {
  return guarded([&] {
    if (!surface.valid() || surface.impl().shape.IsNull()
        || surface.impl().summary.solid_count != 0
        || surface.impl().summary.face_count == 0
        || surface.impl().summary.face_count > 256
        || !std::isfinite(thickness) || thickness < 0.01
        || thickness > 100000.0 || direction > 2) {
      return error_result(
          STATUS_INVALID_PARAMETER,
          "Surface thicken requires a valid non-solid surface, bounded positive thickness, and explicit direction");
    }

    const auto build_half = [&](BRepOffsetAPI_MakeThickSolid& operation, double offset) {
      operation.MakeThickSolidBySimple(surface.impl().shape, offset);
      return operation.IsDone() && !operation.Shape().IsNull();
    };
    const auto oriented_solid = [&](const TopoDS_Shape& candidate) {
      TopoDS_Solid solid;
      if (!candidate.IsNull() && count_subshapes(candidate, TopAbs_SOLID) == 1) {
        for (TopExp_Explorer solids(candidate, TopAbs_SOLID); solids.More(); solids.Next()) {
          solid = TopoDS::Solid(solids.Current());
        }
      }
      if (solid.IsNull()
          || count_subshapes(candidate, TopAbs_FACE) != count_subshapes(solid, TopAbs_FACE)
          || count_subshapes(candidate, TopAbs_EDGE) != count_subshapes(solid, TopAbs_EDGE)
          || !BRepLib::OrientClosedSolid(solid)
          || !BRepCheck_Analyzer(solid, true).IsValid()) {
        return TopoDS_Solid{};
      }
      GProp_GProps properties;
      BRepGProp::VolumeProperties(solid, properties);
      if (!std::isfinite(properties.Mass()) || properties.Mass() <= tolerances().rounding) {
        return TopoDS_Solid{};
      }
      return solid;
    };
    const auto result_history = [&](const TopoDS_Shape& result) {
      std::vector<HistoryRecord> history;
      for (TopExp_Explorer faces(result, TopAbs_FACE); faces.More(); faces.Next()) {
        history.push_back(history_record(
            "surface_thicken.face", "thickened", "surface.target", result, faces.Current()));
      }
      return history;
    };

    if (direction != 2) {
      BRepOffsetAPI_MakeThickSolid operation;
      const double offset = direction == 0 ? -thickness : thickness;
      if (!build_half(operation, offset)) {
        return error_result(
            STATUS_INVALID_SHAPE,
            "OCCT surface thicken failed or produced a self-intersecting/non-solid result");
      }
      const TopoDS_Solid result = oriented_solid(operation.Shape());
      if (result.IsNull()) {
        return error_result(
            STATUS_INVALID_SHAPE,
            "OCCT surface thicken did not produce one orientable positive-volume solid");
      }
      return success_result(result, result_history(result));
    }

    BRepOffsetAPI_MakeThickSolid inward;
    BRepOffsetAPI_MakeThickSolid outward;
    if (!build_half(inward, -thickness * 0.5)
        || !build_half(outward, thickness * 0.5)) {
      return error_result(
          STATUS_INVALID_SHAPE,
          "OCCT symmetric surface thicken half failed or self-intersected");
    }
    const TopoDS_Solid inward_solid = oriented_solid(inward.Shape());
    const TopoDS_Solid outward_solid = oriented_solid(outward.Shape());
    if (inward_solid.IsNull() || outward_solid.IsNull()) {
      return error_result(
          STATUS_INVALID_SHAPE,
          "OCCT symmetric surface thicken half was not an orientable positive-volume solid");
    }
    BRepAlgoAPI_Fuse fusion(inward_solid, outward_solid);
    fusion.Build();
    if (!fusion.IsDone() || fusion.HasErrors() || fusion.Shape().IsNull()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT symmetric surface thicken fuse failed");
    }
    fusion.SimplifyResult(true, true);
    const TopoDS_Solid result = oriented_solid(fusion.Shape());
    if (result.IsNull()) {
      return error_result(
          STATUS_INVALID_SHAPE,
          "Symmetric surface thicken did not produce one valid positive-volume solid");
    }
    return success_result(result, result_history(result));
  });
}

std::unique_ptr<NativeOperationResult> offset_planar_region_native(
    rust::Slice<const double> segments,
    rust::Slice<const std::uint32_t> loop_segment_counts,
    double distance) noexcept {
  return guarded([&] {
    if (loop_segment_counts.size() < 2 || loop_segment_counts.size() > 65
        || segments.empty() || segments.size() % 10 != 0
        || !std::isfinite(distance) || std::abs(distance) < 0.01
        || std::abs(distance) > 100000.0) {
      return error_result(STATUS_INVALID_PARAMETER, "Planar region offset payload is malformed");
    }
    std::size_t declared_segments = 0;
    for (const std::uint32_t count : loop_segment_counts) {
      if (count == 0 || count > 64) {
        return error_result(STATUS_INVALID_PARAMETER, "Planar region offset loop count is invalid");
      }
      declared_segments += count;
    }
    if (declared_segments != segments.size() / 10 || declared_segments > 4096) {
      return error_result(STATUS_INVALID_PARAMETER, "Planar region offset segment counts do not match");
    }

    auto build_wire = [&](std::size_t first_segment, std::size_t segment_count) {
      BRepBuilderAPI_MakeWire wire_builder;
      bool line_only = true;
      for (std::size_t index = 0; index < segment_count; ++index) {
        const std::size_t offset = (first_segment + index) * 10;
        for (std::size_t value = 0; value < 10; ++value) {
          if (!std::isfinite(segments[offset + value])
              || std::abs(segments[offset + value]) > 1000000.0) {
            return TopoDS_Wire{};
          }
        }
        const double kind = segments[offset];
        TopoDS_Edge edge;
        if (kind == 0.0) {
          const gp_Pnt start(segments[offset + 1], segments[offset + 2], 0.0);
          const gp_Pnt end(segments[offset + 3], segments[offset + 4], 0.0);
          if (segments[offset + 5] != 0.0 || segments[offset + 6] != 0.0
              || segments[offset + 7] != 0.0 || segments[offset + 8] != 0.0
              || segments[offset + 9] != 0.0
              || start.Distance(end) < 0.01 || start.Distance(end) > 100000.0) {
            return TopoDS_Wire{};
          }
          BRepBuilderAPI_MakeEdge edge_builder(start, end);
          if (!edge_builder.IsDone()) {
            return TopoDS_Wire{};
          }
          edge = edge_builder.Edge();
        } else if (kind == 1.0) {
          line_only = false;
          const gp_Pnt start(segments[offset + 1], segments[offset + 2], 0.0);
          const gp_Pnt end(segments[offset + 3], segments[offset + 4], 0.0);
          const double center_x = segments[offset + 5];
          const double center_y = segments[offset + 6];
          const bool clockwise = segments[offset + 9] != 0.0;
          const gp_Pnt center(center_x, center_y, 0.0);
          const double radius = start.Distance(center);
          const double end_radius = end.Distance(center);
          if (segments[offset + 7] != 0.0 || segments[offset + 8] != 0.0
              || (segments[offset + 9] != 0.0 && segments[offset + 9] != 1.0)
              || radius < 0.01 || radius > 100000.0
              || std::abs(radius - end_radius) > tolerances().rounding * std::max({radius, end_radius, 1.0})
              || std::abs(center_x) + radius > 1000000.0
              || std::abs(center_y) + radius > 1000000.0) {
            return TopoDS_Wire{};
          }
          const double start_angle = std::atan2(start.Y() - center_y, start.X() - center_x);
          const double end_angle = std::atan2(end.Y() - center_y, end.X() - center_x);
          double sweep = end_angle - start_angle;
          const double tau = 2.0 * std::acos(-1.0);
          if (clockwise) {
            while (sweep >= 0.0) sweep -= tau;
          } else {
            while (sweep <= 0.0) sweep += tau;
          }
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
        } else if (kind == 2.0) {
          line_only = false;
          const gp_Pnt start(segments[offset + 1], segments[offset + 2], 0.0);
          const gp_Pnt end(segments[offset + 3], segments[offset + 4], 0.0);
          const gp_Pnt control_1(segments[offset + 5], segments[offset + 6], 0.0);
          const gp_Pnt control_2(segments[offset + 7], segments[offset + 8], 0.0);
          const double control_polygon_length =
              start.Distance(control_1) + control_1.Distance(control_2) + control_2.Distance(end);
          if (segments[offset + 9] != 0.0 || control_polygon_length < 0.01
              || control_polygon_length > 100000.0) {
            return TopoDS_Wire{};
          }
          edge = cubic_bezier_edge(segments, offset, 0.0);
        } else if (kind == 3.0 && segment_count == 1) {
          line_only = false;
          const double center_x = segments[offset + 1];
          const double center_y = segments[offset + 2];
          const double radius = segments[offset + 3];
          if (radius < 0.01 || radius > 100000.0
              || std::abs(center_x) + radius > 1000000.0
              || std::abs(center_y) + radius > 1000000.0
              || segments[offset + 4] != 0.0 || segments[offset + 5] != 0.0
              || segments[offset + 6] != 0.0 || segments[offset + 7] != 0.0
              || segments[offset + 8] != 0.0
              || (segments[offset + 9] != 0.0 && segments[offset + 9] != 1.0)) {
            return TopoDS_Wire{};
          }
          BRepBuilderAPI_MakeEdge edge_builder(
              gp_Circ(gp_Ax2(gp_Pnt(center_x, center_y, 0.0), gp_Dir(0.0, 0.0, 1.0)), radius));
          if (!edge_builder.IsDone()) {
            return TopoDS_Wire{};
          }
          edge = edge_builder.Edge();
          if (segments[offset + 9] != 0.0) {
            edge.Reverse();
          }
        } else {
          return TopoDS_Wire{};
        }
        if (edge.IsNull()) {
          return TopoDS_Wire{};
        }
        if (kind != 3.0) {
          const std::size_t next = (first_segment + (index + 1) % segment_count) * 10;
          if (segments[offset + 3] != segments[next + 1]
              || segments[offset + 4] != segments[next + 2]) {
            return TopoDS_Wire{};
          }
        }
        wire_builder.Add(edge);
      }
      if (!wire_builder.IsDone() || (line_only && segment_count < 3)) {
        return TopoDS_Wire{};
      }
      return wire_builder.Wire();
    };

    std::vector<TopoDS_Wire> source_wires;
    source_wires.reserve(loop_segment_counts.size());
    std::size_t first_segment = 0;
    for (const std::uint32_t count : loop_segment_counts) {
      TopoDS_Wire wire = build_wire(first_segment, count);
      if (wire.IsNull()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT planar region offset source wire is invalid");
      }
      source_wires.push_back(wire);
      first_segment += count;
    }
    BRepBuilderAPI_MakeFace source_face_builder(source_wires[0], true);
    for (std::size_t index = 1; index < source_wires.size(); ++index) {
      source_face_builder.Add(source_wires[index]);
    }
    source_face_builder.Build();
    if (!source_face_builder.IsDone()
        || !BRepCheck_Analyzer(source_face_builder.Face()).IsValid()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT planar region offset source face is invalid");
    }
    const TopoDS_Face source_face = source_face_builder.Face();

    std::vector<TopoDS_Wire> offset_wires;
    offset_wires.reserve(source_wires.size());
    std::size_t source_segment = 0;
    for (std::size_t index = 0; index < source_wires.size(); ++index) {
      const bool circle_hole = index != 0
          && loop_segment_counts[index] == 1
          && segments[source_segment * 10] == 3.0;
      TopoDS_Wire operation_wire = source_wires[index];
      if (circle_hole) {
        const std::size_t offset = source_segment * 10;
        BRepBuilderAPI_MakeEdge edge_builder(gp_Circ(gp_Ax2(
            gp_Pnt(segments[offset + 1], segments[offset + 2], 0.0),
            gp_Dir(0.0, 0.0, 1.0)), segments[offset + 3]));
        if (!edge_builder.IsDone()) {
          return error_result(STATUS_INVALID_SHAPE, "OCCT planar region circle hole is invalid");
        }
        BRepBuilderAPI_MakeWire wire_builder(edge_builder.Edge());
        if (!wire_builder.IsDone()) {
          return error_result(STATUS_INVALID_SHAPE, "OCCT planar region circle hole is invalid");
        }
        operation_wire = wire_builder.Wire();
      }
      BRepOffsetAPI_MakeOffset operation(operation_wire, GeomAbs_Intersection, false);
      operation.Perform(index == 0 ? distance : -distance);
      if (!operation.IsDone()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT planar region offset did not complete");
      }
      TopoDS_Wire offset_wire;
      for (TopExp_Explorer explorer(operation.Shape(), TopAbs_WIRE); explorer.More(); explorer.Next()) {
        if (!offset_wire.IsNull()) {
          return error_result(STATUS_INVALID_SHAPE, "OCCT planar region offset split a loop");
        }
        offset_wire = TopoDS::Wire(explorer.Current());
      }
      if (offset_wire.IsNull()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT planar region offset collapsed a loop");
      }
      if (circle_hole) {
        offset_wire.Reverse();
      }
      source_segment += loop_segment_counts[index];
      BRepBuilderAPI_MakeFace source_loop_face(source_wires[index], true);
      BRepBuilderAPI_MakeFace offset_loop_face(offset_wire, true);
      if (!source_loop_face.IsDone() || !offset_loop_face.IsDone()
          || !BRepCheck_Analyzer(offset_loop_face.Face()).IsValid()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT planar region offset loop face is invalid");
      }
      GProp_GProps source_loop_properties;
      GProp_GProps offset_loop_properties;
      BRepGProp::SurfaceProperties(source_loop_face.Face(), source_loop_properties);
      BRepGProp::SurfaceProperties(offset_loop_face.Face(), offset_loop_properties);
      const double source_loop_area = source_loop_properties.Mass();
      const double offset_loop_area = offset_loop_properties.Mass();
      const double loop_area_tolerance =
          tolerances().rounding * std::max({source_loop_area, offset_loop_area, 1.0});
      const double loop_distance = index == 0 ? distance : -distance;
      if (!std::isfinite(source_loop_area) || !std::isfinite(offset_loop_area)
          || source_loop_area <= 0.0001 || offset_loop_area <= 0.0001
          || (loop_distance > 0.0 && offset_loop_area <= source_loop_area + loop_area_tolerance)
          || (loop_distance < 0.0 && offset_loop_area >= source_loop_area - loop_area_tolerance)) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT planar region offset loop violates signed semantics");
      }
      offset_wires.push_back(offset_wire);
    }

    BRepBuilderAPI_MakeFace result_builder(offset_wires[0], true);
    for (std::size_t index = 1; index < offset_wires.size(); ++index) {
      result_builder.Add(offset_wires[index]);
    }
    result_builder.Build();
    if (!result_builder.IsDone() || !BRepCheck_Analyzer(result_builder.Face()).IsValid()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT planar region offset face is invalid");
    }
    const TopoDS_Face result = result_builder.Face();
    TopTools_IndexedMapOfShape result_wires;
    TopExp::MapShapes(result, TopAbs_WIRE, result_wires);
    if (result_wires.Extent() != static_cast<Standard_Integer>(source_wires.size())) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT planar region offset changed loop topology");
    }
    GProp_GProps source_properties;
    GProp_GProps result_properties;
    BRepGProp::SurfaceProperties(source_face, source_properties);
    BRepGProp::SurfaceProperties(result, result_properties);
    const double source_area = source_properties.Mass();
    const double result_area = result_properties.Mass();
    const double area_tolerance = tolerances().rounding * std::max({source_area, result_area, 1.0});
    if (!std::isfinite(source_area) || !std::isfinite(result_area)
        || source_area <= 0.0001 || result_area <= 0.0001
        || (distance > 0.0 && result_area <= source_area + area_tolerance)
        || (distance < 0.0 && result_area >= source_area - area_tolerance)) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT planar region offset area violates signed semantics");
    }
    std::vector<HistoryRecord> history;
    history.push_back(history_record(
        "planar_offset.face", "offset_generated", "profile.face", result, result));
    return success_result(result, std::move(history), false, true);
  });
}

std::unique_ptr<NativeOperationResult> offset_planar_circle_native(
    double center_x, double center_y, double radius, double distance) noexcept {
  return guarded([&] {
    const double output_radius = radius + distance;
    if (!std::isfinite(center_x) || !std::isfinite(center_y)
        || !std::isfinite(radius) || !std::isfinite(distance)
        || radius < 0.01 || radius > 100000.0
        || std::abs(distance) < 0.01 || std::abs(distance) > 100000.0
        || output_radius < 0.01 || output_radius > 100000.0
        || std::abs(center_x) + radius > 1000000.0
        || std::abs(center_y) + radius > 1000000.0
        || std::abs(center_x) + output_radius > 1000000.0
        || std::abs(center_y) + output_radius > 1000000.0) {
      return error_result(STATUS_INVALID_PARAMETER, "Planar circle offset is outside the bounded envelope");
    }
    BRepBuilderAPI_MakeEdge edge_builder(
        gp_Circ(gp_Ax2(gp_Pnt(center_x, center_y, 0.0), gp_Dir(0.0, 0.0, 1.0)), radius));
    if (!edge_builder.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT planar circle edge did not complete");
    }
    BRepBuilderAPI_MakeWire source_builder(edge_builder.Edge());
    if (!source_builder.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT planar circle wire did not complete");
    }
    BRepBuilderAPI_MakeFace source_face_builder(source_builder.Wire(), true);
    if (!source_face_builder.IsDone()
        || !BRepCheck_Analyzer(source_face_builder.Face()).IsValid()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT planar circle source face is invalid");
    }

    BRepOffsetAPI_MakeOffset operation(source_builder.Wire(), GeomAbs_Intersection, false);
    operation.Perform(distance);
    if (!operation.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT planar circle offset did not complete");
    }
    TopoDS_Wire offset_wire;
    for (TopExp_Explorer explorer(operation.Shape(), TopAbs_WIRE); explorer.More(); explorer.Next()) {
      if (!offset_wire.IsNull()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT planar circle offset produced multiple wires");
      }
      offset_wire = TopoDS::Wire(explorer.Current());
    }
    if (offset_wire.IsNull()) {
      return error_result(STATUS_NULL_RESULT, "OCCT planar circle offset produced no wire");
    }
    BRepBuilderAPI_MakeFace face_builder(offset_wire, true);
    if (!face_builder.IsDone() || !BRepCheck_Analyzer(face_builder.Face()).IsValid()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT planar circle offset face is invalid");
    }
    const TopoDS_Face result = face_builder.Face();
    std::vector<HistoryRecord> history;
    history.push_back(history_record(
        "planar_offset.face", "offset_generated", "profile.face", result, result));
    return success_result(result, std::move(history), false, true);
  });
}

} // namespace ketchup::exact
