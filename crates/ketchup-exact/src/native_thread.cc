#include "native_common.hxx"

#include <BRepBuilderAPI_MakePolygon.hxx>
#include <GCE2d_MakeSegment.hxx>
#include <Geom2d_TrimmedCurve.hxx>
#include <Geom_CylindricalSurface.hxx>
#include <gp_Ax3.hxx>

namespace ketchup::exact {
namespace {

// A built groove: the solid and, per section edge, the faces it swept.
struct Groove {
  TopoDS_Shape solid;
  std::vector<std::vector<TopoDS_Shape>> sides;
};

// The groove of a thread: the closed `profile` (radius from the axis,
// offset along it) carried by the screw motion about the axis through
// `helix[0..3]` along `helix[3..6]`, `helix[9]` (the pitch) per turn. At
// axial position s from the origin the profile lies in the half-plane at
// angle 2*pi*s/pitch from `helix[6..9]` (negative for a left-handed thread),
// so a part that knows the origin and reference knows the thread's phase.
// The helix runs from `helix[10]` to `helix[11]` along the axis;
// `approximation` bounds the swept faces' deviation. Null on success, the
// error otherwise.
std::unique_ptr<NativeOperationResult> make_groove(
    rust::Slice<const double> helix,
    bool left_handed,
    rust::Slice<const NativePoint> profile,
    double approximation,
    Groove& groove) {
  const double epsilon = tolerances().rounding;
  const double tau = 2.0 * std::acos(-1.0);
  constexpr double coordinate_limit = 1000000.0;
  if (helix.size() != 12 || profile.size() < 3 || profile.size() > 64) {
    return error_result(STATUS_INVALID_PARAMETER, "OCCT thread groove payload is malformed");
  }
  for (double value : helix) {
    if (!std::isfinite(value) || std::abs(value) > coordinate_limit) {
      return error_result(
          STATUS_NON_FINITE_PARAMETER, "OCCT thread groove helix is outside its bounded contract");
    }
  }
  const gp_Vec axis(helix[3], helix[4], helix[5]);
  const gp_Vec reference(helix[6], helix[7], helix[8]);
  const double pitch = helix[9];
  const double start = helix[10];
  const double end = helix[11];
  if (std::abs(axis.Magnitude() - 1.0) > epsilon || std::abs(reference.Magnitude() - 1.0) > epsilon
      || std::abs(axis.Dot(reference)) > epsilon) {
    return error_result(
        STATUS_INVALID_PARAMETER,
        "OCCT thread groove axis and reference are not square unit directions");
  }
  if (!std::isfinite(approximation) || pitch <= tolerances().linear_mm || end <= start
      || approximation <= 0.0 || approximation >= pitch / 16.0) {
    return error_result(
        STATUS_INVALID_PARAMETER, "OCCT thread groove pitch or range is outside its bounded contract");
  }
  if ((end - start) / pitch > 4096.0) {
    return error_result(STATUS_INVALID_PARAMETER, "OCCT thread groove has too many turns");
  }
  double minimum_radius = std::numeric_limits<double>::infinity();
  double maximum_radius = 0.0;
  double minimum_offset = std::numeric_limits<double>::infinity();
  double maximum_offset = -std::numeric_limits<double>::infinity();
  for (const NativePoint& point : profile) {
    if (!std::isfinite(point.x) || !std::isfinite(point.y) || point.z != 0.0
        || point.x <= tolerances().linear_mm || point.x > coordinate_limit
        || std::abs(point.y) > coordinate_limit) {
      return error_result(
          STATUS_INVALID_PARAMETER, "OCCT thread groove profile is outside its bounded contract");
    }
    minimum_radius = std::min(minimum_radius, point.x);
    maximum_radius = std::max(maximum_radius, point.x);
    minimum_offset = std::min(minimum_offset, point.y);
    maximum_offset = std::max(maximum_offset, point.y);
  }
  // Neighbouring turns of a groove at least a pitch wide would touch.
  if (maximum_offset - minimum_offset >= pitch - epsilon) {
    return error_result(
        STATUS_INVALID_PARAMETER, "OCCT thread groove profile is not narrower than the pitch");
  }

  const double sign = left_handed ? -1.0 : 1.0;
  const gp_Pnt origin(helix[0], helix[1], helix[2]);
  const gp_Ax3 placement(origin, gp_Dir(axis), gp_Dir(reference));
  occ::handle<Geom_CylindricalSurface> cylinder =
      new Geom_CylindricalSurface(placement, (minimum_radius + maximum_radius) / 2.0);
  // One edge per turn: a Boolean splits a face that winds round many times
  // unreliably, so every swept face covers one turn at most.
  const auto turns = static_cast<std::size_t>(std::ceil((end - start) / pitch - epsilon));
  BRepBuilderAPI_MakeWire spine_builder;
  for (std::size_t turn = 0; turn < turns; ++turn) {
    const double from = start + pitch * static_cast<double>(turn);
    const double to = turn + 1 == turns ? end : from + pitch;
    GCE2d_MakeSegment segment(
        gp_Pnt2d(sign * tau * from / pitch, from), gp_Pnt2d(sign * tau * to / pitch, to));
    if (!segment.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT thread groove helix is degenerate");
    }
    BRepBuilderAPI_MakeEdge helix_builder(segment.Value(), cylinder);
    if (!helix_builder.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT thread groove helix edge failed");
    }
    TopoDS_Edge helix = helix_builder.Edge();
    if (!BRepLib::BuildCurves3d(helix, approximation, GeomAbs_C2, 14, 10000)) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT thread groove helix curve failed");
    }
    spine_builder.Add(helix);
  }
  if (!spine_builder.IsDone()) {
    return error_result(STATUS_INVALID_SHAPE, "OCCT thread groove spine wire failed");
  }

