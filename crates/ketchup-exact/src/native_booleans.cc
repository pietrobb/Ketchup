#include "native_common.hxx"

namespace ketchup::exact {

std::unique_ptr<NativeOperationResult> transform_body_native(
    const NativeOperationResult& body, rust::Slice<const double> matrix) noexcept {
  return guarded([&] {
    if (!body.valid() || body.impl().shape.IsNull() || matrix.size() != 16) {
      return error_result(STATUS_INVALID_PARAMETER, "Exact body or affine transform is unavailable");
    }
    for (const double value : matrix) {
      if (!std::isfinite(value)) {
        return error_result(STATUS_NON_FINITE_PARAMETER, "Exact body transform is non-finite");
      }
    }
    const auto dot_column = [&](std::size_t left, std::size_t right) {
      return matrix[left] * matrix[right]
          + matrix[4 + left] * matrix[4 + right]
          + matrix[8 + left] * matrix[8 + right];
    };
    const bool rigid =
        std::abs(dot_column(0, 0) - 1.0) <= tolerances().rounding
        && std::abs(dot_column(1, 1) - 1.0) <= tolerances().rounding
        && std::abs(dot_column(2, 2) - 1.0) <= tolerances().rounding
        && std::abs(dot_column(0, 1)) <= tolerances().rounding
        && std::abs(dot_column(0, 2)) <= tolerances().rounding
        && std::abs(dot_column(1, 2)) <= tolerances().rounding;
    const std::uint32_t source_solids = count_subshapes(body.impl().shape, TopAbs_SOLID);
    const bool source_is_planar_face = source_solids == 0
        && count_subshapes(body.impl().shape, TopAbs_FACE) == 1;
    TopoDS_Shape result;
    if (rigid) {
      gp_Trsf transform;
      transform.SetValues(
          matrix[0], matrix[1], matrix[2], matrix[3],
          matrix[4], matrix[5], matrix[6], matrix[7],
          matrix[8], matrix[9], matrix[10], matrix[11]);
      if (source_is_planar_face) {
        result = body.impl().shape.Moved(TopLoc_Location(transform));
      } else {
        BRepBuilderAPI_Transform operation(body.impl().shape, transform, true);
        operation.Build();
        if (!operation.IsDone()) {
          return error_result(STATUS_INVALID_SHAPE, "OCCT rigid body transform did not complete");
        }
        result = operation.Shape();
      }
    } else {
      gp_GTrsf transform;
      for (Standard_Integer row = 1; row <= 3; ++row) {
        for (Standard_Integer column = 1; column <= 4; ++column) {
          transform.SetValue(row, column, matrix[static_cast<std::size_t>((row - 1) * 4 + column - 1)]);
        }
      }
      BRepBuilderAPI_GTransform operation(body.impl().shape, transform, true);
      operation.Build();
      if (!operation.IsDone()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT affine body transform did not complete");
      }
      result = operation.Shape();
    }
    return success_result(result, {}, source_solids >= 2, source_is_planar_face);
  });
}

std::unique_ptr<NativeOperationResult> combine_bodies_native(
    const NativeOperationResult& base, const NativeOperationResult& added) noexcept {
  return guarded([&] {
    if (!base.valid() || !added.valid() || base.impl().shape.IsNull() || added.impl().shape.IsNull()) {
      return error_result(STATUS_INVALID_PARAMETER, "Exact assembly input body is unavailable");
    }
    BRep_Builder builder;
    TopoDS_Compound compound;
    builder.MakeCompound(compound);
    builder.Add(compound, base.impl().shape);
    builder.Add(compound, added.impl().shape);
    return success_result(compound, {}, true);
  });
}

std::unique_ptr<NativeOperationResult> trim_body_by_plane_native(
    const NativeOperationResult& body,
    double origin_x, double origin_y, double origin_z,
    double normal_x, double normal_y, double normal_z,
    double keep_x, double keep_y, double keep_z) noexcept {
  return guarded([&] {
    const std::array<double, 9> values = {
        origin_x, origin_y, origin_z, normal_x, normal_y, normal_z, keep_x, keep_y, keep_z};
    if (!body.valid() || body.impl().shape.IsNull()
        || !std::all_of(values.begin(), values.end(), [](double value) { return std::isfinite(value); })) {
      return error_result(STATUS_INVALID_PARAMETER, "Plane trim input is unavailable or non-finite");
    }
    const gp_Vec normal(normal_x, normal_y, normal_z);
    if (normal.SquareMagnitude() <= tolerances().rounding * tolerances().rounding) {
      return error_result(STATUS_DEGENERATE_OPERATION, "Plane trim normal is degenerate");
    }
    const gp_Pnt origin(origin_x, origin_y, origin_z);
    const gp_Pnt keep(keep_x, keep_y, keep_z);
    if (std::abs(gp_Vec(origin, keep).Dot(normal.Normalized())) <= tolerances().linear_mm) {
      return error_result(STATUS_DEGENERATE_OPERATION, "Plane trim keep point lies on the cutting plane");
    }
    BRepBuilderAPI_MakeFace face_builder(gp_Pln(origin, gp_Dir(normal)));
    if (!face_builder.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT plane trim face did not complete");
    }
    BRepPrimAPI_MakeHalfSpace half_space_builder(face_builder.Face(), keep);
    if (!half_space_builder.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT plane trim half-space did not complete");
    }
    BRepAlgoAPI_Common operation;
    configure_boolean(operation, body.impl().shape, half_space_builder.Solid());
    operation.Build();
    if (!operation.IsDone() || operation.HasErrors()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT plane trim did not complete");
    }
    const TopoDS_Shape result = operation.Shape();
    if (result.IsNull() || !BRepCheck_Analyzer(result, true).IsValid()
        || count_subshapes(result, TopAbs_SOLID) != 1) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT plane trim must produce exactly one valid solid");
    }
    GProp_GProps source_properties;
    GProp_GProps result_properties;
    BRepGProp::VolumeProperties(body.impl().shape, source_properties);
    BRepGProp::VolumeProperties(result, result_properties);
    const double source_volume = source_properties.Mass();
    const double result_volume = result_properties.Mass();
    const double tolerance = tolerances().rounding * std::max(source_volume, 1.0);
    if (!std::isfinite(source_volume) || !std::isfinite(result_volume)
        || result_volume <= tolerance || result_volume >= source_volume - tolerance) {
      return error_result(STATUS_NO_GEOMETRIC_CHANGE, "Plane trim must remove a bounded positive volume");
    }
    std::vector<HistoryRecord> history;
    append_propagated_history(history, operation, result, body.impl());
    return success_result(result, std::move(history));
  });
}

