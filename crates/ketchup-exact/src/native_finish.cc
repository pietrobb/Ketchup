#include "native_common.hxx"

namespace ketchup::exact {

std::unique_ptr<NativeOperationResult> shell_body_native(
    const NativeOperationResult& body,
    rust::Slice<const std::uint32_t> face_ordinals,
    double thickness,
    std::uint8_t direction) noexcept {
  return guarded([&] {
    if (!body.valid() || face_ordinals.size() > 64
        || !std::isfinite(thickness) || thickness <= 0.0 || direction > 2) {
      return error_result(STATUS_INVALID_PARAMETER, "Body shell payload is outside the bounded envelope");
    }
    NCollection_List<TopoDS_Shape> closing_faces;
    std::uint32_t previous = 0;
    bool first = true;
    for (const std::uint32_t ordinal : face_ordinals) {
      if ((!first && ordinal <= previous)
          || ordinal >= body.impl().summary.face_count) {
        return error_result(STATUS_INVALID_PARAMETER, "Body shell face ordinals are invalid or non-canonical");
      }
      const TopoDS_Face face = face_at_ordinal(body.impl().shape, ordinal);
      if (face.IsNull()) {
        return error_result(STATUS_INVALID_SHAPE, "Body shell selected face is absent");
      }
      closing_faces.Append(face);
      previous = ordinal;
      first = false;
    }

    const auto build_thick = [&](BRepOffsetAPI_MakeThickSolid& operation, double offset) {
      operation.MakeThickSolidByJoin(
          body.impl().shape,
          closing_faces,
          offset,
          tolerances().approximation,
          BRepOffset_Skin,
          false,
          false,
          GeomAbs_Intersection,
          true);
      return operation.IsDone() && !operation.Shape().IsNull();
    };
    const auto append_selected_history = [&](
        std::vector<HistoryRecord>& history,
        BRepOffsetAPI_MakeThickSolid& operation,
        const TopoDS_Shape& result,
        const std::string& relation_prefix) {
      for (const std::uint32_t ordinal : face_ordinals) {
        const TopoDS_Face source = face_at_ordinal(body.impl().shape, ordinal);
        const std::string source_id = "generated-result/face/" + std::to_string(ordinal);
        const NCollection_List<TopoDS_Shape>& modified = operation.Modified(source);
        const NCollection_List<TopoDS_Shape>& generated = operation.Generated(source);
        for (NCollection_List<TopoDS_Shape>::Iterator iterator(modified);
             iterator.More(); iterator.Next()) {
          history.push_back(history_record(
              "", relation_prefix + "_selected_modified", source_id, result, iterator.Value()));
        }
        for (NCollection_List<TopoDS_Shape>::Iterator iterator(generated);
             iterator.More(); iterator.Next()) {
          history.push_back(history_record(
              "", relation_prefix + "_selected_generated", source_id, result, iterator.Value()));
        }
        if (operation.IsDeleted(source) || (modified.IsEmpty() && generated.IsEmpty())) {
          history.push_back(HistoryRecord{
              "", relation_prefix + "_selected_removed", source_id, 0, false});
        }
      }
    };

    if (face_ordinals.empty()) {
      BRepOffsetAPI_MakeOffsetShape inward_offset;
      BRepOffsetAPI_MakeOffsetShape outward_offset;
      const auto build_offset = [&](BRepOffsetAPI_MakeOffsetShape& operation, double offset) {
        operation.PerformByJoin(
            body.impl().shape,
            offset,
            tolerances().approximation,
            BRepOffset_Skin,
            false,
            false,
            GeomAbs_Intersection,
            true);
        return operation.IsDone() && !operation.Shape().IsNull()
            && BRepCheck_Analyzer(operation.Shape()).IsValid()
            && count_subshapes(operation.Shape(), TopAbs_SOLID) == 1;
      };
      const double inward_distance = direction == 2 ? -thickness * 0.5 : -thickness;
      const double outward_distance = direction == 2 ? thickness * 0.5 : thickness;
      const TopoDS_Shape* inner = &body.impl().shape;
      const TopoDS_Shape* outer = &body.impl().shape;
      if (direction != 1) {
        if (!build_offset(inward_offset, inward_distance)) {
          return error_result(STATUS_INVALID_SHAPE, "OCCT closed Shell inward offset did not complete");
        }
        inner = &inward_offset.Shape();
      }
      if (direction != 0) {
        if (!build_offset(outward_offset, outward_distance)) {
          return error_result(STATUS_INVALID_SHAPE, "OCCT closed Shell outward offset did not complete");
        }
        outer = &outward_offset.Shape();
      }
      BRepAlgoAPI_Cut cut(*outer, *inner);
      cut.Build();
      if (!cut.IsDone() || cut.HasErrors() || cut.Shape().IsNull()
          || !BRepCheck_Analyzer(cut.Shape()).IsValid()
          || count_subshapes(cut.Shape(), TopAbs_SOLID) != 1) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT closed Shell boolean cut did not complete");
      }
      const TopoDS_Shape result = cut.Shape();
      std::vector<HistoryRecord> history;
      const auto map_intermediate = [&cut, &result](
          std::vector<HistoryRecord>& records,
          const HistoryRecord& source,
          const TopoDS_Shape& intermediate,
          const std::string& relation_prefix) {
        const NCollection_List<TopoDS_Shape>& modified = cut.Modified(intermediate);
        const NCollection_List<TopoDS_Shape>& generated = cut.Generated(intermediate);
        const std::string semantic_role = source.semantic_role.empty()
            ? ""
            : relation_prefix + "(" + source.semantic_role + ")";
        for (NCollection_List<TopoDS_Shape>::Iterator iterator(modified);
             iterator.More(); iterator.Next()) {
          records.push_back(history_record(
              semantic_role,
              relation_prefix + "_modified",
              source.source_element_id,
              result,
              iterator.Value()));
        }
        for (NCollection_List<TopoDS_Shape>::Iterator iterator(generated);
             iterator.More(); iterator.Next()) {
          records.push_back(history_record(
              semantic_role,
              relation_prefix + "_generated",
              source.source_element_id,
              result,
              iterator.Value()));
        }
        if (!cut.IsDeleted(intermediate) && modified.IsEmpty() && generated.IsEmpty()) {
          const HistoryRecord unchanged = history_record(
              semantic_role,
              relation_prefix + "_unchanged",
              source.source_element_id,
              result,
              intermediate);
          if (unchanged.output_present) records.push_back(unchanged);
        }
      };
      const auto append_direct = [&](const std::string& relation_prefix) {
        for (const HistoryRecord& source : body.impl().history) {
          if (!source.output_present) continue;
          const TopoDS_Face source_face =
              face_at_ordinal(body.impl().shape, source.output_ordinal);
          if (!source_face.IsNull()) map_intermediate(history, source, source_face, relation_prefix);
        }
      };
      const auto append_offset = [&](
          BRepOffsetAPI_MakeOffsetShape& offset,
          const std::string& relation_prefix) {
        for (const HistoryRecord& source : body.impl().history) {
          if (!source.output_present) continue;
          const TopoDS_Face source_face =
              face_at_ordinal(body.impl().shape, source.output_ordinal);
          if (source_face.IsNull()) continue;
          const NCollection_List<TopoDS_Shape>& modified = offset.Modified(source_face);
          const NCollection_List<TopoDS_Shape>& generated = offset.Generated(source_face);
          for (NCollection_List<TopoDS_Shape>::Iterator iterator(modified);
               iterator.More(); iterator.Next()) {
            map_intermediate(history, source, iterator.Value(), relation_prefix);
          }
          for (NCollection_List<TopoDS_Shape>::Iterator iterator(generated);
               iterator.More(); iterator.Next()) {
            map_intermediate(history, source, iterator.Value(), relation_prefix);
          }
        }
      };
      if (direction == 0) {
        append_direct("shell_closed_inward_outer");
        append_offset(inward_offset, "shell_closed_inward_inner");
      } else if (direction == 1) {
        append_offset(outward_offset, "shell_closed_outward_outer");
        append_direct("shell_closed_outward_inner");
      } else {
        append_offset(outward_offset, "shell_closed_symmetric_outer");
        append_offset(inward_offset, "shell_closed_symmetric_inner");
      }
      return success_result(result, std::move(history));
    }

    if (direction != 2) {
      BRepOffsetAPI_MakeThickSolid operation;
      const double offset = direction == 0 ? -thickness : thickness;
      if (!build_thick(operation, offset)) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT body shell did not complete");
      }
      const TopoDS_Shape result = operation.Shape();
      std::vector<HistoryRecord> history;
      append_propagated_history(history, operation, result, body.impl());
      append_selected_history(
          history, operation, result, direction == 0 ? "shell_inward" : "shell_outward");
      return success_result(result, std::move(history));
    }

    BRepOffsetAPI_MakeThickSolid inward;
    BRepOffsetAPI_MakeThickSolid outward;
    if (!build_thick(inward, -thickness * 0.5)
        || !build_thick(outward, thickness * 0.5)) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT symmetric body shell halves did not complete");
    }
    BRepAlgoAPI_Fuse fusion(inward.Shape(), outward.Shape());
    fusion.Build();
    if (!fusion.IsDone() || fusion.HasErrors() || fusion.Shape().IsNull()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT symmetric body shell fuse did not complete");
    }
    fusion.SimplifyResult(true, true);
    const TopoDS_Shape result = fusion.Shape();
    std::vector<HistoryRecord> history;
    const auto append_half_history = [&](
        BRepOffsetAPI_MakeThickSolid& half,
        const std::string& relation_prefix) {
      std::vector<HistoryRecord> half_history;
      append_propagated_history(half_history, half, half.Shape(), body.impl());
      append_selected_history(half_history, half, half.Shape(), relation_prefix);
      for (const HistoryRecord& source : half_history) {
        const std::string semantic_role = source.semantic_role.empty()
            ? ""
            : relation_prefix + "(" + source.semantic_role + ")";
        if (!source.output_present) {
          history.push_back(source);
          continue;
        }
        const TopoDS_Face intermediate = face_at_ordinal(half.Shape(), source.output_ordinal);
        const NCollection_List<TopoDS_Shape>& modified = fusion.Modified(intermediate);
        if (modified.IsEmpty()) {
          HistoryRecord mapped = history_record(
              semantic_role,
              source.relation,
              source.source_element_id,
              result,
              intermediate);
          if (mapped.output_present) {
            history.push_back(std::move(mapped));
          }
        } else {
          for (NCollection_List<TopoDS_Shape>::Iterator iterator(modified);
               iterator.More(); iterator.Next()) {
            history.push_back(history_record(
                semantic_role,
                source.relation,
                source.source_element_id,
                result,
                iterator.Value()));
          }
        }
      }
    };
    append_half_history(inward, "shell_symmetric_inward");
    append_half_history(outward, "shell_symmetric_outward");
    return success_result(result, std::move(history));
  });
}