  const double start_angle = sign * tau * start / pitch;
  const gp_Vec radial = reference.Multiplied(std::cos(start_angle))
                            .Added(axis.Crossed(reference).Multiplied(std::sin(start_angle)));
  const gp_Pnt section_origin = origin.Translated(axis.Multiplied(start));
  BRepBuilderAPI_MakePolygon polygon;
  for (const NativePoint& point : profile) {
    polygon.Add(
        section_origin.Translated(radial.Multiplied(point.x).Added(axis.Multiplied(point.y))));
  }
  polygon.Close();
  if (!polygon.IsDone() || !BRepCheck_Analyzer(polygon.Wire()).IsValid()) {
    return error_result(STATUS_INVALID_SHAPE, "OCCT thread groove profile wire is invalid");
  }
  const TopoDS_Wire section = polygon.Wire();

  BRepOffsetAPI_MakePipeShell operation(spine_builder.Wire());
  // A fixed binormal along the axis keeps the profile in the axial
  // half-plane: every turn is the same screw motion of the same section.
  operation.SetMode(gp_Dir(axis));
  operation.SetTolerance(approximation, approximation, tolerances().angular_rad);
  operation.Add(section, false, false);
  if (!operation.IsReady()) {
    return error_result(STATUS_INVALID_SHAPE, "OCCT thread groove pipe is not ready");
  }
  operation.Build();
  if (!operation.IsDone() || !operation.MakeSolid()) {
    return error_result(STATUS_INVALID_SHAPE, "OCCT thread groove did not produce a solid");
  }
  TopoDS_Shape result = operation.Shape();
  // The pipe's solid can come out inside-out; a Boolean would then take the
  // groove for everything around it.
  if (!result.IsNull() && result.ShapeType() == TopAbs_SOLID) {
    TopoDS_Solid solid = TopoDS::Solid(result);
    if (!BRepLib::OrientClosedSolid(solid)) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT thread groove solid cannot be oriented");
    }
    result = solid;
  }
  if (result.IsNull() || !BRepCheck_Analyzer(result).IsValid()
      || count_subshapes(result, TopAbs_SOLID) != 1) {
    return error_result(STATUS_INVALID_SHAPE, "OCCT thread groove is not one valid solid");
  }
  groove.solid = result;
  for (TopExp_Explorer edges(section, TopAbs_EDGE); edges.More(); edges.Next()) {
    std::vector<TopoDS_Shape> faces;
    const NCollection_List<TopoDS_Shape>& generated = operation.Generated(edges.Current());
    for (NCollection_List<TopoDS_Shape>::Iterator face(generated); face.More(); face.Next()) {
      if (face.Value().ShapeType() == TopAbs_FACE) faces.push_back(face.Value());
    }
    groove.sides.push_back(std::move(faces));
  }
  return nullptr;
}

std::string side_role(std::size_t index) {
  return "thread.side." + std::to_string(index);
}