std::unique_ptr<NativeOperationResult> boolean_bodies_native(
    const NativeOperationResult& target, const NativeOperationResult& tool,
    std::uint8_t operation_kind) noexcept {
  return guarded([&] {
    if (!target.valid() || !tool.valid() || target.impl().shape.IsNull() || tool.impl().shape.IsNull()) {
      return error_result(STATUS_INVALID_PARAMETER, "Exact Boolean input body is unavailable");
    }
    const auto finish = [&](const TopoDS_Shape& result, std::vector<HistoryRecord> history) {
      if (result.IsNull()) {
        return error_result(STATUS_NULL_RESULT, "OCCT body Boolean returned a null shape");
      }
      const std::uint32_t solids = count_subshapes(result, TopAbs_SOLID);
      if (solids == 0) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT body Boolean produced no solid result");
      }
      return success_result(result, std::move(history), solids >= 2);
    };
    if (operation_kind == 0) {
      BRepAlgoAPI_Cut operation;
      configure_boolean(operation, target.impl().shape, tool.impl().shape);
      operation.Build();
      if (!operation.IsDone() || operation.HasErrors()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT body cut did not complete");
      }
      const TopoDS_Shape result = operation.Shape();
      std::vector<HistoryRecord> history;
      append_propagated_history(history, operation, result, target.impl());
      return finish(result, std::move(history));
    }
    if (operation_kind == 1) {
      BRepAlgoAPI_Fuse operation;
      configure_boolean(operation, target.impl().shape, tool.impl().shape);
      operation.Build();
      if (operation.IsDone() && !operation.HasErrors()) {
        operation.SimplifyResult(true, true);
      }
      if (!operation.IsDone() || operation.HasErrors()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT body union did not complete");
      }
      const TopoDS_Shape result = operation.Shape();
      std::vector<HistoryRecord> history;
      append_propagated_history(history, operation, result, target.impl());
      append_propagated_history(history, operation, result, tool.impl());
      return finish(result, std::move(history));
    }
    if (operation_kind == 2) {
      BRepAlgoAPI_Common operation;
      configure_boolean(operation, target.impl().shape, tool.impl().shape);
      operation.Build();
      if (!operation.IsDone() || operation.HasErrors()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT body intersection did not complete");
      }
      const TopoDS_Shape result = operation.Shape();
      std::vector<HistoryRecord> history;
      append_propagated_history(history, operation, result, target.impl());
      append_propagated_history(history, operation, result, tool.impl());
      return finish(result, std::move(history));
    }
    if (operation_kind == 3) {
      BOPAlgo_Splitter operation;
      operation.AddArgument(target.impl().shape);
      operation.AddTool(tool.impl().shape);
      operation.Perform();
      if (operation.HasErrors()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT body split did not complete");
      }
      const TopoDS_Shape result = operation.Shape();
      if (count_subshapes(result, TopAbs_SOLID) < 2) {
        return error_result(STATUS_NO_GEOMETRIC_CHANGE, "Body split did not produce multiple target fragments");
      }
      std::vector<HistoryRecord> history;
      append_propagated_history(history, operation, result, target.impl());
      return finish(result, std::move(history));
    }
    return error_result(STATUS_INVALID_PARAMETER, "Exact body Boolean operation is unsupported");
  });
}

