#include "native_common.hxx"

namespace ketchup::exact {


// The tolerances and numeric guards of ketchup-tolerance; native code compares
// geometry only against these values.
const NativeTolerances& tolerances() noexcept {
  static const NativeTolerances values = native_tolerances();
  return values;
}

std::uint32_t count_subshapes(const TopoDS_Shape& shape, TopAbs_ShapeEnum kind) {
  std::uint32_t count = 0;
  for (TopExp_Explorer explorer(shape, kind); explorer.More(); explorer.Next()) {
    ++count;
  }
  return count;
}

TopoDS_Face face_at_ordinal(const TopoDS_Shape& shape, std::uint32_t target) {
  TopTools_IndexedMapOfShape faces;
  TopExp::MapShapes(shape, TopAbs_FACE, faces);
  const Standard_Integer index = static_cast<Standard_Integer>(target) + 1;
  return index <= faces.Extent() ? TopoDS::Face(faces(index)) : TopoDS_Face();
}

TopoDS_Edge edge_at_ordinal(const TopoDS_Shape& shape, std::uint32_t target) {
  TopTools_IndexedMapOfShape edges;
  TopExp::MapShapes(shape, TopAbs_EDGE, edges);
  const Standard_Integer index = static_cast<Standard_Integer>(target) + 1;
  return index <= edges.Extent() ? TopoDS::Edge(edges(index)) : TopoDS_Edge();
}

std::pair<std::uint32_t, bool> face_ordinal(
    const TopoDS_Shape& result, const TopoDS_Shape& candidate) {
  TopTools_IndexedMapOfShape faces;
  TopExp::MapShapes(result, TopAbs_FACE, faces);
  const Standard_Integer index = faces.FindIndex(candidate);
  return index > 0
             ? std::pair<std::uint32_t, bool>{static_cast<std::uint32_t>(index - 1), true}
             : std::pair<std::uint32_t, bool>{0, false};
}

HistoryRecord history_record(
    std::string role,
    std::string relation,
    std::string source,
    const TopoDS_Shape& result,
    const TopoDS_Shape& output) {
  const auto [ordinal, present] = face_ordinal(result, output);
  return HistoryRecord{
      std::move(role), std::move(relation), std::move(source), ordinal, present};
}

EdgeHistoryRecord edge_history_record(
    std::string role,
    std::string relation,
    std::string source,
    const TopoDS_Shape& result,
    const TopoDS_Shape& output) {
  TopTools_IndexedMapOfShape edges;
  TopExp::MapShapes(result, TopAbs_EDGE, edges);
  const Standard_Integer index = edges.FindIndex(output);
  if (index > 0) {
    return EdgeHistoryRecord{
        std::move(role), std::move(relation), std::move(source),
        static_cast<std::uint32_t>(index - 1), true};
  }
  return EdgeHistoryRecord{
      std::move(role), std::move(relation), std::move(source), 0, false};
}

std::string standard_failure_message(const Standard_Failure& failure) {
  const char* message = failure.what();
  return message == nullptr ? "OCCT Standard_Failure without a message" : message;
}

std::unique_ptr<NativeOperationResult> error_result(
    std::uint8_t status, std::string diagnostic) noexcept {
  try {
    auto impl = std::make_unique<NativeOperationResult::Impl>();
    impl->status = status;
    impl->diagnostic = std::move(diagnostic);
    return std::make_unique<NativeOperationResult>(std::move(impl));
  } catch (...) {
    return nullptr;
  }
}

bool oriented_planar_face_normal(const TopoDS_Face& face, gp_Dir& normal) {
  double u0, u1, v0, v1;
  BRepTools::UVBounds(face, u0, u1, v0, v1);
  if (!std::isfinite(u0) || !std::isfinite(u1) || !std::isfinite(v0)
      || !std::isfinite(v1) || u1 <= u0 || v1 <= v0) {
    return false;
  }
  const auto surface = BRep_Tool::Surface(face);
  const occ::handle<Geom_Surface> trimmed =
      new Geom_RectangularTrimmedSurface(surface, u0, u1, v0, v1);
  const GeomLib_IsPlanarSurface planar(trimmed, tolerances().linear_mm);
  if (!planar.IsPlanar()) {
    return false;
  }
  gp_Pnt point;
  gp_Vec du, dv;
  surface->D1((u0 + u1) * 0.5, (v0 + v1) * 0.5, point, du, dv);
  const gp_Vec cross = du.Crossed(dv);
  if (cross.SquareMagnitude() <= tolerances().negligible * tolerances().negligible) {
    return false;
  }
  normal = gp_Dir(cross);
  if (face.Orientation() == TopAbs_REVERSED) {
    normal.Reverse();
  }
  return true;
}

NativeFaceEvidence inspect_face(const TopoDS_Face& face, std::uint32_t ordinal) {
  GProp_GProps properties;
  BRepGProp::SurfaceProperties(face, properties);
  const gp_Pnt centre = properties.CentreOfMass();

  Bnd_Box bounds;
  BRepBndLib::Add(face, bounds);
  double min_x = 0.0;
  double min_y = 0.0;
  double min_z = 0.0;
  double max_x = 0.0;
  double max_y = 0.0;
  double max_z = 0.0;
  bounds.Get(min_x, min_y, min_z, max_x, max_y, max_z);

  std::string surface_kind = "other";
  double normal_x = 0.0;
  double normal_y = 0.0;
  double normal_z = 0.0;
  bool has_axis = false;
  double axis_origin_x = 0.0;
  double axis_origin_y = 0.0;
  double axis_origin_z = 0.0;
  double axis_direction_x = 0.0;
  double axis_direction_y = 0.0;
  double axis_direction_z = 0.0;
  gp_Dir normal;
  if (oriented_planar_face_normal(face, normal)) {
    surface_kind = "plane";
    normal_x = normal.X();
    normal_y = normal.Y();
    normal_z = normal.Z();
  } else {
    BRepAdaptor_Surface surface(face);
    if (surface.GetType() == GeomAbs_Cylinder) {
      surface_kind = "cylinder";
      const gp_Ax1 axis = surface.Cylinder().Axis();
      const gp_Pnt origin = axis.Location();
      const gp_Dir direction = axis.Direction();
      has_axis = true;
      axis_origin_x = origin.X();
      axis_origin_y = origin.Y();
      axis_origin_z = origin.Z();
      axis_direction_x = direction.X();
      axis_direction_y = direction.Y();
      axis_direction_z = direction.Z();
    }
  }

  return NativeFaceEvidence{
      ordinal,
      rust::String(surface_kind),
      properties.Mass(),
      centre.X(),
      centre.Y(),
      centre.Z(),
      normal_x,
      normal_y,
      normal_z,
      has_axis,
      axis_origin_x,
      axis_origin_y,
      axis_origin_z,
      axis_direction_x,
      axis_direction_y,
      axis_direction_z,
      min_x,
      min_y,
      min_z,
      max_x,
      max_y,
      max_z,
      [&face] {
        TopTools_IndexedMapOfShape edges;
        TopExp::MapShapes(face, TopAbs_EDGE, edges);
        return static_cast<std::uint32_t>(edges.Extent());
      }()};
}

NativeEdgeEvidence inspect_edge(const TopoDS_Edge& edge, std::uint32_t ordinal) {
  GProp_GProps properties;
  BRepGProp::LinearProperties(edge, properties);
  const gp_Pnt centroid = properties.CentreOfMass();

  Bnd_Box bounds;
  BRepBndLib::Add(edge, bounds);
  double min_x = 0.0;
  double min_y = 0.0;
  double min_z = 0.0;
  double max_x = 0.0;
  double max_y = 0.0;
  double max_z = 0.0;
  bounds.Get(min_x, min_y, min_z, max_x, max_y, max_z);

  BRepAdaptor_Curve curve(edge);
  std::string curve_kind = "other";
  switch (curve.GetType()) {
    case GeomAbs_Line: curve_kind = "line"; break;
    case GeomAbs_Circle: curve_kind = "circle"; break;
    case GeomAbs_Ellipse: curve_kind = "ellipse"; break;
    case GeomAbs_Hyperbola: curve_kind = "hyperbola"; break;
    case GeomAbs_Parabola: curve_kind = "parabola"; break;
    case GeomAbs_BezierCurve: curve_kind = "bezier"; break;
    case GeomAbs_BSplineCurve: curve_kind = "bspline"; break;
    case GeomAbs_OffsetCurve: curve_kind = "offset"; break;
    default: break;
  }

  bool has_circle = false;
  bool has_axis = false;
  double circle_radius_mm = 0.0;
  double axis_origin_x = 0.0;
  double axis_origin_y = 0.0;
  double axis_origin_z = 0.0;
  double axis_direction_x = 0.0;
  double axis_direction_y = 0.0;
  double axis_direction_z = 0.0;
  if (curve.GetType() == GeomAbs_Line) {
    const gp_Pnt start = curve.Value(curve.FirstParameter());
    const gp_Pnt end = curve.Value(curve.LastParameter());
    const gp_Dir direction(gp_Vec(start, end));
    has_axis = true;
    axis_origin_x = start.X();
    axis_origin_y = start.Y();
    axis_origin_z = start.Z();
    axis_direction_x = direction.X();
    axis_direction_y = direction.Y();
    axis_direction_z = direction.Z();
  } else if (curve.GetType() == GeomAbs_Circle) {
    const gp_Circ circle = curve.Circle();
    const gp_Ax1 axis = circle.Axis();
    has_circle = true;
    has_axis = true;
    circle_radius_mm = circle.Radius();
    axis_origin_x = axis.Location().X();
    axis_origin_y = axis.Location().Y();
    axis_origin_z = axis.Location().Z();
    axis_direction_x = axis.Direction().X();
    axis_direction_y = axis.Direction().Y();
    axis_direction_z = axis.Direction().Z();
  }

  return NativeEdgeEvidence{
      ordinal,
      rust::String(curve_kind),
      properties.Mass(),
      centroid.X(),
      centroid.Y(),
      centroid.Z(),
      min_x,
      min_y,
      min_z,
      max_x,
      max_y,
      max_z,
      BRep_Tool::IsClosed(edge),
      has_circle,
      has_axis,
      circle_radius_mm,
      axis_origin_x,
      axis_origin_y,
      axis_origin_z,
      axis_direction_x,
      axis_direction_y,
      axis_direction_z};
}

std::unique_ptr<NativeOperationResult> success_result(
    TopoDS_Shape shape,
    std::vector<HistoryRecord> history,
    bool allow_multi_solid,
    bool allow_planar_face,
    std::vector<EdgeHistoryRecord> edge_history,
    bool allow_surface) {
  auto impl = std::make_unique<NativeOperationResult::Impl>();
  impl->shape = std::move(shape);
  impl->history = std::move(history);
  impl->edge_history = std::move(edge_history);

  if (impl->shape.IsNull()) {
    impl->status = STATUS_NULL_RESULT;
    impl->diagnostic = "OCCT returned a null shape";
    return std::make_unique<NativeOperationResult>(std::move(impl));
  }

  const BRepCheck_Analyzer analyzer(impl->shape, true);
  const std::uint32_t solids = count_subshapes(impl->shape, TopAbs_SOLID);
  GProp_GProps properties;
  BRepGProp::VolumeProperties(impl->shape, properties);
  const double volume = (allow_planar_face || allow_surface) && solids == 0
      ? 0.0
      : properties.Mass();

  TopTools_IndexedMapOfShape vertices;
  TopTools_IndexedMapOfShape edges;
  TopTools_IndexedMapOfShape faces;
  TopExp::MapShapes(impl->shape, TopAbs_VERTEX, vertices);
  TopExp::MapShapes(impl->shape, TopAbs_EDGE, edges);
  TopExp::MapShapes(impl->shape, TopAbs_FACE, faces);

  Bnd_Box bounds;
  BRepBndLib::AddOptimal(impl->shape, bounds, false, false);
  double min_x = 0.0;
  double min_y = 0.0;
  double min_z = 0.0;
  double max_x = 0.0;
  double max_y = 0.0;
  double max_z = 0.0;
  bounds.Get(min_x, min_y, min_z, max_x, max_y, max_z);

  impl->summary = NativeTopologySummary{
      static_cast<std::uint32_t>(vertices.Extent()),
      static_cast<std::uint32_t>(edges.Extent()),
      count_subshapes(impl->shape, TopAbs_WIRE),
      static_cast<std::uint32_t>(faces.Extent()),
      count_subshapes(impl->shape, TopAbs_SHELL),
      solids,
      volume,
      min_x,
      min_y,
      min_z,
      max_x,
      max_y,
      max_z};

  for (Standard_Integer face_index = 1; face_index <= faces.Extent(); ++face_index) {
    const auto face_ordinal = static_cast<std::uint32_t>(face_index - 1);
    const TopoDS_Face face = TopoDS::Face(faces(face_index));
    impl->faces.push_back(inspect_face(face, face_ordinal));

    TopTools_IndexedMapOfShape boundary_edges;
    TopExp::MapShapes(face, TopAbs_EDGE, boundary_edges);
    for (Standard_Integer boundary_index = 1; boundary_index <= boundary_edges.Extent(); ++boundary_index) {
      const Standard_Integer edge_index = edges.FindIndex(boundary_edges(boundary_index));
      if (edge_index > 0) {
        impl->face_edges.push_back(NativeFaceEdgeEvidence{
            face_ordinal, static_cast<std::uint32_t>(edge_index - 1)});
      }
    }
  }

  TopTools_IndexedDataMapOfShapeListOfShape edge_ancestors;
  TopExp::MapShapesAndAncestors(impl->shape, TopAbs_EDGE, TopAbs_FACE, edge_ancestors);
  for (Standard_Integer edge_index = 1; edge_index <= edges.Extent(); ++edge_index) {
    const TopoDS_Shape& edge = edges(edge_index);
    impl->edges.push_back(inspect_edge(
        TopoDS::Edge(edge), static_cast<std::uint32_t>(edge_index - 1)));
    if (!edge_ancestors.Contains(edge)) {
      continue;
    }
    const TopTools_ListOfShape& adjacent_faces = edge_ancestors.FindFromKey(edge);
    for (NCollection_List<TopoDS_Shape>::Iterator iterator(adjacent_faces); iterator.More(); iterator.Next()) {
      const Standard_Integer face_index = faces.FindIndex(iterator.Value());
      if (face_index > 0) {
        impl->edge_faces.push_back(NativeEdgeFaceEvidence{
            static_cast<std::uint32_t>(edge_index - 1),
            static_cast<std::uint32_t>(face_index - 1)});
      }
    }
  }

  const bool valid_planar_face = allow_planar_face
      && solids == 0
      && faces.Extent() == 1
      && std::isfinite(volume)
      && std::abs(volume) <= tolerances().negligible;
  const bool valid_surface = allow_surface
      && solids == 0
      && faces.Extent() >= 1
      && std::isfinite(volume)
      && std::abs(volume) <= tolerances().negligible;
  const bool valid_solid = !allow_planar_face && !allow_surface
      && ((!allow_multi_solid && solids == 1) || (allow_multi_solid && solids >= 2))
      && std::isfinite(volume)
      && volume > 0.0;
  if (!analyzer.IsValid() || (!valid_planar_face && !valid_surface && !valid_solid)) {
    impl->status = STATUS_INVALID_SHAPE;
    impl->diagnostic = "OCCT result failed the exact-shape validity oracle";
  } else {
    impl->status = STATUS_OK;
    impl->diagnostic = valid_planar_face
        ? "valid exact planar face"
        : (valid_surface ? "valid exact surface" : "valid exact solid");
  }
  return std::make_unique<NativeOperationResult>(std::move(impl));
}

NativeOperationResult::NativeOperationResult(std::unique_ptr<Impl> impl) noexcept
    : impl_(std::move(impl)) {}
NativeOperationResult::~NativeOperationResult() = default;
NativeOperationResult::NativeOperationResult(NativeOperationResult&&) noexcept = default;
NativeOperationResult& NativeOperationResult::operator=(NativeOperationResult&&) noexcept = default;

std::uint8_t NativeOperationResult::status_code() const noexcept {
  return impl_ == nullptr ? STATUS_NULL_RESULT : impl_->status;
}

rust::String NativeOperationResult::diagnostic() const {
  return rust::String(impl_ == nullptr ? "Missing native result" : impl_->diagnostic);
}

bool NativeOperationResult::valid() const noexcept {
  return impl_ != nullptr && impl_->status == STATUS_OK;
}

NativeTopologySummary NativeOperationResult::topology_summary() const noexcept {
  return impl_ == nullptr ? NativeTopologySummary{} : impl_->summary;
}

rust::Vec<NativeFaceEvidence> NativeOperationResult::face_evidence() const {
  rust::Vec<NativeFaceEvidence> output;
  if (impl_ != nullptr) {
    output.reserve(impl_->faces.size());
    for (const NativeFaceEvidence& face : impl_->faces) {
      output.push_back(face);
    }
  }
  return output;
}

rust::Vec<NativeEdgeEvidence> NativeOperationResult::edge_evidence() const {
  rust::Vec<NativeEdgeEvidence> output;
  if (impl_ != nullptr) {
    output.reserve(impl_->edges.size());
    for (const NativeEdgeEvidence& edge : impl_->edges) {
      output.push_back(edge);
    }
  }
  return output;
}

rust::Vec<NativeFaceEdgeEvidence> NativeOperationResult::face_edge_evidence() const {
  rust::Vec<NativeFaceEdgeEvidence> output;
  if (impl_ != nullptr) {
    output.reserve(impl_->face_edges.size());
    for (const NativeFaceEdgeEvidence& entry : impl_->face_edges) {
      output.push_back(entry);
    }
  }
  return output;
}

rust::Vec<NativeEdgeFaceEvidence> NativeOperationResult::edge_face_evidence() const {
  rust::Vec<NativeEdgeFaceEvidence> output;
  if (impl_ != nullptr) {
    output.reserve(impl_->edge_faces.size());
    for (const NativeEdgeFaceEvidence& entry : impl_->edge_faces) {
      output.push_back(entry);
    }
  }
  return output;
}

rust::Vec<NativeHistoryEvidence> NativeOperationResult::history_evidence() const {
  rust::Vec<NativeHistoryEvidence> output;
  if (impl_ != nullptr) {
    output.reserve(impl_->history.size());
    for (const HistoryRecord& record : impl_->history) {
      output.push_back(NativeHistoryEvidence{
          rust::String(record.semantic_role),
          rust::String(record.relation),
          rust::String(record.source_element_id),
          record.output_ordinal,
          record.output_present});
    }
  }
  return output;
}

rust::Vec<NativeEdgeHistoryEvidence> NativeOperationResult::edge_history_evidence() const {
  rust::Vec<NativeEdgeHistoryEvidence> output;
  if (impl_ != nullptr) {
    output.reserve(impl_->edge_history.size());
    for (const EdgeHistoryRecord& record : impl_->edge_history) {
      output.push_back(NativeEdgeHistoryEvidence{
          rust::String(record.semantic_role),
          rust::String(record.relation),
          rust::String(record.source_element_id),
          record.output_ordinal,
          record.output_present});
    }
  }
  return output;
}

const NativeOperationResult::Impl& NativeOperationResult::impl() const noexcept {
  return *impl_;
}
} // namespace ketchup::exact