std::unique_ptr<NativeOperationResult> offset_body_face_native(
    const NativeOperationResult& body,
    std::uint32_t face_ordinal,
    double distance) noexcept {
  return guarded([&] {
    if (!body.valid() || face_ordinal >= body.impl().summary.face_count
        || !std::isfinite(distance) || std::abs(distance) < tolerances().rounding) {
      return error_result(STATUS_INVALID_PARAMETER, "Body face offset payload is outside the bounded envelope");
    }
    const TopoDS_Face face = face_at_ordinal(body.impl().shape, face_ordinal);
    if (face.IsNull()) {
      return error_result(STATUS_INVALID_SHAPE, "Body face offset selected face is absent");
    }
    gp_Dir normal;
    if (!oriented_planar_face_normal(face, normal)) {
      return error_result(STATUS_INVALID_PARAMETER, "Body face offset requires a planar face");
    }
    const gp_Vec vector(normal.X() * distance, normal.Y() * distance, normal.Z() * distance);
    BRepPrimAPI_MakePrism prism(face, vector, true, false);
    if (!prism.IsDone() || prism.Shape().IsNull()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT face offset prism did not complete");
    }
    const std::string source_id = "generated-result/face/" + std::to_string(face_ordinal);
    if (distance > 0.0) {
      BRepAlgoAPI_Fuse operation(body.impl().shape, prism.Shape());
      operation.Build();
      if (!operation.IsDone() || operation.HasErrors() || operation.Shape().IsNull()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT outward face offset did not complete");
      }
      operation.SimplifyResult(true, true);
      const TopoDS_Shape result = operation.Shape();
      std::vector<HistoryRecord> history;
      append_propagated_history(history, operation, result, body.impl());
      const NCollection_List<TopoDS_Shape>& modified = operation.Modified(face);
      for (NCollection_List<TopoDS_Shape>::Iterator iterator(modified);
           iterator.More(); iterator.Next()) {
        history.push_back(history_record(
            "", "face_offset_selected_modified", source_id, result, iterator.Value()));
      }
      return success_result(result, std::move(history));
    }
    BRepAlgoAPI_Cut operation(body.impl().shape, prism.Shape());
    operation.Build();
    if (!operation.IsDone() || operation.HasErrors() || operation.Shape().IsNull()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT inward face offset did not complete");
    }
    operation.SimplifyResult(true, true);
    const TopoDS_Shape result = operation.Shape();
    std::vector<HistoryRecord> history;
    append_propagated_history(history, operation, result, body.impl());
    const NCollection_List<TopoDS_Shape>& modified = operation.Modified(face);
    for (NCollection_List<TopoDS_Shape>::Iterator iterator(modified);
         iterator.More(); iterator.Next()) {
      history.push_back(history_record(
          "", "face_offset_selected_modified", source_id, result, iterator.Value()));
    }
    return success_result(result, std::move(history));
  });
}