// Resolve on the evaluated local body before placement; never re-index a transformed body.
NativePairQuery query_face_pair_native(
    const NativeOperationResult& left, std::uint32_t left_face,
    rust::Slice<const double> left_matrix,
    const NativeOperationResult& right, std::uint32_t right_face,
    rust::Slice<const double> right_matrix) noexcept {
  NativePairQuery result{};
  result.status = STATUS_INVALID_SHAPE;
  try {
    const auto placed_face = [](const NativeOperationResult& body, std::uint32_t ordinal,
                                rust::Slice<const double> matrix) -> TopoDS_Shape {
      if (!body.valid() || body.impl().shape.IsNull() || matrix.size() != 16
          || ordinal >= body.impl().faces.size()) {
        return {};
      }
      for (double value : matrix) {
        if (!std::isfinite(value)) return {};
      }
      if (matrix[12] != 0 || matrix[13] != 0 || matrix[14] != 0 || matrix[15] != 1) return {};
      // Deliberately bounded to rigid placements. No scale/shear approximation.
      for (std::size_t a = 0; a < 3; ++a) {
        for (std::size_t b = 0; b < 3; ++b) {
          const double dot = matrix[a]*matrix[b] + matrix[4+a]*matrix[4+b]
              + matrix[8+a]*matrix[8+b];
          if (std::abs(dot - (a == b ? 1.0 : 0.0)) > tolerances().rounding) return {};
        }
      }
      const TopoDS_Face face = face_at_ordinal(body.impl().shape, ordinal);
      if (face.IsNull()) return {};
      gp_Trsf transform;
      transform.SetValues(matrix[0], matrix[1], matrix[2], matrix[3],
                          matrix[4], matrix[5], matrix[6], matrix[7],
                          matrix[8], matrix[9], matrix[10], matrix[11]);
      return face.Moved(TopLoc_Location(transform));
    };
    const TopoDS_Shape a = placed_face(left, left_face, left_matrix);
    const TopoDS_Shape b = placed_face(right, right_face, right_matrix);
    if (a.IsNull() || b.IsNull()) {
      result.diagnostic = "Face query requires valid face ordinals and rigid placements";
      return result;
    }
    BRepExtrema_DistShapeShape distance;
    distance.LoadS1(a);
    distance.LoadS2(b);
    distance.Perform();
    if (!distance.IsDone() || distance.NbSolution() < 1
        || !std::isfinite(distance.Value()) || distance.Value() < 0.0) {
      result.diagnostic = "OCCT trimmed-face minimum distance did not complete";
      return result;
    }
    result.distance_mm = distance.Value();
    result.status = STATUS_OK;
  } catch (const Standard_Failure& error) {
    result.status = STATUS_BACKEND_EXCEPTION;
    result.diagnostic = standard_failure_message(error);
  } catch (const std::exception& error) {
    result.status = STATUS_BACKEND_EXCEPTION;
    result.diagnostic = error.what();
  } catch (...) {
    result.status = STATUS_BACKEND_EXCEPTION;
    result.diagnostic = "Unknown native face-distance exception";
  }
  return result;
}