// The solid cylinder of `radius` about the axis of `helix`, from `from` to
// `to` along it.
TopoDS_Shape axial_cylinder(
    rust::Slice<const double> helix, double radius, double from, double to) {
  const gp_Dir axis(helix[3], helix[4], helix[5]);
  const gp_Dir reference(helix[6], helix[7], helix[8]);
  const gp_Pnt base =
      gp_Pnt(helix[0], helix[1], helix[2]).Translated(gp_Vec(axis).Multiplied(from));
  return BRepPrimAPI_MakeCylinder(gp_Ax2(base, axis, reference), radius, to - from).Shape();
}

// `faces` of `shape` as `operation` left them in its result.
template <typename Operation>
std::vector<TopoDS_Shape> kept_faces(Operation& operation, const std::vector<TopoDS_Shape>& faces) {
  std::vector<TopoDS_Shape> kept;
  for (const TopoDS_Shape& face : faces) {
    if (operation.IsDeleted(face)) continue;
    const NCollection_List<TopoDS_Shape>& modified = operation.Modified(face);
    if (modified.IsEmpty()) {
      kept.push_back(face);
    }
    for (NCollection_List<TopoDS_Shape>::Iterator iterator(modified); iterator.More();
         iterator.Next()) {
      kept.push_back(iterator.Value());
    }
  }
  return kept;
}

// The faces of `shape` on a surface of `kind`.
std::vector<TopoDS_Shape> faces_of_kind(const TopoDS_Shape& shape, GeomAbs_SurfaceType kind) {
  std::vector<TopoDS_Shape> faces;
  for (TopExp_Explorer explorer(shape, TopAbs_FACE); explorer.More(); explorer.Next()) {
    if (BRepAdaptor_Surface(TopoDS::Face(explorer.Current())).GetType() == kind) {
      faces.push_back(explorer.Current());
    }
  }
  return faces;
}

// Ends the groove at `from` and `to` along the axis: flat, square to it.
// Null on success, the error otherwise; `ends` are the flat faces.
std::unique_ptr<NativeOperationResult> trim_groove(
    rust::Slice<const double> helix,
    double from,
    double to,
    double reach,
    Groove& groove,
    std::vector<TopoDS_Shape>& ends) {
  const TopoDS_Shape slab = axial_cylinder(helix, 2.0 * reach, from, to);
  BRepAlgoAPI_Common operation;
  configure_boolean(operation, groove.solid, slab);
  operation.SetRunParallel(true);
  operation.Build();
  if (!operation.IsDone() || operation.HasErrors() || operation.Shape().IsNull()
      || count_subshapes(operation.Shape(), TopAbs_SOLID) != 1) {
    return error_result(STATUS_INVALID_SHAPE, "OCCT thread groove trim did not complete");
  }
  for (std::vector<TopoDS_Shape>& side : groove.sides) {
    side = kept_faces(operation, side);
  }
  ends = kept_faces(operation, faces_of_kind(slab, GeomAbs_Plane));
  groove.solid = operation.Shape();
  return nullptr;
}

}  // namespace

std::unique_ptr<NativeOperationResult> thread_groove_native(
    rust::Slice<const double> helix,
    bool left_handed,
    rust::Slice<const NativePoint> profile,
    double approximation) noexcept {
  return guarded([&] {
    Groove groove;
    if (auto error = make_groove(helix, left_handed, profile, approximation, groove)) {
      return error;
    }
    std::vector<HistoryRecord> history;
    for (std::size_t index = 0; index < groove.sides.size(); ++index) {
      for (const TopoDS_Shape& face : groove.sides[index]) {
        history.push_back(
            history_record(side_role(index), "generated", side_role(index), groove.solid, face));
      }
    }
    return success_result(groove.solid, std::move(history));
  });
}

