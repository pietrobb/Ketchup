#include "native_common.hxx"

namespace ketchup::exact {

namespace {

// One closed planar wire at height z; wire_edges gets every segment's edge as
// stored in the wire, which is what sweep builders report history for.
TopoDS_Face named_profile_face(
    rust::Slice<const NativeSegment> segments, double z, std::vector<TopoDS_Edge>& wire_edges) {
  BRepBuilderAPI_MakeWire wire_builder;
  for (const NativeSegment& segment : segments) {
    const gp_Pnt start = planar_point(segment.start, z);
    const gp_Pnt end = planar_point(segment.end, z);
    TopoDS_Edge edge;
    if (segment.kind == NativeSegmentKind::Line) {
      if (start.Distance(end) <= tolerances().linear_mm) return {};
      BRepBuilderAPI_MakeEdge edge_builder(start, end);
      if (!edge_builder.IsDone()) return {};
      edge = edge_builder.Edge();
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
      const double radius = start.Distance(gp_Pnt(center_x, center_y, z));
      const double middle_angle = start_angle + sweep / 2.0;
      const gp_Pnt middle(
          center_x + radius * std::cos(middle_angle),
          center_y + radius * std::sin(middle_angle),
          z);
      GC_MakeArcOfCircle arc_builder(start, middle, end);
      if (!arc_builder.IsDone()) return {};
      BRepBuilderAPI_MakeEdge edge_builder(arc_builder.Value());
      if (!edge_builder.IsDone()) return {};
      edge = edge_builder.Edge();
    } else if (segment.kind == NativeSegmentKind::CubicBezier) {
      edge = cubic_bezier_edge(segment, z);
    }
    if (edge.IsNull()) return {};
    wire_builder.Add(edge);
    if (!wire_builder.IsDone()) return {};
    wire_edges.push_back(wire_builder.Edge());
  }
  if (!wire_builder.IsDone() || !wire_builder.Wire().Closed()) return {};
  BRepBuilderAPI_MakeFace face_builder(wire_builder.Wire(), true);
  if (!face_builder.IsDone() || !BRepCheck_Analyzer(face_builder.Face()).IsValid()) return {};
  return face_builder.Face();
}

void record_name(
    std::vector<HistoryRecord>& history,
    const std::string& name,
    const TopoDS_Shape& result,
    const TopoDS_Shape& face) {
  HistoryRecord record = history_record(name, "named", name, result, face);
  if (record.output_present) history.push_back(std::move(record));
}

template <typename Operation>
void record_generated_names(
    std::vector<HistoryRecord>& history,
    Operation& operation,
    const TopoDS_Shape& source,
    const std::string& name,
    const TopoDS_Shape& result) {
  const NCollection_List<TopoDS_Shape>& generated = operation.Generated(source);
  for (NCollection_List<TopoDS_Shape>::Iterator iterator(generated);
       iterator.More(); iterator.Next()) {
    if (iterator.Value().ShapeType() == TopAbs_FACE) {
      record_name(history, name, result, iterator.Value());
    }
  }
}

bool labels_match(const TopoDS_Shape& shape, rust::Slice<const rust::String> labels) {
  TopTools_IndexedMapOfShape faces;
  TopExp::MapShapes(shape, TopAbs_FACE, faces);
  return static_cast<std::size_t>(faces.Extent()) == labels.size();
}

// Carries every source face's label to the faces it became in the result.
template <typename Operation>
void propagate_names(
    std::vector<HistoryRecord>& history,
    Operation& operation,
    const TopoDS_Shape& result,
    const TopoDS_Shape& source,
    rust::Slice<const rust::String> labels) {
  TopTools_IndexedMapOfShape faces;
  TopExp::MapShapes(source, TopAbs_FACE, faces);
  for (Standard_Integer index = 1; index <= faces.Extent(); ++index) {
    const TopoDS_Shape& face = faces(index);
    const std::string name(labels[static_cast<std::size_t>(index - 1)]);
    if (operation.IsDeleted(face)) continue;
    const NCollection_List<TopoDS_Shape>& modified = operation.Modified(face);
    for (NCollection_List<TopoDS_Shape>::Iterator iterator(modified);
         iterator.More(); iterator.Next()) {
      record_name(history, name, result, iterator.Value());
    }
    if (modified.IsEmpty()) record_name(history, name, result, face);
  }
}

std::string finish_name(const std::string& kind, std::vector<std::string> around) {
  std::sort(around.begin(), around.end());
  around.erase(std::unique(around.begin(), around.end()), around.end());
  std::string name = kind + "(";
  for (std::size_t position = 0; position < around.size(); ++position) {
    name += (position == 0 ? "" : ",") + around[position];
  }
  return name + ")";
}

// The original sharp geometry lies outside a finish face, off it by less than
// `reach`, and its nearest point on the face is strictly inside the face.
bool lies_over(const gp_Pnt& point, const TopoDS_Face& face, double reach) {
  BRepExtrema_DistShapeShape distance(BRepBuilderAPI_MakeVertex(point).Vertex(), face);
  if (!distance.IsDone() || distance.Value() <= tolerances().linear_mm
      || distance.Value() > reach) {
    return false;
  }
  for (Standard_Integer index = 1; index <= distance.NbSolution(); ++index) {
    if (distance.SupportTypeShape2(index) == BRepExtrema_IsInFace) return true;
  }
  return false;
}

// Names the faces a fillet or chamfer created. OCCT finishes whole tangent
// chains and its Generated() history attributes chain faces to whichever
// edge was selected, so history is not used here. A new face is named after
// the input edge it replaced: it borders both faces of that edge and at least
// two points along the edge lie over its interior. A face that replaced no
// edge but lies over an input vertex is a corner blend named after every
// face at that vertex. Faces matching nothing stay unnamed and are reported.
void name_finish_faces(
    std::vector<HistoryRecord>& history,
    const TopoDS_Shape& result,
    const TopoDS_Shape& source,
    rust::Slice<const rust::String> labels,
    const std::string& kind,
    double amount) {
  TopTools_IndexedMapOfShape result_faces;
  TopExp::MapShapes(result, TopAbs_FACE, result_faces);
  std::vector<std::set<std::string>> result_names(
      static_cast<std::size_t>(result_faces.Extent()));
  for (const HistoryRecord& record : history) {
    if (record.output_present) result_names[record.output_ordinal].insert(record.source_element_id);
  }
  TopTools_IndexedDataMapOfShapeListOfShape result_edge_faces;
  TopExp::MapShapesAndAncestors(result, TopAbs_EDGE, TopAbs_FACE, result_edge_faces);

  TopTools_IndexedMapOfShape source_faces;
  TopExp::MapShapes(source, TopAbs_FACE, source_faces);
  const auto names_around = [&](const NCollection_List<TopoDS_Shape>& faces) {
    std::vector<std::string> names;
    for (NCollection_List<TopoDS_Shape>::Iterator face(faces); face.More(); face.Next()) {
      const Standard_Integer ordinal = source_faces.FindIndex(face.Value());
      if (ordinal > 0) names.emplace_back(labels[static_cast<std::size_t>(ordinal - 1)]);
    }
    std::sort(names.begin(), names.end());
    names.erase(std::unique(names.begin(), names.end()), names.end());
    return names;
  };
  TopTools_IndexedDataMapOfShapeListOfShape source_edge_faces;
  TopExp::MapShapesAndAncestors(source, TopAbs_EDGE, TopAbs_FACE, source_edge_faces);
  TopTools_IndexedDataMapOfShapeListOfShape source_vertex_faces;
  TopExp::MapShapesAndAncestors(source, TopAbs_VERTEX, TopAbs_FACE, source_vertex_faces);
  const double reach = 10.0 * amount;
  constexpr int SAMPLES = 9;

  for (std::size_t index = 0; index < result_names.size(); ++index) {
    if (!result_names[index].empty()) continue;
    const TopoDS_Face face = TopoDS::Face(result_faces(static_cast<Standard_Integer>(index + 1)));
    std::set<std::string> neighbours;
    for (TopExp_Explorer edge(face, TopAbs_EDGE); edge.More(); edge.Next()) {
      const Standard_Integer found = result_edge_faces.FindIndex(edge.Current());
      if (found == 0) continue;
      for (NCollection_List<TopoDS_Shape>::Iterator other(result_edge_faces(found));
           other.More(); other.Next()) {
        const Standard_Integer ordinal = result_faces.FindIndex(other.Value());
        if (ordinal > 0) {
          const auto& names = result_names[static_cast<std::size_t>(ordinal - 1)];
          neighbours.insert(names.begin(), names.end());
        }
      }
    }
    const auto bordered = [&](const std::vector<std::string>& names) {
      return std::all_of(names.begin(), names.end(), [&](const std::string& name) {
        return neighbours.count(name) != 0;
      });
    };
    std::vector<std::string> spanned;
    for (Standard_Integer edge_index = 1; edge_index <= source_edge_faces.Extent(); ++edge_index) {
      const std::vector<std::string> around = names_around(source_edge_faces(edge_index));
      if (around.size() != 2 || !bordered(around)) continue;
      const BRepAdaptor_Curve curve(TopoDS::Edge(source_edge_faces.FindKey(edge_index)));
      int over = 0;
      for (int sample = 0; sample < SAMPLES && over < 2; ++sample) {
        const double parameter = curve.FirstParameter()
            + (curve.LastParameter() - curve.FirstParameter()) * (sample + 0.5) / SAMPLES;
        if (lies_over(curve.Value(parameter), face, reach)) ++over;
      }
      if (over >= 2) spanned.push_back(finish_name(kind, around));
    }
    if (spanned.empty()) {
      for (Standard_Integer vertex_index = 1; vertex_index <= source_vertex_faces.Extent();
           ++vertex_index) {
        const std::vector<std::string> around = names_around(source_vertex_faces(vertex_index));
        const gp_Pnt point =
            BRep_Tool::Pnt(TopoDS::Vertex(source_vertex_faces.FindKey(vertex_index)));
        // A corner where every edge is finished borders only other blends,
        // so the vertex is matched by the face lying over it alone.
        if (around.size() >= 3 && lies_over(point, face, reach)) {
          spanned.push_back(finish_name(kind, around));
        }
      }
    }
    for (const std::string& name : spanned) record_name(history, name, result, face);
  }
}

} // namespace

std::unique_ptr<NativeOperationResult> named_prism_native(
    rust::Slice<const NativeSegment> segments, double base_z, double height) noexcept {
  return guarded([&] {
    if (segments.empty() || segments.size() > 4096
        || !std::isfinite(base_z) || !std::isfinite(height) || height <= 0.0) {
      return error_result(STATUS_INVALID_PARAMETER, "Named prism payload is malformed");
    }
    std::vector<TopoDS_Edge> wire_edges;
    const TopoDS_Face profile = named_profile_face(segments, base_z, wire_edges);
    if (profile.IsNull()) {
      return error_result(STATUS_INVALID_PARAMETER, "Named prism profile is not one closed planar loop");
    }
    // Canonical walls (planes, cylinders) keep later fillets exact and let
    // coplanar or coaxial walls merge.
    BRepPrimAPI_MakePrism operation(profile, gp_Vec(0.0, 0.0, height), true, true);
    if (!operation.IsDone() || operation.Shape().IsNull()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT named prism did not complete");
    }
    const TopoDS_Shape result = operation.Shape();
    std::vector<HistoryRecord> history;
    record_name(history, "start", result, operation.FirstShape());
    record_name(history, "end", result, operation.LastShape());
    for (std::size_t index = 0; index < wire_edges.size(); ++index) {
      record_generated_names(
          history, operation, wire_edges[index], "seg:" + std::to_string(index), result);
    }
    return success_result(result, std::move(history));
  });
}

std::unique_ptr<NativeOperationResult> named_revol_native(
    rust::Slice<const NativeSegment> segments,
    double axis_start_x, double axis_start_y,
    double axis_end_x, double axis_end_y,
    double angle_degrees) noexcept {
  return guarded([&] {
    const gp_Vec axis_vector(axis_end_x - axis_start_x, axis_end_y - axis_start_y, 0.0);
    if (segments.empty() || segments.size() > 4096
        || !std::isfinite(angle_degrees) || angle_degrees <= 0.0 || angle_degrees > 360.0
        || !(axis_vector.Magnitude() > tolerances().negligible)) {
      return error_result(STATUS_INVALID_PARAMETER, "Named revolve payload is malformed");
    }
    std::vector<TopoDS_Edge> wire_edges;
    const TopoDS_Face profile = named_profile_face(segments, 0.0, wire_edges);
    if (profile.IsNull()) {
      return error_result(STATUS_INVALID_PARAMETER, "Named revolve profile is not one closed planar loop");
    }
    BRepPrimAPI_MakeRevol operation(
        profile,
        gp_Ax1(gp_Pnt(axis_start_x, axis_start_y, 0.0), gp_Dir(axis_vector)),
        angle_degrees * std::acos(-1.0) / 180.0,
        true);
    if (!operation.IsDone() || operation.Shape().IsNull()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT named revolve did not complete");
    }
    const TopoDS_Shape result = operation.Shape();
    std::vector<HistoryRecord> history;
    if (angle_degrees < 360.0) {
      record_name(history, "start", result, operation.FirstShape());
      record_name(history, "end", result, operation.LastShape());
    }
    for (std::size_t index = 0; index < wire_edges.size(); ++index) {
      record_generated_names(
          history, operation, wire_edges[index], "seg:" + std::to_string(index), result);
    }
    return success_result(result, std::move(history));
  });
}

std::unique_ptr<NativeOperationResult> named_finish_native(
    const NativeOperationResult& body,
    rust::Slice<const rust::String> labels,
    rust::Slice<const std::uint32_t> edge_ordinals,
    double amount,
    bool fillet) noexcept {
  return guarded([&]() -> std::unique_ptr<NativeOperationResult> {
    if (!body.valid() || !labels_match(body.impl().shape, labels) || edge_ordinals.empty()
        || edge_ordinals.size() > 64 || !std::isfinite(amount) || amount <= 0.0) {
      return error_result(STATUS_INVALID_PARAMETER, "Named finish payload is malformed");
    }
    std::vector<TopoDS_Edge> selected;
    for (const std::uint32_t ordinal : edge_ordinals) {
      const TopoDS_Edge edge = edge_at_ordinal(body.impl().shape, ordinal);
      if (edge.IsNull()) {
        return error_result(STATUS_INVALID_PARAMETER, "Named finish edge is absent");
      }
      selected.push_back(edge);
    }
    const auto collect = [&](auto& operation) -> std::unique_ptr<NativeOperationResult> {
      operation.Build();
      if (!operation.IsDone() || operation.Shape().IsNull()) {
        std::ostringstream message;
        message << "the " << (fillet ? "fillet" : "chamfer") << " of " << amount
                << " mm does not fit on the selected edges (finish did not complete); "
                   "the faces next to an edge must be wider than the amount";
        return error_result(STATUS_INVALID_SHAPE, message.str());
      }
      const TopoDS_Shape result = operation.Shape();
      std::vector<HistoryRecord> history;
      propagate_names(history, operation, result, body.impl().shape, labels);
      name_finish_faces(
          history, result, body.impl().shape, labels, fillet ? "fillet" : "chamfer", amount);
      return success_result(result, std::move(history));
    };
    if (fillet) {
      BRepFilletAPI_MakeFillet operation(body.impl().shape);
      for (const TopoDS_Edge& edge : selected) operation.Add(amount, edge);
      return collect(operation);
    }
    BRepFilletAPI_MakeChamfer operation(body.impl().shape);
    for (const TopoDS_Edge& edge : selected) operation.Add(amount, edge);
    return collect(operation);
  });
}

std::unique_ptr<NativeOperationResult> named_boolean_native(
    const NativeOperationResult& target,
    rust::Slice<const rust::String> target_labels,
    const NativeOperationResult& tool,
    rust::Slice<const rust::String> tool_labels,
    std::uint8_t operation_kind) noexcept {
  return guarded([&]() -> std::unique_ptr<NativeOperationResult> {
    if (!target.valid() || !tool.valid()
        || !labels_match(target.impl().shape, target_labels)
        || !labels_match(tool.impl().shape, tool_labels) || operation_kind > 2) {
      return error_result(STATUS_INVALID_PARAMETER, "Named Boolean payload is malformed");
    }
    const auto collect = [&](auto& operation) -> std::unique_ptr<NativeOperationResult> {
      operation.Build();
      if (!operation.IsDone() || operation.HasErrors() || operation.Shape().IsNull()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT named Boolean did not complete");
      }
      operation.SimplifyResult(true, true);
      const TopoDS_Shape result = operation.Shape();
      std::vector<HistoryRecord> history;
      propagate_names(history, operation, result, target.impl().shape, target_labels);
      propagate_names(history, operation, result, tool.impl().shape, tool_labels);
      return success_result(result, std::move(history));
    };
    if (operation_kind == 0) {
      BRepAlgoAPI_Cut operation;
      configure_boolean(operation, target.impl().shape, tool.impl().shape);
      return collect(operation);
    }
    if (operation_kind == 2) {
      BRepAlgoAPI_Common operation;
      configure_boolean(operation, target.impl().shape, tool.impl().shape);
      return collect(operation);
    }
    BRepAlgoAPI_Fuse operation;
    configure_boolean(operation, target.impl().shape, tool.impl().shape);
    return collect(operation);
  });
}

std::unique_ptr<NativeOperationResult> named_offset_face_native(
    const NativeOperationResult& body,
    rust::Slice<const rust::String> labels,
    std::uint32_t face_ordinal,
    double distance) noexcept {
  return guarded([&]() -> std::unique_ptr<NativeOperationResult> {
    if (!body.valid() || !labels_match(body.impl().shape, labels)
        || face_ordinal >= labels.size() || !std::isfinite(distance)
        || std::abs(distance) < tolerances().rounding) {
      return error_result(STATUS_INVALID_PARAMETER, "Named face offset payload is malformed");
    }
    const TopoDS_Face face = face_at_ordinal(body.impl().shape, face_ordinal);
    gp_Dir normal;
    if (face.IsNull() || !oriented_planar_face_normal(face, normal)) {
      return error_result(STATUS_INVALID_PARAMETER, "Named face offset requires a planar face");
    }
    // Canonical side surfaces (a round edge sweeps a cylinder, not a surface
    // of extrusion) let the fuse merge them with the walls they continue. No
    // copy, so the sides trace back to the body's own edges.
    BRepPrimAPI_MakePrism prism(face, gp_Vec(normal) * distance, false, true);
    if (!prism.IsDone() || prism.Shape().IsNull()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT named face offset prism did not complete");
    }
    const std::string moved_name(labels[face_ordinal]);
    const auto collect = [&](auto& operation) -> std::unique_ptr<NativeOperationResult> {
      operation.Build();
      if (!operation.IsDone() || operation.HasErrors() || operation.Shape().IsNull()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT named face offset did not complete");
      }
      operation.SimplifyResult(true, true);
      const TopoDS_Shape result = operation.Shape();
      std::vector<HistoryRecord> history;
      propagate_names(history, operation, result, body.impl().shape, labels);
      // The far cap of the offset prism is where the pushed or pulled face now lies.
      const TopoDS_Shape moved = prism.LastShape();
      const NCollection_List<TopoDS_Shape>& modified = operation.Modified(moved);
      for (NCollection_List<TopoDS_Shape>::Iterator iterator(modified);
           iterator.More(); iterator.Next()) {
        record_name(history, moved_name, result, iterator.Value());
      }
      if (modified.IsEmpty()) record_name(history, moved_name, result, moved);
      // Each side of the prism continues the wall across its generating edge;
      // where the fuse could not merge the two, the side keeps that wall's name.
      TopTools_IndexedMapOfShape body_faces;
      TopExp::MapShapes(body.impl().shape, TopAbs_FACE, body_faces);
      TopTools_IndexedDataMapOfShapeListOfShape edge_faces;
      TopExp::MapShapesAndAncestors(body.impl().shape, TopAbs_EDGE, TopAbs_FACE, edge_faces);
      for (TopExp_Explorer edges(face, TopAbs_EDGE); edges.More(); edges.Next()) {
        const TopoDS_Shape& edge = edges.Current();
        if (!edge_faces.Contains(edge)) continue;
        for (NCollection_List<TopoDS_Shape>::Iterator wall(edge_faces.FindFromKey(edge));
             wall.More(); wall.Next()) {
          if (wall.Value().IsSame(face)) continue;
          const Standard_Integer wall_index = body_faces.FindIndex(wall.Value());
          if (wall_index == 0) continue;
          const std::string wall_name(labels[static_cast<std::size_t>(wall_index - 1)]);
          const NCollection_List<TopoDS_Shape>& sides = prism.Generated(edge);
          for (NCollection_List<TopoDS_Shape>::Iterator side(sides); side.More(); side.Next()) {
            if (side.Value().ShapeType() != TopAbs_FACE) continue;
            const NCollection_List<TopoDS_Shape>& fused = operation.Modified(side.Value());
            for (NCollection_List<TopoDS_Shape>::Iterator piece(fused); piece.More(); piece.Next()) {
              record_name(history, wall_name, result, piece.Value());
            }
            if (fused.IsEmpty()) record_name(history, wall_name, result, side.Value());
          }
        }
      }
      return success_result(result, std::move(history));
    };
    if (distance > 0.0) {
      BRepAlgoAPI_Fuse operation;
      configure_boolean(operation, body.impl().shape, prism.Shape());
      return collect(operation);
    }
    BRepAlgoAPI_Cut operation;
    configure_boolean(operation, body.impl().shape, prism.Shape());
    return collect(operation);
  });
}

namespace {

// A point strictly inside `face`: its UV centre when that lies on the face
// (not in a hole of it), else the first grid point that does.
bool interior_point(const TopoDS_Face& face, gp_Pnt& point) {
  Standard_Real u_min = 0.0, u_max = 0.0, v_min = 0.0, v_max = 0.0;
  BRepTools::UVBounds(face, u_min, u_max, v_min, v_max);
  const BRepAdaptor_Surface surface(face);
  constexpr int GRID = 9;
  for (int step = 0; step <= GRID * GRID; ++step) {
    double u_fraction = 0.5, v_fraction = 0.5;
    if (step > 0) {
      u_fraction = ((step - 1) % GRID + 0.5) / GRID;
      v_fraction = ((step - 1) / GRID + 0.5) / GRID;
    }
    const gp_Pnt2d uv(u_min + (u_max - u_min) * u_fraction, v_min + (v_max - v_min) * v_fraction);
    BRepClass_FaceClassifier classifier(face, uv, tolerances().linear_mm);
    if (classifier.State() == TopAbs_IN) {
      point = surface.Value(uv.X(), uv.Y());
      return true;
    }
  }
  return false;
}

} // namespace

std::unique_ptr<NativeOperationResult> named_shell_native(
    const NativeOperationResult& body,
    rust::Slice<const rust::String> labels,
    rust::Slice<const std::uint32_t> open_ordinals,
    double thickness,
    rust::Str prefix) noexcept {
  return guarded([&]() -> std::unique_ptr<NativeOperationResult> {
    if (!body.valid() || !labels_match(body.impl().shape, labels) || open_ordinals.empty()
        || open_ordinals.size() > 64
        || !std::isfinite(thickness) || thickness <= 0.0 || prefix.empty()) {
      return error_result(STATUS_INVALID_PARAMETER, "Named shell payload is malformed");
    }
    const TopoDS_Shape& source = body.impl().shape;
    NCollection_List<TopoDS_Shape> closing;
    for (const std::uint32_t ordinal : open_ordinals) {
      const TopoDS_Face face = face_at_ordinal(source, ordinal);
      if (face.IsNull()) {
        return error_result(STATUS_INVALID_PARAMETER, "Named shell open face is absent");
      }
      closing.Append(face);
    }
    std::ostringstream too_thick;
    too_thick << "walls " << thickness
              << " mm thick do not fit inside the part (shell did not complete); "
                 "use thinner walls or open other faces";
    BRepOffsetAPI_MakeThickSolid operation;
    operation.MakeThickSolidByJoin(
        source, closing, -thickness, tolerances().approximation, BRepOffset_Skin, false, false,
        GeomAbs_Intersection, true);
    if (!operation.IsDone() || operation.Shape().IsNull()) {
      return error_result(STATUS_INVALID_SHAPE, too_thick.str());
    }
    const TopoDS_Shape result = operation.Shape();
    if (!BRepCheck_Analyzer(result).IsValid() || count_subshapes(result, TopAbs_SOLID) != 1) {
      return error_result(STATUS_INVALID_SHAPE, too_thick.str());
    }
    // A face lying on a source face keeps its name (outer walls, and the rim
    // left where an open face was); one running the wall thickness inside a
    // source face is that face's inner wall, "<prefix>.<name>".
    TopTools_IndexedMapOfShape source_faces;
    TopExp::MapShapes(source, TopAbs_FACE, source_faces);
    TopTools_IndexedMapOfShape result_faces;
    TopExp::MapShapes(result, TopAbs_FACE, result_faces);
    const double tolerance = tolerances().approximation * std::max(1.0, thickness);
    const std::string inner_prefix = std::string(prefix) + ".";
    std::vector<HistoryRecord> history;
    for (Standard_Integer index = 1; index <= result_faces.Extent(); ++index) {
      const TopoDS_Face face = TopoDS::Face(result_faces(index));
      gp_Pnt point;
      if (!interior_point(face, point)) continue;
      const TopoDS_Vertex vertex = BRepBuilderAPI_MakeVertex(point).Vertex();
      double nearest = std::numeric_limits<double>::infinity();
      Standard_Integer owner = 0;
      for (Standard_Integer candidate = 1; candidate <= source_faces.Extent(); ++candidate) {
        BRepExtrema_DistShapeShape distance(vertex, source_faces(candidate));
        if (distance.IsDone() && distance.Value() < nearest) {
          nearest = distance.Value();
          owner = candidate;
        }
      }
      if (owner == 0) continue;
      const std::string label(labels[static_cast<std::size_t>(owner - 1)]);
      if (nearest <= tolerance) {
        record_name(history, label, result, face);
      } else if (std::abs(nearest - thickness) <= tolerance) {
        record_name(history, inner_prefix + label, result, face);
      }
    }
    return success_result(result, std::move(history));
  });
}

} // namespace ketchup::exact