NativePairQuery query_body_pair_native(
    const NativeOperationResult& left, const NativeOperationResult& right) noexcept {
  NativePairQuery result{};
  result.status = STATUS_INVALID_SHAPE;
  try {
    if (!left.valid() || !right.valid() || left.impl().shape.IsNull() ||
        right.impl().shape.IsNull() ||
        count_subshapes(left.impl().shape, TopAbs_SOLID) == 0 ||
        count_subshapes(right.impl().shape, TopAbs_SOLID) == 0) {
      result.diagnostic = "Pair query requires valid solid bodies";
      return result;
    }
    // Only actual native-handle identity permits bypassing Boolean Common.
    if (&left == &right) {
      if (!BRepCheck_Analyzer(left.impl().shape, true).IsValid()) {
        result.diagnostic = "OCCT pair self query requires a verified solid";
        return result;
      }
      GProp_GProps properties;
      for (TopExp_Explorer solids(left.impl().shape, TopAbs_SOLID); solids.More(); solids.Next()) {
        GProp_GProps solid_properties;
        BRepGProp::VolumeProperties(solids.Current(), solid_properties);
        properties.Add(solid_properties);
      }
      const double volume = properties.Mass();
      if (!std::isfinite(volume) || volume <= 0.0) {
        result.diagnostic = "OCCT pair self query requires finite positive volume";
        return result;
      }
      result.common_volume_mm3 = volume;
      result.distance_mm = 0.0;
      result.status = STATUS_OK;
      return result;
    }
    // Tight geometric bounds, never render triangulation. If they overlap no
    // thicker than OCCT's coincidence tolerance along some axis, the solids can
    // only share faces, edges or vertices: Boolean Common would report zero
    // volume, so it is skipped and contact is measured on that slab only.
    Bnd_Box left_bounds, right_bounds;
    BRepBndLib::AddOptimal(left.impl().shape, left_bounds, false, false);
    BRepBndLib::AddOptimal(right.impl().shape, right_bounds, false, false);
    int slab_axis = -1;
    double slab_low = 0.0;
    double slab_high = 0.0;
    if (!left_bounds.IsVoid() && !right_bounds.IsVoid()) {
      double left_min[3], left_max[3], right_min[3], right_max[3];
      left_bounds.Get(left_min[0], left_min[1], left_min[2], left_max[0], left_max[1], left_max[2]);
      right_bounds.Get(
          right_min[0], right_min[1], right_min[2], right_max[0], right_max[1], right_max[2]);
      for (int axis = 0; axis < 3 && slab_axis < 0; ++axis) {
        const double low = std::max(left_min[axis], right_min[axis]);
        const double high = std::min(left_max[axis], right_max[axis]);
        // Optimal bounds still carry ~Confusion() per side, so an apparent overlap up
        // to 3 * Confusion() is a true overlap of at most the contact tolerance.
        if (high - low <= 3.0 * tolerances().linear_mm) {
          slab_axis = axis;
          slab_low = std::min(low, high);
          slab_high = std::max(low, high);
        }
      }
    }
    double volume = 0.0;
    double contact_area = 0.0;
    if (slab_axis < 0) {
      // Non-destructive: the same native shapes are reused by subsequent pairs.
      BRepAlgoAPI_Common common;
      NCollection_List<TopoDS_Shape> arguments, tools;
      arguments.Append(left.impl().shape);
      tools.Append(right.impl().shape);
      common.SetArguments(arguments);
      common.SetTools(tools);
      common.SetNonDestructive(true);
      common.Build();
      if (!common.IsDone() || common.HasErrors() || common.HasWarnings() || common.Shape().IsNull() ||
          !BRepCheck_Analyzer(common.Shape(), true).IsValid()) {
        result.diagnostic = "OCCT pair common did not produce a verified result";
        return result;
      }
      // An empty compound is a successful common query, unlike Intersect features.
      // Only solids contribute volume; face/edge/vertex contact has zero volume.
      GProp_GProps properties;
      for (TopExp_Explorer solids(common.Shape(), TopAbs_SOLID); solids.More(); solids.Next()) {
        GProp_GProps solid_properties;
        BRepGProp::VolumeProperties(solids.Current(), solid_properties);
        properties.Add(solid_properties);
      }
      volume = properties.Mass();
    }
    if (!std::isfinite(volume) || volume < 0.0 || !std::isfinite(contact_area) ||
        contact_area < 0.0) {
      result.diagnostic = "OCCT pair volume query failed";
      return result;
    }
    // A verified positive common volume proves zero solid-set distance.
    if (volume > 0.0) {
      result.common_volume_mm3 = volume;
      result.common_contact_area_mm2 = 0.0;
      result.distance_mm = 0.0;
      result.status = STATUS_OK;
      return result;
    }
    struct ContactFace {
      TopoDS_Shape shape;
      Bnd_Box bounds;
      gp_Pln plane;
      double tolerance;
      bool planar;
    };
    // With a slab, only planar faces perpendicular to its axis and lying inside
    // it can share area; every other face pair has zero-area contact.
    const auto contact_faces = [&](const TopoDS_Shape& shape) {
      std::vector<ContactFace> faces;
      for (TopExp_Explorer face(shape, TopAbs_FACE); face.More(); face.Next()) {
        Bnd_Box bounds;
        BRepBndLib::AddOptimal(face.Current(), bounds, false, true);
        const TopoDS_Face& topo_face = TopoDS::Face(face.Current());
        BRepAdaptor_Surface surface(topo_face);
        bool planar = surface.GetType() == GeomAbs_Plane;
        gp_Pln plane = planar ? surface.Plane() : gp_Pln();
        // How far the face may lie from `plane`: zero for a true plane, the fitting
        // tolerance for a fitted one. The face's own BRep tolerance is not used: an
        // imported or repaired face can carry a hundredth of a millimetre, and parallel
        // faces that far apart are a gap to measure, not a contact.
        double tolerance = 0.0;
        if (!planar) {
          // Extruded straight profile edges are flat SurfaceOfExtrusion faces;
          // their fitted plane deviates by at most the fitting tolerance.
          const GeomLib_IsPlanarSurface flat(BRep_Tool::Surface(topo_face), tolerances().linear_mm);
          if (flat.IsPlanar()) {
            planar = true;
            plane = flat.Plan();
            tolerance += tolerances().linear_mm;
          }
        }
        const gp_Dir normal = plane.Axis().Direction();
        if (slab_axis >= 0) {
          const gp_Dir axis(slab_axis == 0 ? 1.0 : 0.0, slab_axis == 1 ? 1.0 : 0.0,
                            slab_axis == 2 ? 1.0 : 0.0);
          Bnd_Box geometry;
          BRepBndLib::AddOptimal(face.Current(), geometry, false, false);
          // Flatness is proven by the face lying inside the slab, whatever its surface
          // type: extruded profile edges are flat SurfaceOfExtrusion faces, not planes.
          if (geometry.IsVoid() || (planar && std::abs(normal.Dot(axis)) < 1.0 - tolerances().rounding)) {
            continue;
          }
          double minimum[3], maximum[3];
          geometry.Get(minimum[0], minimum[1], minimum[2], maximum[0], maximum[1], maximum[2]);
          if (minimum[slab_axis] < slab_low - tolerances().linear_mm ||
              maximum[slab_axis] > slab_high + tolerances().linear_mm) {
            continue;
          }
        }
        faces.push_back({face.Current(), bounds, plane, tolerance, planar});
      }
      return faces;
    };
    const auto measure_contact_area = [&]() -> bool {
      const std::vector<ContactFace> left_faces = contact_faces(left.impl().shape);
      if (left_faces.empty()) {
        return true;
      }
      const std::vector<ContactFace> right_faces = contact_faces(right.impl().shape);
      for (const auto& left_face : left_faces) {
        for (const auto& right_face : right_faces) {
          if (left_face.bounds.IsVoid() || right_face.bounds.IsVoid() ||
              left_face.bounds.IsOut(right_face.bounds)) {
            continue;
          }
          // Planar faces share area only when they lie in one plane: parallel
          // and no further apart than their plane fit and the contact tolerance.
          if (left_face.planar && right_face.planar &&
              (std::abs(left_face.plane.Axis().Direction().Dot(right_face.plane.Axis().Direction())) <
                   1.0 - tolerances().rounding ||
               right_face.plane.Distance(left_face.plane.Location()) >
                   left_face.tolerance + right_face.tolerance + tolerances().linear_mm)) {
            continue;
          }
          BRepAlgoAPI_Common face_common;
          configure_boolean(face_common, left_face.shape, right_face.shape);
          face_common.Build();
          if (!face_common.IsDone() || face_common.HasErrors()) {
            result.diagnostic = "OCCT pair face-contact query failed";
            return false;
          }
          if (face_common.Shape().IsNull()) {
            continue;
          }
          GProp_GProps face_properties;
          BRepGProp::SurfaceProperties(face_common.Shape(), face_properties);
          const double area = face_properties.Mass();
          if (!std::isfinite(area) || area < 0.0) {
            result.diagnostic = "OCCT pair face-contact area is invalid";
            return false;
          }
          contact_area += area;
        }
      }
      return true;
    };
    // Positive shared face area already proves zero distance, so the distance
    // query runs only for solids that share no face area.
    if (!measure_contact_area()) {
      return result;
    }
    if (contact_area > 0.0) {
      result.common_volume_mm3 = 0.0;
      result.common_contact_area_mm2 = contact_area;
      result.distance_mm = 0.0;
      result.status = STATUS_OK;
      return result;
    }
    // The two-shape constructor already performs the distance computation. Only
    // the minimum is wanted; the default also searches every face pair's maximum.
    BRepExtrema_DistShapeShape distance(
        left.impl().shape, right.impl().shape, Extrema_ExtFlag_MIN);
    if (!distance.IsDone() || distance.NbSolution() == 0 ||
        !std::isfinite(distance.Value()) || distance.Value() < 0.0) {
      result.diagnostic = "OCCT pair volume or distance query failed";
      return result;
    }
    result.common_volume_mm3 = volume;
    result.common_contact_area_mm2 = 0.0;
    result.distance_mm = distance.Value();
    result.status = STATUS_OK;
  } catch (const Standard_Failure& failure) {
    result.status = STATUS_BACKEND_EXCEPTION;
    result.diagnostic = standard_failure_message(failure);
  } catch (const std::exception& failure) {
    result.status = STATUS_BACKEND_EXCEPTION;
    result.diagnostic = failure.what();
  } catch (...) {
    result.status = STATUS_BACKEND_EXCEPTION;
    result.diagnostic = "Unknown native pair query failure";
  }
  return result;
}