// `target` with the groove of `thread_groove_native` cut out of it. The
// thread's faces are named `thread.side.<section edge>`. A `trim` of two
// values ends the thread flat at those offsets along the axis
// (`thread.end`); two more first turn the stretch between them down to the
// crest radius from the outer one (`thread.crest`), as a bolt's thread is
// turned down from its shank.
std::unique_ptr<NativeOperationResult> thread_cut_native(
    const NativeOperationResult& target,
    rust::Slice<const double> helix,
    bool left_handed,
    rust::Slice<const NativePoint> profile,
    rust::Slice<const double> trim,
    double approximation) noexcept {
  return guarded([&] {
    if (!target.valid() || target.impl().shape.IsNull()) {
      return error_result(STATUS_INVALID_PARAMETER, "Thread cut target body is unavailable");
    }
    Groove groove;
    if (auto error = make_groove(helix, left_handed, profile, approximation, groove)) {
      return error;
    }
    if (trim.size() != 0 && trim.size() != 2 && trim.size() != 4) {
      return error_result(STATUS_INVALID_PARAMETER, "OCCT thread trim payload is malformed");
    }
    for (double value : trim) {
      if (!std::isfinite(value)) {
        return error_result(STATUS_NON_FINITE_PARAMETER, "OCCT thread trim is not finite");
      }
    }
    double reach = 0.0;
    for (const NativePoint& point : profile) reach = std::max(reach, point.x);
    std::vector<TopoDS_Shape> ends;
    const NativeOperationResult::Impl* source = &target.impl();
    NativeOperationResult::Impl turned;
    std::vector<TopoDS_Shape> crest;
    if (trim.size() >= 2) {
      const double from = trim[0];
      const double to = trim[1];
      if (!(helix[10] < from && from < to && to < helix[11])) {
        return error_result(
            STATUS_INVALID_PARAMETER, "OCCT thread trim lies outside the thread's helix");
      }
      if (trim.size() == 4) {
        const double crest_radius = trim[2];
        const double outer_radius = trim[3];
        if (!(crest_radius > tolerances().linear_mm && crest_radius < outer_radius)) {
          return error_result(STATUS_INVALID_PARAMETER, "OCCT thread crest radii are invalid");
        }
        reach = std::max(reach, outer_radius);
        const double margin = helix[9];
        BRepAlgoAPI_Cut ring;
        configure_boolean(
            ring,
            axial_cylinder(helix, outer_radius, from, to),
            axial_cylinder(helix, crest_radius, from - margin, to + margin));
        ring.Build();
        if (!ring.IsDone() || ring.HasErrors() || ring.Shape().IsNull()) {
          return error_result(STATUS_INVALID_SHAPE, "OCCT thread crest ring did not complete");
        }
        BRepAlgoAPI_Cut turn;
        configure_boolean(turn, target.impl().shape, ring.Shape());
        turn.Build();
        if (!turn.IsDone() || turn.HasErrors() || turn.Shape().IsNull()
            || count_subshapes(turn.Shape(), TopAbs_SOLID) != 1) {
          return error_result(STATUS_INVALID_SHAPE, "OCCT thread crest turning did not complete");
        }
        std::vector<TopoDS_Shape> crest_faces;
        for (const TopoDS_Shape& face : faces_of_kind(ring.Shape(), GeomAbs_Cylinder)) {
          if (std::abs(BRepAdaptor_Surface(TopoDS::Face(face)).Cylinder().Radius() - crest_radius)
              <= tolerances().linear_mm) {
            crest_faces.push_back(face);
          }
        }
        crest = kept_faces(turn, crest_faces);
        turned.shape = turn.Shape();
        append_propagated_history(turned.history, turn, turned.shape, target.impl());
        source = &turned;
      }
      if (auto error = trim_groove(helix, from, to, reach, groove, ends)) {
        return error;
      }
    }
    BRepAlgoAPI_Cut operation;
    configure_boolean(operation, source->shape, groove.solid);
    // A thread meets its body once per turn and section edge; those
    // intersections are independent, and in parallel the cut takes a third
    // of the time.
    operation.SetRunParallel(true);
    operation.Build();
    if (!operation.IsDone() || operation.HasErrors()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT thread cut did not complete");
    }
    const TopoDS_Shape result = operation.Shape();
    if (result.IsNull() || count_subshapes(result, TopAbs_SOLID) != 1) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT thread cut is not one solid");
    }
    std::vector<HistoryRecord> history;
    append_propagated_history(history, operation, result, *source);
    // A face wholly inside the body is kept as it is.
    const auto name = [&](const std::string& role, const std::vector<TopoDS_Shape>& faces) {
      for (const TopoDS_Shape& face : kept_faces(operation, faces)) {
        HistoryRecord record = history_record(role, "generated", role, result, face);
        if (record.output_present) history.push_back(std::move(record));
      }
    };
    for (std::size_t index = 0; index < groove.sides.size(); ++index) {
      name(side_role(index), groove.sides[index]);
    }
    name("thread.end", ends);
    name("thread.crest", crest);
    return success_result(result, std::move(history));
  });
}

}  // namespace ketchup::exact
