#include "native_common.hxx"

#include <BRepAlgoAPI_Defeaturing.hxx>
#include <BRepAlgoAPI_Section.hxx>
#include <BRepOffsetAPI_DraftAngle.hxx>
#include <BRepTools_ReShape.hxx>
#include <Geom2d_Curve.hxx>
#include <GeomAPI_ProjectPointOnSurf.hxx>
#include <GeomProjLib.hxx>
#include <Geom_ElementarySurface.hxx>
#include <TopAbs.hxx>
#include <TopoDS_Iterator.hxx>
#include <gp_Ax3.hxx>

#include <algorithm>
#include <cstdio>

namespace ketchup::exact {

namespace {

constexpr const char* NOT_HOLLOWED =
    "Shell left the body solid: the wall is too thick for the body, or a curved face "
    "cannot be offset by that thickness";

constexpr const char* OFFSET_FAILED =
    "Shell could not offset the body by the wall thickness: the wall is thicker than "
    "the body allows, or a round or curved face is tighter than the wall";

double volume_of(const TopoDS_Shape& shape) {
  GProp_GProps properties;
  BRepGProp::VolumeProperties(shape, properties);
  return properties.Mass();
}

// The solid an offset encloses. OCCT hands back the offset of a body with
// rounds as a closed shell rather than a solid; it is made one here.
// OCCT also offsets a round tighter than the distance, turning it inside out
// instead of letting it vanish; every face of the offset must lie the offset
// distance from the body's surface.
bool offset_faces_at_distance(
    const TopoDS_Shape& offset_shape, const TopoDS_Shape& source, double distance) {
  TopExp_Explorer shells(source, TopAbs_SHELL);
  if (!shells.More()) return false;
  const TopoDS_Shape surface = shells.Current();
  const double tolerance = std::max(tolerances().approximation, tolerances().rounding * distance);
  for (TopExp_Explorer faces(offset_shape, TopAbs_FACE); faces.More(); faces.Next()) {
    const TopoDS_Face face = TopoDS::Face(faces.Current());
    double u0, u1, v0, v1;
    BRepTools::UVBounds(face, u0, u1, v0, v1);
    const occ::handle<Geom_Surface> geometry = BRep_Tool::Surface(face);
    constexpr int SAMPLES = 3;
    for (int i = 1; i <= SAMPLES; ++i) {
      for (int j = 1; j <= SAMPLES; ++j) {
        const gp_Pnt2d uv(u0 + (u1 - u0) * i / (SAMPLES + 1), v0 + (v1 - v0) * j / (SAMPLES + 1));
        BRepClass_FaceClassifier inside(face, uv, tolerances().linear_mm);
        if (inside.State() != TopAbs_IN) continue;
        const TopoDS_Vertex sample = BRepBuilderAPI_MakeVertex(geometry->Value(uv.X(), uv.Y()));
        BRepExtrema_DistShapeShape measure(sample, surface);
        if (!measure.IsDone() || std::abs(measure.Value() - distance) > tolerance) return false;
      }
    }
  }
  return true;
}

TopoDS_Shape offset_solid(BRepOffsetAPI_MakeOffsetShape& offset, const TopoDS_Shape& source,
                          double distance) {
  if (!offset.IsDone() || offset.Shape().IsNull()) return {};
  TopoDS_Shape shape = offset.Shape();
  if (shape.ShapeType() == TopAbs_SHELL && shape.Closed()) {
    BRepBuilderAPI_MakeSolid solid_maker(TopoDS::Shell(shape));
    if (!solid_maker.IsDone()) return {};
    TopoDS_Solid solid = solid_maker.Solid();
    if (!BRepLib::OrientClosedSolid(solid)) return {};
    shape = solid;
  }
  if (!BRepCheck_Analyzer(shape).IsValid() || count_subshapes(shape, TopAbs_SOLID) != 1
      || !(volume_of(shape) > 0.0) || !offset_faces_at_distance(shape, source, distance)) {
    return {};
  }
  return shape;
}

// A convex round (cylinder, sphere or torus tube curving away from the
// material) whose radius is no larger than `distance`.
bool convex_round_within(const TopoDS_Face& face, double distance) {
  const BRepAdaptor_Surface surface(face);
  double u0, u1, v0, v1;
  BRepTools::UVBounds(face, u0, u1, v0, v1);
  gp_Pnt point;
  gp_Vec along_u, along_v;
  surface.D1((u0 + u1) * 0.5, (v0 + v1) * 0.5, point, along_u, along_v);
  gp_Vec normal = along_u.Crossed(along_v);
  if (normal.Magnitude() <= tolerances().approximation) return false;
  if (face.Orientation() == TopAbs_REVERSED) normal.Reverse();
  gp_Pnt centre;
  double radius = 0.0;
  switch (surface.GetType()) {
    case GeomAbs_Cylinder: {
      const gp_Ax1 axis = surface.Cylinder().Axis();
      const gp_Vec direction(axis.Direction());
      centre = axis.Location().Translated(
          direction * gp_Vec(axis.Location(), point).Dot(direction));
      radius = surface.Cylinder().Radius();
      break;
    }
    case GeomAbs_Sphere:
      centre = surface.Sphere().Location();
      radius = surface.Sphere().Radius();
      break;
    case GeomAbs_Torus: {
      const gp_Ax3 frame = surface.Torus().Position();
      const gp_Vec axis(frame.Direction());
      gp_Vec across(frame.Location(), point);
      across -= axis * across.Dot(axis);
      if (across.Magnitude() <= tolerances().approximation) return false;
      centre = frame.Location().Translated(
          across.Normalized() * surface.Torus().MajorRadius());
      radius = surface.Torus().MinorRadius();
      break;
    }
    default:
      return false;
  }
  return radius <= distance + tolerances().linear_mm
      && normal.Dot(gp_Vec(centre, point)) > 0.0;
}

// The body an inward offset of `distance` starts from. No point inside a
// convex round of radius r ≤ t lies t from it: eroding a round by the wall
// erodes its sharp edge, so such rounds are taken back to the edge their
// neighbours meet at. `defeaturing` stays empty when there are none.
TopoDS_Shape inward_offset_source(
    const TopoDS_Shape& body, double distance,
    std::unique_ptr<BRepAlgoAPI_Defeaturing>& defeaturing) {
  NCollection_List<TopoDS_Shape> rounds;
  for (TopExp_Explorer faces(body, TopAbs_FACE); faces.More(); faces.Next()) {
    if (convex_round_within(TopoDS::Face(faces.Current()), distance)) rounds.Append(faces.Current());
  }
  if (rounds.IsEmpty()) return body;
  defeaturing = std::make_unique<BRepAlgoAPI_Defeaturing>();
  defeaturing->SetShape(body);
  defeaturing->AddFacesToRemove(rounds);
  defeaturing->SetToFillHistory(true);
  defeaturing->Build();
  if (!defeaturing->IsDone() || defeaturing->HasErrors() || defeaturing->Shape().IsNull()
      || !BRepCheck_Analyzer(defeaturing->Shape()).IsValid()
      || count_subshapes(defeaturing->Shape(), TopAbs_SOLID) != 1) {
    return {};
  }
  return defeaturing->Shape();
}

// A body face's images on an offset made from `inward_offset_source`.
NCollection_List<TopoDS_Shape> offset_images(
    const TopoDS_Shape& face,
    const std::unique_ptr<BRepAlgoAPI_Defeaturing>& defeaturing,
    BRepOffsetAPI_MakeOffsetShape& offset) {
  NCollection_List<TopoDS_Shape> sources;
  if (!defeaturing) {
    sources.Append(face);
  } else if (!defeaturing->IsDeleted(face)) {
    sources = defeaturing->Modified(face);
    if (sources.IsEmpty()) sources.Append(face);
  }
  NCollection_List<TopoDS_Shape> images;
  for (NCollection_List<TopoDS_Shape>::Iterator source(sources); source.More(); source.Next()) {
    for (NCollection_List<TopoDS_Shape>::Iterator image(offset.Modified(source.Value()));
         image.More(); image.Next()) {
      images.Append(image.Value());
    }
    for (NCollection_List<TopoDS_Shape>::Iterator image(offset.Generated(source.Value()));
         image.More(); image.Next()) {
      images.Append(image.Value());
    }
  }
  return images;
}

// OCCT can report an open shell as done yet hand back the body unchanged,
// e.g. when the wall is thicker than the body or a round cannot be offset.
// An inward wall must remove volume; an outward one must change it.
bool shell_hollowed(const TopoDS_Shape& source, const TopoDS_Shape& shape, double offset) {
  const double source_volume = volume_of(source);
  const double result_volume = volume_of(shape);
  const double tolerance = tolerances().rounding * std::max(source_volume, 1.0);
  return std::isfinite(source_volume) && std::isfinite(result_volume)
      && result_volume > tolerance
      && (offset < 0.0 ? result_volume < source_volume - tolerance
                       : std::abs(result_volume - source_volume) > tolerance);
}

// The open shell OCCT's thick solid cannot make, as where an opening blends
// smoothly into rounds: the body less its inward offset (or its outward offset
// less the body), each planar opening cut through by its face swept across the
// wall. The opening keeps the outline of the face's image on the inner side.
std::unique_ptr<NativeOperationResult> open_shell_by_offset(
    const NativeOperationResult& body,
    const NCollection_List<TopoDS_Shape>& openings,
    rust::Slice<const std::uint32_t> opening_ordinals,
    double offset) {
  std::unique_ptr<BRepAlgoAPI_Defeaturing> defeaturing;
  const TopoDS_Shape source = offset < 0.0
      ? inward_offset_source(body.impl().shape, -offset, defeaturing)
      : body.impl().shape;
  if (source.IsNull()) {
    return error_result(STATUS_INVALID_SHAPE, OFFSET_FAILED);
  }
  BRepOffsetAPI_MakeOffsetShape offset_body;
  offset_body.PerformByJoin(
      source,
      offset,
      tolerances().approximation,
      BRepOffset_Skin,
      false,
      false,
      GeomAbs_Intersection,
      true);
  const TopoDS_Shape offset_shape = offset_solid(offset_body, body.impl().shape, std::abs(offset));
  if (offset_shape.IsNull()) {
    return error_result(STATUS_INVALID_SHAPE, OFFSET_FAILED);
  }
  const bool inward = offset < 0.0;
  const double thickness = std::abs(offset);
  const TopoDS_Shape& outer = inward ? body.impl().shape : offset_shape;
  const TopoDS_Shape& cavity = inward ? offset_shape : body.impl().shape;
  NCollection_List<TopoDS_Shape> tools;
  tools.Append(cavity);
  for (NCollection_List<TopoDS_Shape>::Iterator iterator(openings);
       iterator.More(); iterator.Next()) {
    const TopoDS_Face face = TopoDS::Face(iterator.Value());
    gp_Dir normal;
    if (!oriented_planar_face_normal(face, normal)) {
      return error_result(
          STATUS_INVALID_PARAMETER,
          "Shell could not open a curved face that blends into its neighbours; "
          "open a flat face");
    }
    TopoDS_Face mouth = face;
    if (inward) {
      // The offset's face in the plane of the opening moved in by the wall.
      const gp_Pnt on_opening = BRep_Tool::Pnt(TopExp::FirstVertex(
          TopoDS::Edge(TopExp_Explorer(face, TopAbs_EDGE).Current())));
      int found = 0;
      for (TopExp_Explorer candidates(offset_shape, TopAbs_FACE); candidates.More();
           candidates.Next()) {
        const TopoDS_Face candidate = TopoDS::Face(candidates.Current());
        gp_Dir candidate_normal;
        if (!oriented_planar_face_normal(candidate, candidate_normal)
            || candidate_normal.Dot(normal) < 1.0 - tolerances().approximation) {
          continue;
        }
        const gp_Pnt on_candidate = BRep_Tool::Pnt(TopExp::FirstVertex(
            TopoDS::Edge(TopExp_Explorer(candidate, TopAbs_EDGE).Current())));
        const double depth = gp_Vec(on_opening, on_candidate).Dot(gp_Vec(normal));
        if (std::abs(depth + thickness) <= tolerances().linear_mm) {
          mouth = candidate;
          ++found;
        }
      }
      if (found != 1) {
        return error_result(
            STATUS_NO_GEOMETRIC_CHANGE,
            "Shell opening has no single face on the inner side of the wall");
      }
    }
    // From the inner side of the wall to one wall past its outer side.
    BRepPrimAPI_MakePrism sweep(mouth, gp_Vec(normal) * (2.0 * thickness), true, false);
    if (!sweep.IsDone() || sweep.Shape().IsNull()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT shell opening sweep did not complete");
    }
    // Across the wall the sweep must stay in the body: one that leaves it
    // through a neighbouring wall would cut that wall as well.
    BRepAlgoAPI_Common crossing;
    configure_boolean(crossing, sweep.Shape(), outer);
    crossing.Build();
    GProp_GProps mouth_area;
    BRepGProp::SurfaceProperties(mouth, mouth_area);
    const double expected = mouth_area.Mass() * thickness;
    // Both volumes are measured by the kernel, so they agree to its approximation.
    if (!crossing.IsDone() || crossing.HasErrors() || crossing.Shape().IsNull()
        || !(std::abs(volume_of(crossing.Shape()) - expected)
             <= tolerances().approximation * std::max(expected, 1.0))) {
      return error_result(
          STATUS_NO_GEOMETRIC_CHANGE,
          "Shell opening would cut through a neighbouring wall: its walls lean over "
          "the opening by more than the thickness allows");
    }
    tools.Append(sweep.Shape());
  }
  // One tool: the cavity and its openings fused, the faces they share in a
  // plane merged, so the cut meets no face lying on another.
  NCollection_List<TopoDS_Shape> fuse_arguments;
  fuse_arguments.Append(cavity);
  tools.RemoveFirst();
  BRepAlgoAPI_Fuse fuse;
  fuse.SetArguments(fuse_arguments);
  fuse.SetTools(tools);
  fuse.SetNonDestructive(true);
  fuse.Build();
  if (!fuse.IsDone() || fuse.HasErrors() || fuse.Shape().IsNull()) {
    return error_result(STATUS_INVALID_SHAPE, "OCCT shell opening fuse did not complete");
  }
  fuse.SimplifyResult();
  BRepAlgoAPI_Cut cut;
  configure_boolean(cut, outer, fuse.Shape());
  cut.Build();
  if (!cut.IsDone() || cut.HasErrors() || cut.Shape().IsNull()
      || count_subshapes(cut.Shape(), TopAbs_SOLID) != 1
      || !shell_hollowed(body.impl().shape, cut.Shape(), offset)) {
    return error_result(STATUS_NO_GEOMETRIC_CHANGE, NOT_HOLLOWED);
  }

  const TopoDS_Shape result = cut.Shape();
  const ShapeIndex body_index(body.impl().shape);
  const ShapeIndex result_index(result);
  const std::string prefix = inward ? "shell_inward" : "shell_outward";
  std::vector<HistoryRecord> history;
  // A face of the outer body as it is, or of the cavity as it is in the tool.
  const auto before_cut = [&](const TopoDS_Shape& face, bool in_cavity) {
    NCollection_List<TopoDS_Shape> faces;
    if (!in_cavity) {
      faces.Append(face);
    } else if (!fuse.IsDeleted(face)) {
      faces = fuse.Modified(face);
      if (faces.IsEmpty()) faces.Append(face);
    }
    return faces;
  };
  // A face of the body or of its offset, carried through the cut.
  const auto carry = [&](const HistoryRecord& source, const TopoDS_Shape& face, bool in_cavity,
                         const std::string& relation) {
    const std::string role =
        source.semantic_role.empty() ? "" : relation + "(" + source.semantic_role + ")";
    const NCollection_List<TopoDS_Shape> faces = before_cut(face, in_cavity);
    for (NCollection_List<TopoDS_Shape>::Iterator tool_face(faces); tool_face.More();
         tool_face.Next()) {
      const NCollection_List<TopoDS_Shape>& modified = cut.Modified(tool_face.Value());
      for (NCollection_List<TopoDS_Shape>::Iterator image(modified); image.More(); image.Next()) {
        history.push_back(history_record(
            role, relation + "_modified", source.source_element_id, result_index, image.Value()));
      }
      if (modified.IsEmpty() && !cut.IsDeleted(tool_face.Value())) {
        const HistoryRecord unchanged = history_record(
            role, relation + "_unchanged", source.source_element_id, result_index,
            tool_face.Value());
        if (unchanged.output_present) history.push_back(unchanged);
      }
    }
  };
  for (const HistoryRecord& source : body.impl().history) {
    if (!source.output_present) continue;
    const TopoDS_Face face = body_index.face(source.output_ordinal);
    if (face.IsNull()) continue;
    carry(source, face, !inward, prefix + (inward ? "_outer" : "_inner"));
    const NCollection_List<TopoDS_Shape> images = offset_images(face, defeaturing, offset_body);
    for (NCollection_List<TopoDS_Shape>::Iterator image(images); image.More(); image.Next()) {
      carry(source, image.Value(), inward, prefix + (inward ? "_inner" : "_outer"));
    }
  }
  std::size_t index = 0;
  for (NCollection_List<TopoDS_Shape>::Iterator iterator(openings);
       iterator.More(); iterator.Next(), ++index) {
    const std::string source_id =
        "generated-result/face/" + std::to_string(opening_ordinals[index]);
    std::size_t rims = 0;
    const NCollection_List<TopoDS_Shape> faces = before_cut(iterator.Value(), !inward);
    for (NCollection_List<TopoDS_Shape>::Iterator tool_face(faces); tool_face.More();
         tool_face.Next()) {
      const NCollection_List<TopoDS_Shape>& modified = cut.Modified(tool_face.Value());
      for (NCollection_List<TopoDS_Shape>::Iterator rim(modified); rim.More(); rim.Next(), ++rims) {
        history.push_back(history_record(
            "", prefix + "_selected_modified", source_id, result_index, rim.Value()));
      }
    }
    if (rims == 0) {
      history.push_back(HistoryRecord{"", prefix + "_selected_removed", source_id, 0, false});
    }
  }
  return success_result(result, std::move(history));
}

}  // namespace

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
    const ShapeIndex body_index(body.impl().shape);
    NCollection_List<TopoDS_Shape> closing_faces;
    std::uint32_t previous = 0;
    bool first = true;
    for (const std::uint32_t ordinal : face_ordinals) {
      if ((!first && ordinal <= previous)
          || ordinal >= body.impl().summary.face_count) {
        return error_result(STATUS_INVALID_PARAMETER, "Body shell face ordinals are invalid or non-canonical");
      }
      const TopoDS_Face face = body_index.face(ordinal);
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
    const auto hollowed = [&](const TopoDS_Shape& shape, double offset) {
      return shell_hollowed(body.impl().shape, shape, offset);
    };
    const auto append_selected_history = [&](
        std::vector<HistoryRecord>& history,
        auto& operation,
        const ShapeIndex& result_index,
        const std::string& relation_prefix) {
      for (const std::uint32_t ordinal : face_ordinals) {
        const TopoDS_Face source = body_index.face(ordinal);
        const std::string source_id = "generated-result/face/" + std::to_string(ordinal);
        const NCollection_List<TopoDS_Shape>& modified = operation.Modified(source);
        const NCollection_List<TopoDS_Shape>& generated = operation.Generated(source);
        for (NCollection_List<TopoDS_Shape>::Iterator iterator(modified);
             iterator.More(); iterator.Next()) {
          history.push_back(history_record(
              "", relation_prefix + "_selected_modified", source_id, result_index, iterator.Value()));
        }
        for (NCollection_List<TopoDS_Shape>::Iterator iterator(generated);
             iterator.More(); iterator.Next()) {
          history.push_back(history_record(
              "", relation_prefix + "_selected_generated", source_id, result_index, iterator.Value()));
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
      std::unique_ptr<BRepAlgoAPI_Defeaturing> inward_defeaturing;
      std::unique_ptr<BRepAlgoAPI_Defeaturing> outward_defeaturing;
      const auto build_offset = [&](BRepOffsetAPI_MakeOffsetShape& operation, double offset) {
        const TopoDS_Shape source = offset < 0.0
            ? inward_offset_source(body.impl().shape, -offset, inward_defeaturing)
            : body.impl().shape;
        if (source.IsNull()) return TopoDS_Shape();
        operation.PerformByJoin(
            source,
            offset,
            tolerances().approximation,
            BRepOffset_Skin,
            false,
            false,
            GeomAbs_Intersection,
            true);
        return offset_solid(operation, body.impl().shape, std::abs(offset));
      };
      const double inward_distance = direction == 2 ? -thickness * 0.5 : -thickness;
      const double outward_distance = direction == 2 ? thickness * 0.5 : thickness;
      TopoDS_Shape inner = body.impl().shape;
      TopoDS_Shape outer = body.impl().shape;
      if (direction != 1) {
        inner = build_offset(inward_offset, inward_distance);
        if (inner.IsNull()) {
          return error_result(STATUS_INVALID_SHAPE, "OCCT closed Shell inward offset did not complete");
        }
      }
      if (direction != 0) {
        outer = build_offset(outward_offset, outward_distance);
        if (outer.IsNull()) {
          return error_result(STATUS_INVALID_SHAPE, "OCCT closed Shell outward offset did not complete");
        }
      }
      BRepAlgoAPI_Cut cut;
      configure_boolean(cut, outer, inner);
      cut.Build();
      if (!cut.IsDone() || cut.HasErrors() || cut.Shape().IsNull()
          || !BRepCheck_Analyzer(cut.Shape()).IsValid()
          || count_subshapes(cut.Shape(), TopAbs_SOLID) != 1) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT closed Shell boolean cut did not complete");
      }
      const TopoDS_Shape result = cut.Shape();
      const ShapeIndex result_index(result);
      std::vector<HistoryRecord> history;
      const auto map_intermediate = [&cut, &result_index](
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
              result_index,
              iterator.Value()));
        }
        for (NCollection_List<TopoDS_Shape>::Iterator iterator(generated);
             iterator.More(); iterator.Next()) {
          records.push_back(history_record(
              semantic_role,
              relation_prefix + "_generated",
              source.source_element_id,
              result_index,
              iterator.Value()));
        }
        if (!cut.IsDeleted(intermediate) && modified.IsEmpty() && generated.IsEmpty()) {
          const HistoryRecord unchanged = history_record(
              semantic_role,
              relation_prefix + "_unchanged",
              source.source_element_id,
              result_index,
              intermediate);
          if (unchanged.output_present) records.push_back(unchanged);
        }
      };
      const auto append_direct = [&](const std::string& relation_prefix) {
        for (const HistoryRecord& source : body.impl().history) {
          if (!source.output_present) continue;
          const TopoDS_Face source_face = body_index.face(source.output_ordinal);
          if (!source_face.IsNull()) map_intermediate(history, source, source_face, relation_prefix);
        }
      };
      const auto append_offset = [&](
          BRepOffsetAPI_MakeOffsetShape& offset,
          const std::unique_ptr<BRepAlgoAPI_Defeaturing>& defeaturing,
          const std::string& relation_prefix) {
        for (const HistoryRecord& source : body.impl().history) {
          if (!source.output_present) continue;
          const TopoDS_Face source_face = body_index.face(source.output_ordinal);
          if (source_face.IsNull()) continue;
          const NCollection_List<TopoDS_Shape> images =
              offset_images(source_face, defeaturing, offset);
          for (NCollection_List<TopoDS_Shape>::Iterator iterator(images);
               iterator.More(); iterator.Next()) {
            map_intermediate(history, source, iterator.Value(), relation_prefix);
          }
        }
      };
      if (direction == 0) {
        append_direct("shell_closed_inward_outer");
        append_offset(inward_offset, inward_defeaturing, "shell_closed_inward_inner");
      } else if (direction == 1) {
        append_offset(outward_offset, outward_defeaturing, "shell_closed_outward_outer");
        append_direct("shell_closed_outward_inner");
      } else {
        append_offset(outward_offset, outward_defeaturing, "shell_closed_symmetric_outer");
        append_offset(inward_offset, inward_defeaturing, "shell_closed_symmetric_inner");
      }
      return success_result(result, std::move(history));
    }

    if (direction != 2) {
      BRepOffsetAPI_MakeThickSolid operation;
      const double offset = direction == 0 ? -thickness : thickness;
      if (!build_thick(operation, offset) || !hollowed(operation.Shape(), offset)) {
        return open_shell_by_offset(body, closing_faces, face_ordinals, offset);
      }
      const TopoDS_Shape result = operation.Shape();
      const ShapeIndex result_index(result);
      std::vector<HistoryRecord> history;
      append_propagated_history(history, operation, result_index, body.impl());
      append_selected_history(
          history, operation, result_index, direction == 0 ? "shell_inward" : "shell_outward");
      return success_result(result, std::move(history));
    }

    BRepOffsetAPI_MakeThickSolid inward;
    BRepOffsetAPI_MakeThickSolid outward;
    if (!build_thick(inward, -thickness * 0.5)
        || !build_thick(outward, thickness * 0.5)) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT symmetric body shell halves did not complete");
    }
    if (!hollowed(inward.Shape(), -thickness * 0.5)
        || !hollowed(outward.Shape(), thickness * 0.5)) {
      return error_result(STATUS_NO_GEOMETRIC_CHANGE, NOT_HOLLOWED);
    }
    BRepAlgoAPI_Fuse fusion;
    configure_boolean(fusion, inward.Shape(), outward.Shape());
    fusion.Build();
    if (!fusion.IsDone() || fusion.HasErrors() || fusion.Shape().IsNull()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT symmetric body shell fuse did not complete");
    }
    fusion.SimplifyResult(true, true);
    const TopoDS_Shape result = fusion.Shape();
    const ShapeIndex result_index(result);
    std::vector<HistoryRecord> history;
    const auto append_half_history = [&](
        BRepOffsetAPI_MakeThickSolid& half,
        const std::string& relation_prefix) {
      std::vector<HistoryRecord> half_history;
      const ShapeIndex half_index(half.Shape());
      append_propagated_history(half_history, half, half_index, body.impl());
      append_selected_history(half_history, half, half_index, relation_prefix);
      for (const HistoryRecord& source : half_history) {
        const std::string semantic_role = source.semantic_role.empty()
            ? ""
            : relation_prefix + "(" + source.semantic_role + ")";
        if (!source.output_present) {
          history.push_back(source);
          continue;
        }
        const TopoDS_Face intermediate = half_index.face(source.output_ordinal);
        const NCollection_List<TopoDS_Shape>& modified = fusion.Modified(intermediate);
        if (modified.IsEmpty()) {
          HistoryRecord mapped = history_record(
              semantic_role,
              source.relation,
              source.source_element_id,
              result_index,
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
                result_index,
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
      const gp_Vec side_normal = du.Crossed(dv);
      const gp_Vec target_normal = tdu.Crossed(tdv);
      // A singular point (a cubic without a tangent at its end) has no normal to
      // compare; the side then stays unmerged rather than failing the offset.
      if (side_normal.Magnitude() <= gp::Resolution()
          || target_normal.Magnitude() <= gp::Resolution()) {
        return false;
      }
      same_sense += side_normal.Normalized().Dot(target_normal.Normalized());
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

// Why OCCT could not round a run of edges, in the user's terms.
const char* round_failure_cause(ChFiDS_ErrorStatus status) {
  switch (status) {
    case ChFiDS_WalkingFailure:
      return "the round runs out of room on a neighbouring face";
    case ChFiDS_StartsolFailure:
      return "no round of this radius touches both faces next to the edge";
    case ChFiDS_TwistedSurface:
      return "the round would fold over itself";
    default:
      return "OCCT could not build the round";
  }
}

// Names the selected edges whose round failed, why, and the largest radius
// that does fit (`largest_radius`, negative when not searched), so the user
// can lower the radius or leave those edges out instead of guessing.
std::string fillet_failure(
    const BRepFilletAPI_MakeFillet& operation,
    const std::vector<TopoDS_Edge>& selected,
    rust::Slice<const std::uint32_t> edge_ordinals,
    double radius,
    double largest_radius) {
  char radius_text[32];
  std::snprintf(radius_text, sizeof(radius_text), "%g", radius);
  std::string message = std::string("Fillet of radius ") + radius_text + " mm";
  std::vector<std::uint32_t> failed;
  const char* cause = nullptr;
  for (int index = 1; index <= operation.NbFaultyContours(); ++index) {
    const int contour = operation.FaultyContour(index);
    if (cause == nullptr) {
      cause = round_failure_cause(operation.StripeStatus(contour));
    }
    for (int edge = 1; edge <= operation.NbEdges(contour); ++edge) {
      for (std::size_t chosen = 0; chosen < selected.size(); ++chosen) {
        if (selected[chosen].IsSame(operation.Edge(contour, edge))
            && std::find(failed.begin(), failed.end(), edge_ordinals[chosen]) == failed.end()) {
          failed.push_back(edge_ordinals[chosen]);
        }
      }
    }
  }
  if (failed.empty()) {
    // OCCT failed where the rounds meet, not along one run: every selected
    // edge is a candidate.
    failed.assign(edge_ordinals.begin(), edge_ordinals.end());
    cause = operation.NbFaultyVertices() > 0
        ? "the rounds cannot be joined where these edges meet"
        : "the rounds leave no face between them";
  }
  std::sort(failed.begin(), failed.end());
  message += failed.size() == 1 ? " does not fit on edge " : " does not fit on edges ";
  for (std::size_t index = 0; index < failed.size(); ++index) {
    if (index > 0) message += ", ";
    message += std::to_string(failed[index]);
  }
  message += ": ";
  message += cause;
  if (largest_radius >= 0.0) message += largest_fit_hint(largest_radius, true);
  return message + "; use a smaller radius or fewer edges";
}

// Names the selected edges a chamfer could not bevel and the largest distance
// that does (OCCT reports no faulty run for a chamfer, so every edge is named).
std::string chamfer_failure(
    rust::Slice<const std::uint32_t> edge_ordinals, double distance, double largest_distance) {
  char distance_text[32];
  std::snprintf(distance_text, sizeof(distance_text), "%g", distance);
  std::string message = std::string("Chamfer of ") + distance_text + " mm";
  message += edge_ordinals.size() == 1 ? " does not fit on edge " : " does not fit on edges ";
  for (std::size_t index = 0; index < edge_ordinals.size(); ++index) {
    if (index > 0) message += ", ";
    message += std::to_string(edge_ordinals[index]);
  }
  message += ": the bevel runs past a face next to the edge or into another bevel";
  if (largest_distance >= 0.0) message += largest_fit_hint(largest_distance, false);
  return message + "; use a smaller distance or fewer edges";
}

bool finish_fits(
    const TopoDS_Shape& shape, const std::vector<TopoDS_Edge>& edges, double amount, bool fillet) {
  try {
    if (fillet) {
      BRepFilletAPI_MakeFillet operation(shape);
      for (const TopoDS_Edge& edge : edges) operation.Add(amount, edge);
      operation.Build();
      return operation.IsDone() && !operation.Shape().IsNull()
          && BRepCheck_Analyzer(operation.Shape()).IsValid();
    }
    BRepFilletAPI_MakeChamfer operation(shape);
    for (const TopoDS_Edge& edge : edges) operation.Add(amount, edge);
    operation.Build();
    return operation.IsDone() && !operation.Shape().IsNull()
        && BRepCheck_Analyzer(operation.Shape()).IsValid();
  } catch (const Standard_Failure&) {
    return false;
  }
}

}  // namespace

double largest_fitting_finish(
    const TopoDS_Shape& shape, const std::vector<TopoDS_Edge>& edges, double amount, bool fillet) {
  if (!std::isfinite(amount) || amount <= 0.0) return 0.0;
  // Steps of the third significant digit of `amount`: at most ten builds.
  const double step = std::pow(10.0, std::floor(std::log10(amount)) - 2.0);
  long long fits = 0;
  long long fails = static_cast<long long>(std::ceil(amount / step));
  while (fails - fits > 1) {
    const long long middle = fits + (fails - fits) / 2;
    if (finish_fits(shape, edges, static_cast<double>(middle) * step, fillet)) {
      fits = middle;
    } else {
      fails = middle;
    }
  }
  return static_cast<double>(fits) * step;
}

std::string largest_fit_hint(double largest, bool fillet) {
  const std::string what = fillet ? "radius" : "distance";
  if (largest <= 0.0) return "; no smaller " + what + " fits these edges either";
  char text[32];
  std::snprintf(text, sizeof(text), "%g", largest);
  return "; the largest " + what + " found to fit is " + text + " mm";
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
    const TopoDS_Face face = ShapeIndex(body.impl().shape).face(face_ordinal);
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
      const ShapeIndex result_index(result);
      std::vector<HistoryRecord> history;
      append_propagated_history(history, operation, result_index, body.impl());
      const NCollection_List<TopoDS_Shape>& modified = operation.Modified(face);
      for (NCollection_List<TopoDS_Shape>::Iterator iterator(modified);
           iterator.More(); iterator.Next()) {
        history.push_back(history_record(
            "", "face_offset_selected_modified", source_id, result_index, iterator.Value()));
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
    const ShapeIndex result_index(result);
    std::vector<HistoryRecord> history;
    append_propagated_history(history, operation, result_index, body.impl());
    const NCollection_List<TopoDS_Shape>& modified = operation.Modified(face);
    for (NCollection_List<TopoDS_Shape>::Iterator iterator(modified);
         iterator.More(); iterator.Next()) {
      history.push_back(history_record(
          "", "face_offset_selected_modified", source_id, result_index, iterator.Value()));
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
    const ShapeIndex body_index(body.impl().shape);
    std::vector<TopoDS_Edge> selected;
    selected.reserve(edge_ordinals.size());
    std::uint32_t previous = 0;
    bool first = true;
    for (const std::uint32_t ordinal : edge_ordinals) {
      if ((!first && ordinal <= previous)
          || ordinal >= body.impl().summary.edge_count) {
        return error_result(STATUS_INVALID_PARAMETER, "Body edge finish ordinals are invalid or non-canonical");
      }
      const TopoDS_Edge edge = body_index.edge(ordinal);
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
      const TopoDS_Face face = body_index.face(ordinal);
      if (face.IsNull()) {
        return error_result(STATUS_INVALID_SHAPE, "Advanced chamfer side face is absent");
      }
      selected_faces.push_back(face);
    }

    const auto collect_finished = [&](auto& operation) -> std::unique_ptr<NativeOperationResult> {
      if (!operation.IsDone() || operation.Shape().IsNull()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT body edge finish did not complete");
      }
      const TopoDS_Shape result = operation.Shape();
      const ShapeIndex result_index(result);
      std::vector<HistoryRecord> history;
      std::vector<EdgeHistoryRecord> edge_history;
      append_propagated_history(history, operation, result_index, body.impl());
      append_propagated_edge_history(edge_history, operation, result_index, body.impl());
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
              source_id, result_index, iterator.Value()));
        }
        for (NCollection_List<TopoDS_Shape>::Iterator iterator(generated);
             iterator.More(); iterator.Next()) {
          edge_history.push_back(edge_history_record(
              "", fillet ? "fillet_selected_generated" : "chamfer_selected_generated",
              source_id, result_index, iterator.Value()));
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
        // A run of edges that closes on itself (a circle, a tangent loop) has
        // no free end: a radius that ends unlike it starts would step there.
        const double end_radius = fillet_radius_stations[fillet_radius_stations.size() - 1];
        if (end_radius != amount) {
          for (int contour = 1; contour <= operation.NbContours(); ++contour) {
            if (operation.Closed(contour)) {
              return error_result(
                  STATUS_INVALID_PARAMETER,
                  "Variable fillet on a closed run of edges must end with the radius it "
                  "starts with");
            }
          }
        }
      }
      operation.Build();
      // A round OCCT finishes may still be no solid (rounds crossing where
      // they leave no face between them): treat it like one it could not make.
      if (!operation.IsDone() || operation.Shape().IsNull()
          || !BRepCheck_Analyzer(operation.Shape()).IsValid()) {
        const TopoDS_Shape full_round =
            fillet_radius_stations.empty() && selected.size() == body.impl().summary.edge_count
                ? full_round_of_convex_polyhedron(body.impl().shape, amount)
                : TopoDS_Shape();
        if (!full_round.IsNull()) {
          // Every face and edge of the body is rebuilt from primitives, so
          // none carries over; the selected edges are all consumed.
          std::vector<EdgeHistoryRecord> edge_history;
          for (const std::uint32_t ordinal : edge_ordinals) {
            edge_history.push_back(EdgeHistoryRecord{
                "", "fillet_selected_consumed",
                "generated-result/edge/" + std::to_string(ordinal), 0, false});
          }
          return success_result(full_round, {}, false, false, std::move(edge_history));
        }
        const double largest_radius = fillet_radius_stations.empty()
            ? largest_fitting_finish(body.impl().shape, selected, amount, true)
            : -1.0;
        return error_result(
            STATUS_INVALID_SHAPE,
            fillet_failure(operation, selected, edge_ordinals, amount, largest_radius));
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
    operation.Build();
    if (!operation.IsDone() || operation.Shape().IsNull()) {
      // Only an even chamfer is searched: an uneven one has two figures to trade.
      const double largest_distance = chamfer_mode == 0
          ? largest_fitting_finish(body.impl().shape, selected, amount, false)
          : -1.0;
      return error_result(
          STATUS_INVALID_SHAPE, chamfer_failure(edge_ordinals, amount, largest_distance));
    }
    return collect_finished(operation);
  });
}

// Tilts each selected face by `angle_degrees` about its line on the neutral plane.
// The pull direction points from the neutral face into the body; a positive angle
// leans the faces inward along it, so the body narrows away from the neutral plane.
// Only planes and cylinders along the pull are drafted; every refusal names its cause.
std::unique_ptr<NativeOperationResult> draft_body_native(
    const NativeOperationResult& body,
    rust::Slice<const std::uint32_t> face_ordinals,
    std::uint32_t neutral_ordinal,
    double angle_degrees) noexcept {
  return guarded([&] {
    if (!body.valid() || face_ordinals.empty() || face_ordinals.size() > 64
        || neutral_ordinal >= body.impl().summary.face_count
        || !std::isfinite(angle_degrees) || angle_degrees == 0.0
        || std::abs(angle_degrees) >= 90.0) {
      return error_result(STATUS_INVALID_PARAMETER, "Body draft payload is outside the bounded envelope");
    }
    const ShapeIndex body_index(body.impl().shape);
    const TopoDS_Face neutral = body_index.face(neutral_ordinal);
    gp_Dir neutral_normal;
    if (neutral.IsNull() || !oriented_planar_face_normal(neutral, neutral_normal)) {
      return error_result(STATUS_INVALID_PARAMETER,
          "draft_neutral_face_not_planar: the neutral face must be flat");
    }
    const gp_Dir pull = neutral_normal.Reversed();
    const gp_Pln neutral_plane(
        BRepAdaptor_Surface(neutral, false).Plane().Location(), neutral_normal);
    const double angular = tolerances().angular_rad;
    std::vector<TopoDS_Face> selected;
    std::uint32_t previous = 0;
    bool first = true;
    for (const std::uint32_t ordinal : face_ordinals) {
      if ((!first && ordinal <= previous) || ordinal >= body.impl().summary.face_count
          || ordinal == neutral_ordinal) {
        return error_result(STATUS_INVALID_PARAMETER,
            "Draft face ordinals are invalid, non-canonical or include the neutral face");
      }
      previous = ordinal;
      first = false;
      const TopoDS_Face face = body_index.face(ordinal);
      if (face.IsNull()) {
        return error_result(STATUS_INVALID_SHAPE, "Draft selected face is absent");
      }
      const BRepAdaptor_Surface surface(face, false);
      const std::string which = "face " + std::to_string(ordinal);
      if (surface.GetType() == GeomAbs_Plane) {
        if (surface.Plane().Axis().Direction().IsParallel(pull, angular)) {
          return error_result(STATUS_INVALID_PARAMETER,
              "draft_face_perpendicular_to_pull: " + which
              + " faces along the pull direction, so it has no wall to tilt");
        }
      } else if (surface.GetType() == GeomAbs_Cylinder) {
        if (!surface.Cylinder().Axis().Direction().IsParallel(pull, angular)) {
          return error_result(STATUS_INVALID_PARAMETER,
              "draft_cylinder_axis_not_along_pull: the axis of cylindrical " + which
              + " is not along the pull direction");
        }
      } else {
        return error_result(STATUS_INVALID_PARAMETER,
            "draft_unsupported_surface: " + which + " is neither flat nor cylindrical");
      }
      BRepAlgoAPI_Section section(face, neutral_plane, false);
      section.Build();
      if (!section.IsDone() || section.HasErrors()
          || (count_subshapes(section.Shape(), TopAbs_EDGE) == 0
              && count_subshapes(section.Shape(), TopAbs_VERTEX) == 0)) {
        return error_result(STATUS_INVALID_PARAMETER,
            "draft_neutral_plane_misses_face: the neutral plane neither crosses nor touches "
            + which);
      }
      selected.push_back(face);
    }

    constexpr double DEGREES_TO_RADIANS = 3.14159265358979323846 / 180.0;
    BRepOffsetAPI_DraftAngle draft(body.impl().shape);
    bool built = true;
    try {
      for (const TopoDS_Face& face : selected) {
        draft.Add(face, pull, angle_degrees * DEGREES_TO_RADIANS, neutral_plane);
        built = built && draft.AddDone();
      }
      if (built) draft.Build();
    } catch (const Standard_Failure&) {
      // OCCT throws when a tilted face no longer meets its neighbours.
      built = false;
    }
    if (!built || !draft.IsDone() || draft.Shape().IsNull()
        || !BRepCheck_Analyzer(draft.Shape()).IsValid()
        || count_subshapes(draft.Shape(), TopAbs_SOLID) != 1
        || count_subshapes(draft.Shape(), TopAbs_FACE)
            != count_subshapes(body.impl().shape, TopAbs_FACE)) {
      return error_result(STATUS_DEGENERATE_OPERATION,
          "draft_face_degenerates: tilting the faces by this angle would collapse a face "
          "or the body");
    }
    const TopoDS_Shape result = draft.Shape();
    const ShapeIndex result_index(result);
    std::vector<HistoryRecord> history;
    append_propagated_history(history, draft, result_index, body.impl());
    for (std::size_t index = 0; index < selected.size(); ++index) {
      const std::string source_id = "generated-result/face/" + std::to_string(face_ordinals[index]);
      const NCollection_List<TopoDS_Shape>& modified = draft.Modified(selected[index]);
      for (NCollection_List<TopoDS_Shape>::Iterator iterator(modified);
           iterator.More(); iterator.Next()) {
        history.push_back(history_record(
            "", "draft_selected_modified", source_id, result_index, iterator.Value()));
      }
    }
    return success_result(result, std::move(history));
  });
}

std::unique_ptr<NativeOperationResult> exception_probe_native() noexcept {
  return guarded([]() -> std::unique_ptr<NativeOperationResult> {
    throw Standard_Failure("intentional A0 exception-boundary probe");
    return error_result(STATUS_BACKEND_EXCEPTION, "unreachable");
  });
}

} // namespace ketchup::exact
