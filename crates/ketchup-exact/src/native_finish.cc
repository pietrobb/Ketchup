#include "native_common.hxx"

#include <BRepTools_ReShape.hxx>
#include <Geom2d_Curve.hxx>
#include <GeomAPI_ProjectPointOnSurf.hxx>
#include <GeomProjLib.hxx>
#include <Geom_ElementarySurface.hxx>
#include <TopAbs.hxx>
#include <TopoDS_Iterator.hxx>

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
      BRepAlgoAPI_Cut cut;
      configure_boolean(cut, *outer, *inner);
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
    BRepAlgoAPI_Fuse fusion;
    configure_boolean(fusion, inward.Shape(), outward.Shape());
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

namespace {

// Rebuilds `side` on the surface of `neighbour` when every sample of `side` lies on
// that surface. The rebuilt face keeps the boundary and outward sense of `side`.
bool rebuild_on_neighbour_surface(
    const TopoDS_Face& side,
    const TopoDS_Face& neighbour,
    TopoDS_Face& rebuilt) {
  const double kCoincidence = tolerances().approximation;
  TopLoc_Location target_location;
  const occ::handle<Geom_Surface>& target = BRep_Tool::Surface(neighbour, target_location);
  if (target.IsNull() || target->IsKind(STANDARD_TYPE(Geom_ElementarySurface))) {
    // OCCT already unifies elementary surfaces by geometry.
    return false;
  }
  const occ::handle<Geom_Surface> located_target = BRep_Tool::Surface(neighbour);
  BRepAdaptor_Surface side_surface(side, false);
  double u0 = 0.0, u1 = 0.0, v0 = 0.0, v1 = 0.0;
  BRepTools::UVBounds(side, u0, u1, v0, v1);
  double same_sense = 0.0;
  for (int i = 0; i <= 2; ++i) {
    for (int j = 0; j <= 2; ++j) {
      const double u = u0 + (u1 - u0) * i * 0.5;
      const double v = v0 + (v1 - v0) * j * 0.5;
      gp_Pnt point;
      gp_Vec du, dv;
      side_surface.D1(u, v, point, du, dv);
      GeomAPI_ProjectPointOnSurf projection(point, located_target);
      if (!projection.IsDone() || projection.NbPoints() == 0
          || projection.LowerDistance() > kCoincidence) {
        return false;
      }
      double tu = 0.0, tv = 0.0;
      projection.LowerDistanceParameters(tu, tv);
      gp_Pnt target_point;
      gp_Vec tdu, tdv;
      located_target->D1(tu, tv, target_point, tdu, tdv);
      same_sense += du.Crossed(dv).Normalized().Dot(tdu.Crossed(tdv).Normalized());
    }
  }
  if (std::abs(same_sense) < 8.0) {
    return false;
  }
  const bool forward_sense = same_sense > 0.0;

  BRep_Builder builder;
  builder.MakeFace(rebuilt, target, target_location, BRep_Tool::Tolerance(side));
  TopoDS_Face forward_side = side;
  forward_side.Orientation(TopAbs_FORWARD);
  const gp_Trsf to_target_frame = target_location.Transformation().Inverted();
  struct PendingCurve {
    TopoDS_Edge edge;
    occ::handle<Geom2d_Curve> pcurve;
  };
  std::vector<PendingCurve> pending;
  for (TopoDS_Iterator wires(forward_side); wires.More(); wires.Next()) {
    if (wires.Value().ShapeType() != TopAbs_WIRE) return false;
    const TopoDS_Wire wire = TopoDS::Wire(wires.Value());
    for (TopExp_Explorer edges(wire, TopAbs_EDGE); edges.More(); edges.Next()) {
      const TopoDS_Edge edge = TopoDS::Edge(edges.Current());
      if (BRep_Tool::IsClosed(edge, forward_side) || BRep_Tool::Degenerated(edge)) {
        return false;
      }
      double first = 0.0, last = 0.0;
      const occ::handle<Geom_Curve> curve = BRep_Tool::Curve(edge, first, last);
      if (curve.IsNull()) return false;
      const occ::handle<Geom_Curve> local =
          occ::down_cast<Geom_Curve>(curve->Transformed(to_target_frame));
      const occ::handle<Geom2d_Curve> pcurve =
          GeomProjLib::Curve2d(local, first, last, target);
      if (pcurve.IsNull()) return false;
      pending.push_back(PendingCurve{edge, pcurve});
    }
    builder.Add(rebuilt, forward_sense ? wire : TopoDS::Wire(wire.Reversed()));
  }
  for (const PendingCurve& curve : pending) {
    builder.UpdateEdge(curve.edge, curve.pcurve, rebuilt, BRep_Tool::Tolerance(curve.edge));
  }
  BRepLib::SameParameter(rebuilt, kCoincidence);
  rebuilt.Orientation(forward_sense ? side.Orientation() : TopAbs::Reverse(side.Orientation()));
  return true;
}

// Puts each side face of a face-offset prism on the surface of the body face it
// continues, so the boolean can merge them into one face instead of leaving a seam
// where OCCT cannot prove two swept surfaces equal. Returns the prism unchanged when
// nothing continues a body face or the rebuilt prism is not a valid solid.
TopoDS_Shape share_continued_side_surfaces(
    const TopoDS_Shape& prism,
    const TopoDS_Shape& body,
    const TopoDS_Face& offset_face) {
  TopTools_IndexedDataMapOfShapeListOfShape edge_faces;
  TopExp::MapShapesAndAncestors(body, TopAbs_EDGE, TopAbs_FACE, edge_faces);
  std::vector<TopoDS_Face> neighbours;
  for (TopExp_Explorer edges(offset_face, TopAbs_EDGE); edges.More(); edges.Next()) {
    if (!edge_faces.Contains(edges.Current())) continue;
    for (NCollection_List<TopoDS_Shape>::Iterator faces(edge_faces.FindFromKey(edges.Current()));
         faces.More(); faces.Next()) {
      const TopoDS_Face neighbour = TopoDS::Face(faces.Value());
      const bool known = std::any_of(neighbours.begin(), neighbours.end(),
          [&](const TopoDS_Face& face) { return face.IsSame(neighbour); });
      if (!neighbour.IsSame(offset_face) && !known) neighbours.push_back(neighbour);
    }
  }
  occ::handle<BRepTools_ReShape> reshape = new BRepTools_ReShape();
  bool changed = false;
  for (TopExp_Explorer faces(prism, TopAbs_FACE); faces.More(); faces.Next()) {
    const TopoDS_Face side = TopoDS::Face(faces.Current());
    for (const TopoDS_Face& neighbour : neighbours) {
      TopoDS_Face rebuilt;
      if (rebuild_on_neighbour_surface(side, neighbour, rebuilt)) {
        reshape->Replace(side, rebuilt);
        changed = true;
        break;
      }
    }
  }
  if (!changed) return prism;
  const TopoDS_Shape shared = reshape->Apply(prism);
  if (shared.IsNull() || !BRepCheck_Analyzer(shared).IsValid()
      || count_subshapes(shared, TopAbs_SOLID) != 1) {
    return prism;
  }
  return shared;
}

}  // namespace

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
    const TopoDS_Shape tool =
        share_continued_side_surfaces(prism.Shape(), body.impl().shape, face);
    const std::string source_id = "generated-result/face/" + std::to_string(face_ordinal);
    if (distance > 0.0) {
      BRepAlgoAPI_Fuse operation;
      configure_boolean(operation, body.impl().shape, tool);
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
    BRepAlgoAPI_Cut operation;
    configure_boolean(operation, body.impl().shape, tool);
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