std::unique_ptr<NativeOperationResult> finish_body_native(
    const NativeOperationResult& body,
    rust::Slice<const std::uint32_t> edge_ordinals,
    rust::Slice<const std::uint32_t> face_ordinals,
    double amount,
    bool fillet,
    rust::Slice<const double> fillet_radius_stations,
    std::uint8_t chamfer_mode,
    double chamfer_secondary) noexcept {
  return guarded([&] {
    if (!body.valid() || edge_ordinals.empty() || edge_ordinals.size() > 64
        || !std::isfinite(amount) || amount <= 0.0
        || fillet_radius_stations.size() > 64
        || fillet_radius_stations.size() % 2 != 0
        || (!fillet && !fillet_radius_stations.empty())
        || chamfer_mode > 2
        || (chamfer_mode == 0 && !face_ordinals.empty())
        || (chamfer_mode != 0 && (fillet || face_ordinals.size() != edge_ordinals.size()))
        || (chamfer_mode == 1 && (!std::isfinite(chamfer_secondary) || chamfer_secondary <= 0.0))
        || (chamfer_mode == 2 && (!std::isfinite(chamfer_secondary)
            || chamfer_secondary <= 0.1 || chamfer_secondary >= 89.9))) {
      return error_result(STATUS_INVALID_PARAMETER, "Body edge finish payload is outside the bounded envelope");
    }
    double previous_position = 0.0;
    for (std::size_t index = 0; index < fillet_radius_stations.size(); index += 2) {
      const double position = fillet_radius_stations[index];
      const double radius = fillet_radius_stations[index + 1];
      if (!std::isfinite(position) || position <= previous_position || position > 1.0
          || !std::isfinite(radius) || radius <= 0.0) {
        return error_result(STATUS_INVALID_PARAMETER, "Variable fillet stations are invalid or non-canonical");
      }
      previous_position = position;
    }
    if (!fillet_radius_stations.empty() && previous_position != 1.0) {
      return error_result(STATUS_INVALID_PARAMETER, "Variable fillet stations must end at normalized position one");
    }
    std::vector<TopoDS_Edge> selected;
    selected.reserve(edge_ordinals.size());
    std::uint32_t previous = 0;
    bool first = true;
    for (const std::uint32_t ordinal : edge_ordinals) {
      if ((!first && ordinal <= previous)
          || ordinal >= body.impl().summary.edge_count) {
        return error_result(STATUS_INVALID_PARAMETER, "Body edge finish ordinals are invalid or non-canonical");
      }
      const TopoDS_Edge edge = edge_at_ordinal(body.impl().shape, ordinal);
      if (edge.IsNull()) {
        return error_result(STATUS_INVALID_SHAPE, "Body edge finish selected edge is absent");
      }
      selected.push_back(edge);
      previous = ordinal;
      first = false;
    }
    std::vector<TopoDS_Face> selected_faces;
    selected_faces.reserve(face_ordinals.size());
    for (const std::uint32_t ordinal : face_ordinals) {
      if (ordinal >= body.impl().summary.face_count) {
        return error_result(STATUS_INVALID_PARAMETER, "Advanced chamfer face ordinal is out of range");
      }
      const TopoDS_Face face = face_at_ordinal(body.impl().shape, ordinal);
      if (face.IsNull()) {
        return error_result(STATUS_INVALID_SHAPE, "Advanced chamfer side face is absent");
      }
      selected_faces.push_back(face);
    }

    const auto collect_finished = [&](auto& operation) -> std::unique_ptr<NativeOperationResult> {
      operation.Build();
      if (!operation.IsDone() || operation.Shape().IsNull()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT body edge finish did not complete");
      }
      const TopoDS_Shape result = operation.Shape();
      std::vector<HistoryRecord> history;
      std::vector<EdgeHistoryRecord> edge_history;
      append_propagated_history(history, operation, result, body.impl());
      append_propagated_edge_history(edge_history, operation, result, body.impl());
      for (std::size_t index = 0; index < selected.size(); ++index) {
        const TopoDS_Edge& source = selected[index];
        const std::string source_id =
            "generated-result/edge/" + std::to_string(edge_ordinals[index]);
        const NCollection_List<TopoDS_Shape>& modified = operation.Modified(source);
        const NCollection_List<TopoDS_Shape>& generated = operation.Generated(source);
        for (NCollection_List<TopoDS_Shape>::Iterator iterator(modified);
             iterator.More(); iterator.Next()) {
          edge_history.push_back(edge_history_record(
              "", fillet ? "fillet_selected_modified" : "chamfer_selected_modified",
              source_id, result, iterator.Value()));
        }
        for (NCollection_List<TopoDS_Shape>::Iterator iterator(generated);
             iterator.More(); iterator.Next()) {
          edge_history.push_back(edge_history_record(
              "", fillet ? "fillet_selected_generated" : "chamfer_selected_generated",
              source_id, result, iterator.Value()));
        }
        if (operation.IsDeleted(source) || (modified.IsEmpty() && generated.IsEmpty())) {
          edge_history.push_back(EdgeHistoryRecord{
              "", fillet ? "fillet_selected_consumed" : "chamfer_selected_consumed",
              source_id, 0, false});
        }
      }
      return success_result(
          result, std::move(history), false, false, std::move(edge_history));
    };

    if (fillet) {
      BRepFilletAPI_MakeFillet operation(body.impl().shape);
      if (fillet_radius_stations.empty()) {
        for (const TopoDS_Edge& edge : selected) {
          operation.Add(amount, edge);
        }
      } else {
        NCollection_Array1<gp_Pnt2d> stations(1, static_cast<Standard_Integer>(fillet_radius_stations.size() / 2 + 1));
        stations.SetValue(1, gp_Pnt2d(0.0, amount));
        for (std::size_t index = 0; index < fillet_radius_stations.size(); index += 2) {
          stations.SetValue(
              static_cast<Standard_Integer>(index / 2 + 2),
              gp_Pnt2d(fillet_radius_stations[index], fillet_radius_stations[index + 1]));
        }
        for (const TopoDS_Edge& edge : selected) {
          operation.Add(stations, edge);
        }
      }
      return collect_finished(operation);
    }
    BRepFilletAPI_MakeChamfer operation(body.impl().shape);
    for (std::size_t index = 0; index < selected.size(); ++index) {
      if (chamfer_mode == 1) {
        operation.Add(amount, chamfer_secondary, selected[index], selected_faces[index]);
      } else if (chamfer_mode == 2) {
        constexpr double DEGREES_TO_RADIANS = 3.14159265358979323846 / 180.0;
        operation.AddDA(
            amount, chamfer_secondary * DEGREES_TO_RADIANS,
            selected[index], selected_faces[index]);
      } else {
        operation.Add(amount, selected[index]);
      }
    }
    return collect_finished(operation);
  });
}

std::unique_ptr<NativeOperationResult> exception_probe_native() noexcept {
  return guarded([]() -> std::unique_ptr<NativeOperationResult> {
    throw Standard_Failure("intentional A0 exception-boundary probe");
    return error_result(STATUS_BACKEND_EXCEPTION, "unreachable");
  });
}

} // namespace ketchup::exact