rust::String export_step_native(
    const NativeOperationResult& body, rust::Str path) noexcept {
  try {
    if (!body.valid() || body.impl().shape.IsNull()) {
      return rust::String("Exact body is unavailable or invalid");
    }
    const std::string native_path(path.data(), path.size());
    STEPControl_Writer writer;
    if (writer.Transfer(body.impl().shape, STEPControl_AsIs) != IFSelect_RetDone) {
      return rust::String("STEP writer could not transfer the exact body");
    }
    if (writer.Write(native_path.c_str()) != IFSelect_RetDone) {
      return rust::String("STEP writer could not write the target");
    }
    return rust::String();
  } catch (const Standard_Failure& failure) {
    return rust::String(standard_failure_message(failure));
  } catch (const std::exception& failure) {
    return rust::String(failure.what());
  } catch (...) {
    return rust::String("Unknown native STEP export failure");
  }
}

rust::String export_iges_native(
    const NativeOperationResult& body, rust::Str path) noexcept {
  try {
    if (!body.valid() || body.impl().shape.IsNull()) {
      return rust::String("Exact body is unavailable or invalid");
    }
    const std::string native_path(path.data(), path.size());
    IGESControl_Writer writer("MM", 1);
    std::size_t solid_count = 0;
    for (TopExp_Explorer explorer(body.impl().shape, TopAbs_SOLID);
         explorer.More(); explorer.Next()) {
      const TopoDS_Shape located_solid = explorer.Current();
      const gp_Trsf placement = located_solid.Location().Transformation();
      const TopoDS_Shape local_solid = located_solid.Located(TopLoc_Location());
      BRepBuilderAPI_Transform bake_placement(local_solid, placement, true);
      if (!bake_placement.IsDone()
          || !writer.AddShape(bake_placement.Shape())) {
        return rust::String("IGES writer could not transfer an exact solid");
      }
      ++solid_count;
    }
    if (solid_count == 0) {
      return rust::String("IGES export requires at least one exact solid");
    }
    writer.ComputeModel();
    if (!writer.Write(native_path.c_str())) {
      return rust::String("IGES writer could not write the target");
    }
    return rust::String();
  } catch (const Standard_Failure& failure) {
    return rust::String(standard_failure_message(failure));
  } catch (const std::exception& failure) {
    return rust::String(failure.what());
  } catch (...) {
    return rust::String("Unknown native IGES export failure");
  }
}

} // namespace ketchup::exact
