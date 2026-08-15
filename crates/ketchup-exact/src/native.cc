#include "ketchup_exact.hxx"
#include "ketchup-exact/src/lib.rs.h"

#include <BRepAlgoAPI_Common.hxx>
#include <BRepAlgoAPI_Cut.hxx>
#include <BRepAlgoAPI_Fuse.hxx>
#include <BOPAlgo_Splitter.hxx>
#include <BRepAdaptor_Curve.hxx>
#include <BRepAdaptor_Surface.hxx>
#include <BRepBndLib.hxx>
#include <BRepBuilderAPI_FindPlane.hxx>
#include <BRepBuilderAPI_MakeEdge.hxx>
#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepBuilderAPI_MakeWire.hxx>
#include <BRepBuilderAPI_GTransform.hxx>
#include <BRepCheck_Analyzer.hxx>
#include <BRep_Builder.hxx>
#include <BRepGProp.hxx>
#include <BRepFilletAPI_MakeChamfer.hxx>
#include <BRepFilletAPI_MakeFillet.hxx>
#include <BRepOffsetAPI_MakeOffset.hxx>
#include <BRepOffsetAPI_ThruSections.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <BRepPrimAPI_MakePrism.hxx>
#include <BRepPrimAPI_MakeRevol.hxx>
#include <BRep_Tool.hxx>
#include <Bnd_Box.hxx>
#include <GProp_GProps.hxx>
#include <GC_MakeArcOfCircle.hxx>
#include <GeomAbs_CurveType.hxx>
#include <GeomAbs_JoinType.hxx>
#include <GeomAbs_SurfaceType.hxx>
#include <GeomAPI_Interpolate.hxx>
#include <Geom_Plane.hxx>
#include <Geom_TrimmedCurve.hxx>
#include <Standard_Failure.hxx>
#include <STEPControl_Reader.hxx>
#include <STEPControl_Writer.hxx>
#include <IFSelect_ReturnStatus.hxx>
#include <TopAbs_Orientation.hxx>
#include <TopAbs_ShapeEnum.hxx>
#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <NCollection_List.hxx>
#include <TopTools_IndexedDataMapOfShapeListOfShape.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TColgp_HArray1OfPnt.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Compound.hxx>
#include <TopoDS_Edge.hxx>
#include <TopoDS_Face.hxx>
#include <TopoDS_Shape.hxx>
#include <TopoDS_Vertex.hxx>
#include <TopoDS_Wire.hxx>
#include <gp_Ax1.hxx>
#include <gp_Ax2.hxx>
#include <gp_Circ.hxx>
#include <gp_Dir.hxx>
#include <gp_GTrsf.hxx>
#include <gp_Pln.hxx>
#include <gp_Pnt.hxx>
#include <gp_Vec.hxx>

#include <algorithm>
#include <cctype>
#include <cmath>
#include <cstdint>
#include <exception>
#include <functional>
#include <limits>
#include <memory>
#include <string>
#include <utility>
#include <vector>

namespace ketchup::exact {
namespace {

constexpr std::uint8_t STATUS_OK = 0;
constexpr std::uint8_t STATUS_INVALID_PARAMETER = 1;
constexpr std::uint8_t STATUS_NON_FINITE_PARAMETER = 2;
constexpr std::uint8_t STATUS_NO_GEOMETRIC_CHANGE = 3;
constexpr std::uint8_t STATUS_DEGENERATE_OPERATION = 4;
constexpr std::uint8_t STATUS_INVALID_SHAPE = 5;
constexpr std::uint8_t STATUS_BACKEND_EXCEPTION = 6;
constexpr std::uint8_t STATUS_NULL_RESULT = 7;

struct HistoryRecord {
  std::string semantic_role;
  std::string relation;
  std::string source_element_id;
  std::uint32_t output_ordinal = 0;
  bool output_present = false;
};

std::uint32_t count_subshapes(const TopoDS_Shape& shape, TopAbs_ShapeEnum kind) {
  std::uint32_t count = 0;
  for (TopExp_Explorer explorer(shape, kind); explorer.More(); explorer.Next()) {
    ++count;
  }
  return count;
}

TopoDS_Face face_at_ordinal(const TopoDS_Shape& shape, std::uint32_t target) {
  std::uint32_t ordinal = 0;
  for (TopExp_Explorer explorer(shape, TopAbs_FACE); explorer.More(); explorer.Next(), ++ordinal) {
    if (ordinal == target) {
      return TopoDS::Face(explorer.Current());
    }
  }
  return TopoDS_Face();
}

std::pair<std::uint32_t, bool> face_ordinal(
    const TopoDS_Shape& result, const TopoDS_Shape& candidate) {
  std::uint32_t ordinal = 0;
  for (TopExp_Explorer explorer(result, TopAbs_FACE); explorer.More(); explorer.Next(), ++ordinal) {
    if (explorer.Current().IsSame(candidate)) {
      return {ordinal, true};
    }
  }
  return {0, false};
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

std::string standard_failure_message(const Standard_Failure& failure) {
  const char* message = failure.what();
  return message == nullptr ? "OCCT Standard_Failure without a message" : message;
}

} // namespace

struct NativeOperationResult::Impl {
  std::uint8_t status = STATUS_NULL_RESULT;
  std::string diagnostic = "Native operation did not produce a result";
  TopoDS_Shape shape;
  NativeTopologySummary summary{};
  std::vector<NativeFaceEvidence> faces;
  std::vector<NativeFaceEdgeEvidence> face_edges;
  std::vector<NativeEdgeFaceEvidence> edge_faces;
  std::vector<HistoryRecord> history;
};

namespace {

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
  BRepBuilderAPI_FindPlane plane_finder(face);
  if (plane_finder.Found()) {
    surface_kind = "plane";
    gp_Dir normal = plane_finder.Plane()->Pln().Axis().Direction();
    if (face.Orientation() == TopAbs_REVERSED) {
      normal.Reverse();
    }
    normal_x = normal.X();
    normal_y = normal.Y();
    normal_z = normal.Z();
  } else if (BRepAdaptor_Surface(face).GetType() == GeomAbs_Cylinder) {
    surface_kind = "cylinder";
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

std::unique_ptr<NativeOperationResult> success_result(
    TopoDS_Shape shape,
    std::vector<HistoryRecord> history,
    bool allow_multi_solid = false,
    bool allow_planar_face = false) {
  auto impl = std::make_unique<NativeOperationResult::Impl>();
  impl->shape = std::move(shape);
  impl->history = std::move(history);

  if (impl->shape.IsNull()) {
    impl->status = STATUS_NULL_RESULT;
    impl->diagnostic = "OCCT returned a null shape";
    return std::make_unique<NativeOperationResult>(std::move(impl));
  }

  const BRepCheck_Analyzer analyzer(impl->shape, true);
  const std::uint32_t solids = count_subshapes(impl->shape, TopAbs_SOLID);
  GProp_GProps properties;
  BRepGProp::VolumeProperties(impl->shape, properties);
  const double volume = properties.Mass();

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
      && std::abs(volume) <= 1.0e-12;
  const bool valid_solid = !allow_planar_face
      && ((!allow_multi_solid && solids == 1) || (allow_multi_solid && solids >= 2))
      && std::isfinite(volume)
      && volume > 0.0;
  if (!analyzer.IsValid() || (!valid_planar_face && !valid_solid)) {
    impl->status = STATUS_INVALID_SHAPE;
    impl->diagnostic = "OCCT result failed the exact-shape validity oracle";
  } else {
    impl->status = STATUS_OK;
    impl->diagnostic = valid_planar_face ? "valid exact planar face" : "valid exact solid";
  }
  return std::make_unique<NativeOperationResult>(std::move(impl));
}

template <typename Operation>
std::unique_ptr<NativeOperationResult> guarded(Operation&& operation) noexcept {
  try {
    return std::invoke(std::forward<Operation>(operation));
  } catch (const Standard_Failure& failure) {
    return error_result(STATUS_BACKEND_EXCEPTION, standard_failure_message(failure));
  } catch (const std::exception& failure) {
    return error_result(STATUS_BACKEND_EXCEPTION, failure.what());
  } catch (...) {
    return error_result(STATUS_BACKEND_EXCEPTION, "Unknown native backend exception");
  }
}

void append_cut_history(
    std::vector<HistoryRecord>& history,
    BRepAlgoAPI_Cut& cut,
    const TopoDS_Shape& result,
    const TopoDS_Shape& source,
    const char* source_prefix) {
  std::uint32_t source_ordinal = 0;
  for (TopExp_Explorer explorer(source, TopAbs_FACE); explorer.More(); explorer.Next(), ++source_ordinal) {
    const TopoDS_Shape face = explorer.Current();
    const std::string source_id = std::string(source_prefix) + std::to_string(source_ordinal);
    if (cut.IsDeleted(face)) {
      history.push_back(HistoryRecord{"", "deleted", source_id, 0, false});
    }
    const NCollection_List<TopoDS_Shape>& modified = cut.Modified(face);
    for (NCollection_List<TopoDS_Shape>::Iterator iterator(modified); iterator.More(); iterator.Next()) {
      history.push_back(history_record("", "modified", source_id, result, iterator.Value()));
    }
    const NCollection_List<TopoDS_Shape>& generated = cut.Generated(face);
    for (NCollection_List<TopoDS_Shape>::Iterator iterator(generated); iterator.More(); iterator.Next()) {
      history.push_back(history_record("", "generated", source_id, result, iterator.Value()));
    }
  }
}

} // namespace

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

const NativeOperationResult::Impl& NativeOperationResult::impl() const noexcept {
  return *impl_;
}

std::unique_ptr<NativeOperationResult> make_box_native(
    double origin_x, double origin_y, double origin_z,
    double size_x, double size_y, double size_z) noexcept {
  return guarded([&] {
    BRepPrimAPI_MakeBox operation(gp_Pnt(origin_x, origin_y, origin_z), size_x, size_y, size_z);
    const TopoDS_Shape result = operation.Shape();
    if (!operation.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT box builder did not complete");
    }
    return success_result(result, {});
  });
}

std::unique_ptr<NativeOperationResult> extrude_rectangle_native(
    double width, double depth, double height) noexcept {
  return guarded([&] {
    const gp_Pnt south_west(0.0, 0.0, 0.0);
    const gp_Pnt south_east(width, 0.0, 0.0);
    const gp_Pnt north_east(width, depth, 0.0);
    const gp_Pnt north_west(0.0, depth, 0.0);

    const TopoDS_Edge south = BRepBuilderAPI_MakeEdge(south_west, south_east).Edge();
    const TopoDS_Edge east = BRepBuilderAPI_MakeEdge(south_east, north_east).Edge();
    const TopoDS_Edge north = BRepBuilderAPI_MakeEdge(north_east, north_west).Edge();
    const TopoDS_Edge west = BRepBuilderAPI_MakeEdge(north_west, south_west).Edge();
    BRepBuilderAPI_MakeWire wire_builder;
    wire_builder.Add(south);
    wire_builder.Add(east);
    wire_builder.Add(north);
    wire_builder.Add(west);
    if (!wire_builder.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT profile wire builder did not complete");
    }

    BRepBuilderAPI_MakeFace face_builder(wire_builder.Wire(), true);
    if (!face_builder.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT profile face builder did not complete");
    }
    const TopoDS_Face profile = face_builder.Face();
    TopoDS_Edge profile_east;
    for (TopExp_Explorer explorer(profile, TopAbs_EDGE); explorer.More(); explorer.Next()) {
      const TopoDS_Edge candidate = TopoDS::Edge(explorer.Current());
      TopoDS_Vertex first;
      TopoDS_Vertex last;
      TopExp::Vertices(candidate, first, last);
      if (!first.IsNull()
          && !last.IsNull()
          && std::abs(BRep_Tool::Pnt(first).X() - width) <= 1.0e-9
          && std::abs(BRep_Tool::Pnt(last).X() - width) <= 1.0e-9) {
        if (!profile_east.IsNull()) {
          return error_result(STATUS_INVALID_SHAPE, "OCCT profile has ambiguous east edge identity");
        }
        profile_east = candidate;
      }
    }
    if (profile_east.IsNull()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT profile lacks east edge identity");
    }
    BRepPrimAPI_MakePrism operation(profile, gp_Vec(0.0, 0.0, height), true, false);
    const TopoDS_Shape result = operation.Shape();
    if (!operation.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT prism builder did not complete");
    }

    std::vector<HistoryRecord> history;
    history.push_back(history_record(
        "extrusion.bottom",
        "first_shape",
        "profile.face",
        result,
        operation.FirstShape()));
    history.push_back(history_record(
        "extrusion.top",
        "last_shape",
        "profile.face",
        result,
        operation.LastShape()));
    HistoryRecord east_history{
        "extrusion.side(profile_edge=east)",
        "generated",
        "profile.edge.east",
        0,
        false};
    const NCollection_List<TopoDS_Shape>& east_generated = operation.Generated(profile_east);
    for (NCollection_List<TopoDS_Shape>::Iterator iterator(east_generated); iterator.More(); iterator.Next()) {
      const HistoryRecord backend_history = history_record(
          "extrusion.side(profile_edge=east)",
          "generated",
          "profile.edge.east",
          result,
          iterator.Value());
      if (backend_history.output_present) {
        east_history = backend_history;
        break;
      }
    }
    history.push_back(std::move(east_history));
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

std::unique_ptr<NativeOperationResult> sweep_rectangle_native(
    rust::Slice<const double> values) noexcept {
  return guarded([&] {
    if (values.size() != 8) {
      return error_result(STATUS_INVALID_PARAMETER, "OCCT sweep payload is malformed");
    }
    const double min_u = values[0];
    const double min_v = values[1];
    const double max_u = values[2];
    const double max_v = values[3];
    const double path_start_x = values[4];
    const double path_start_y = values[5];
    const double path_end_x = values[6];
    const double path_end_y = values[7];
    const double path_x = path_end_x - path_start_x;
    const double path_y = path_end_y - path_start_y;
    const double path_length = std::hypot(path_x, path_y);
    if (!std::isfinite(path_length) || path_length <= 1.0e-12) {
      return error_result(STATUS_INVALID_PARAMETER, "OCCT sweep path is degenerate");
    }
    const double section_x = path_y / path_length;
    const double section_y = -path_x / path_length;
    const auto point = [&](double u, double v) {
      return gp_Pnt(
          path_start_x + section_x * u,
          path_start_y + section_y * u,
          v);
    };
    const gp_Pnt corners[] = {
        point(min_u, min_v), point(max_u, min_v),
        point(max_u, max_v), point(min_u, max_v)};
    TopoDS_Edge profile_edges[] = {
        BRepBuilderAPI_MakeEdge(corners[0], corners[1]).Edge(),
        BRepBuilderAPI_MakeEdge(corners[1], corners[2]).Edge(),
        BRepBuilderAPI_MakeEdge(corners[2], corners[3]).Edge(),
        BRepBuilderAPI_MakeEdge(corners[3], corners[0]).Edge()};
    BRepBuilderAPI_MakeWire wire_builder;
    for (const TopoDS_Edge& edge : profile_edges) {
      wire_builder.Add(edge);
    }
    if (!wire_builder.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT sweep profile wire did not complete");
    }
    BRepBuilderAPI_MakeFace face_builder(wire_builder.Wire(), true);
    if (!face_builder.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT sweep profile face did not complete");
    }
    const TopoDS_Face profile = face_builder.Face();
    TopoDS_Edge operation_edges[4];
    for (std::size_t index = 0; index < 4; ++index) {
      const gp_Pnt& expected_start = corners[index];
      const gp_Pnt& expected_end = corners[(index + 1) % 4];
      for (TopExp_Explorer explorer(profile, TopAbs_EDGE); explorer.More(); explorer.Next()) {
        const TopoDS_Edge candidate = TopoDS::Edge(explorer.Current());
        TopoDS_Vertex first;
        TopoDS_Vertex last;
        TopExp::Vertices(candidate, first, last);
        if (first.IsNull() || last.IsNull()) {
          continue;
        }
        const gp_Pnt first_point = BRep_Tool::Pnt(first);
        const gp_Pnt last_point = BRep_Tool::Pnt(last);
        const bool forward = first_point.Distance(expected_start) <= 1.0e-9
            && last_point.Distance(expected_end) <= 1.0e-9;
        const bool reversed = first_point.Distance(expected_end) <= 1.0e-9
            && last_point.Distance(expected_start) <= 1.0e-9;
        if (forward || reversed) {
          if (!operation_edges[index].IsNull()) {
            return error_result(STATUS_INVALID_SHAPE, "OCCT sweep profile edge identity is ambiguous");
          }
          operation_edges[index] = candidate;
        }
      }
      if (operation_edges[index].IsNull()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT sweep profile edge identity was lost");
      }
    }
    BRepPrimAPI_MakePrism operation(profile, gp_Vec(path_x, path_y, 0.0), true, false);
    const TopoDS_Shape result = operation.Shape();
    if (!operation.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT sweep prism did not complete");
    }
    std::vector<HistoryRecord> history;
    history.push_back(history_record(
        "sweep.start", "first_shape", "profile.face", result, operation.FirstShape()));
    history.push_back(history_record(
        "sweep.end", "last_shape", "profile.face", result, operation.LastShape()));
    const char* roles[] = {
        "sweep.side.0", "sweep.side.1", "sweep.side.2", "sweep.side.3"};
    for (std::size_t index = 0; index < 4; ++index) {
      const std::string source = std::string("profile.edge.") + std::to_string(index);
      HistoryRecord record{roles[index], "generated", source, 0, false};
      const NCollection_List<TopoDS_Shape>& generated = operation.Generated(operation_edges[index]);
      for (NCollection_List<TopoDS_Shape>::Iterator iterator(generated); iterator.More(); iterator.Next()) {
        const HistoryRecord candidate = history_record(
            roles[index], "generated", source.c_str(), result, iterator.Value());
        if (candidate.output_present) {
          record = candidate;
          break;
        }
      }
      history.push_back(std::move(record));
    }
    return success_result(result, std::move(history));
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
      GeomAPI_Interpolate interpolation(points, true, 1.0e-9);
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
    BRepOffsetAPI_ThruSections operation(true, false, 1.0e-6);
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

std::unique_ptr<NativeOperationResult> extrude_circle_native(
    double center_x, double center_y, double radius, double height) noexcept {
  return guarded([&] {
    BRepPrimAPI_MakeCylinder operation(
        gp_Ax2(gp_Pnt(center_x, center_y, 0.0), gp_Dir(0.0, 0.0, 1.0)),
        radius,
        height);
    operation.Build();
    if (!operation.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT cylinder builder did not complete");
    }
    const TopoDS_Shape result = operation.Shape();
    TopoDS_Face bottom;
    TopoDS_Face top;
    TopoDS_Face side;
    for (TopExp_Explorer explorer(result, TopAbs_FACE); explorer.More(); explorer.Next()) {
      const TopoDS_Face candidate = TopoDS::Face(explorer.Current());
      BRepAdaptor_Surface surface(candidate);
      if (surface.GetType() == GeomAbs_Cylinder) {
        if (!side.IsNull()) {
          return error_result(STATUS_INVALID_SHAPE, "OCCT cylinder has ambiguous side identity");
        }
        side = candidate;
        continue;
      }
      BRepBuilderAPI_FindPlane plane(candidate);
      if (!plane.Found()) {
        continue;
      }
      GProp_GProps properties;
      BRepGProp::SurfaceProperties(candidate, properties);
      if (std::abs(properties.CentreOfMass().Z()) <= 1.0e-9) {
        bottom = candidate;
      } else if (std::abs(properties.CentreOfMass().Z() - height) <= 1.0e-9) {
        top = candidate;
      }
    }
    if (bottom.IsNull() || top.IsNull() || side.IsNull()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT cylinder face identity is incomplete");
    }
    std::vector<HistoryRecord> history;
    history.push_back(history_record(
        "extrusion.bottom", "analytic_cap", "profile.face", result, bottom));
    history.push_back(history_record(
        "extrusion.top", "analytic_cap", "profile.face", result, top));
    history.push_back(history_record(
        "extrusion.side(profile_edge=circle)",
        "analytic_generated",
        "profile.edge.circle",
        result,
        side));
    return success_result(result, std::move(history));
  });
}

std::unique_ptr<NativeOperationResult> extrude_mixed_profile_native(
    rust::Slice<const double> segments, double height) noexcept {
  return guarded([&] {
    if (segments.size() < 16 || segments.size() % 8 != 0) {
      return error_result(STATUS_INVALID_PARAMETER, "Mixed profile segment payload is malformed");
    }
    BRepBuilderAPI_MakeWire wire_builder;
    std::vector<TopoDS_Edge> profile_edges;
    profile_edges.reserve(segments.size() / 8);
    std::size_t first_arc_index = profile_edges.capacity();
    for (std::size_t offset = 0; offset < segments.size(); offset += 8) {
      const double kind = segments[offset];
      const gp_Pnt start(segments[offset + 1], segments[offset + 2], 0.0);
      const gp_Pnt end(segments[offset + 3], segments[offset + 4], 0.0);
      TopoDS_Edge edge;
      if (kind == 0.0) {
        edge = BRepBuilderAPI_MakeEdge(start, end).Edge();
      } else if (kind == 1.0) {
        const double center_x = segments[offset + 5];
        const double center_y = segments[offset + 6];
        const bool clockwise = segments[offset + 7] != 0.0;
        const double start_angle = std::atan2(start.Y() - center_y, start.X() - center_x);
        const double end_angle = std::atan2(end.Y() - center_y, end.X() - center_x);
        double sweep = end_angle - start_angle;
        const double tau = 2.0 * std::acos(-1.0);
        if (clockwise) {
          while (sweep >= 0.0) sweep -= tau;
        } else {
          while (sweep <= 0.0) sweep += tau;
        }
        const double radius = start.Distance(gp_Pnt(center_x, center_y, 0.0));
        const double middle_angle = start_angle + sweep / 2.0;
        const gp_Pnt middle(
            center_x + radius * std::cos(middle_angle),
            center_y + radius * std::sin(middle_angle),
            0.0);
        GC_MakeArcOfCircle arc_builder(start, middle, end);
        if (!arc_builder.IsDone()) {
          return error_result(STATUS_INVALID_SHAPE, "OCCT mixed profile arc builder did not complete");
        }
        edge = BRepBuilderAPI_MakeEdge(arc_builder.Value()).Edge();
        if (first_arc_index == profile_edges.capacity()) {
          first_arc_index = profile_edges.size();
        }
      } else {
        return error_result(STATUS_INVALID_PARAMETER, "Mixed profile segment kind is invalid");
      }
      if (edge.IsNull()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT mixed profile edge is null");
      }
      profile_edges.push_back(edge);
      wire_builder.Add(edge);
    }
    if (!wire_builder.IsDone() || first_arc_index >= profile_edges.size()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT mixed profile wire is incomplete");
    }
    BRepBuilderAPI_MakeFace face_builder(wire_builder.Wire(), true);
    if (!face_builder.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT mixed profile face builder did not complete");
    }
    const TopoDS_Face profile = face_builder.Face();
    TopoDS_Edge profile_arc;
    const std::size_t arc_offset = first_arc_index * 8;
    const gp_Pnt expected_arc_start(segments[arc_offset + 1], segments[arc_offset + 2], 0.0);
    const gp_Pnt expected_arc_end(segments[arc_offset + 3], segments[arc_offset + 4], 0.0);
    for (TopExp_Explorer explorer(profile, TopAbs_EDGE); explorer.More(); explorer.Next()) {
      const TopoDS_Edge candidate = TopoDS::Edge(explorer.Current());
      if (BRepAdaptor_Curve(candidate).GetType() != GeomAbs_Circle) {
        continue;
      }
      TopoDS_Vertex first;
      TopoDS_Vertex last;
      TopExp::Vertices(candidate, first, last);
      if (first.IsNull() || last.IsNull()) {
        continue;
      }
      const gp_Pnt first_point = BRep_Tool::Pnt(first);
      const gp_Pnt last_point = BRep_Tool::Pnt(last);
      const bool endpoints_match =
          (first_point.Distance(expected_arc_start) <= 1.0e-9
              && last_point.Distance(expected_arc_end) <= 1.0e-9)
          || (first_point.Distance(expected_arc_end) <= 1.0e-9
              && last_point.Distance(expected_arc_start) <= 1.0e-9);
      if (endpoints_match) {
        if (!profile_arc.IsNull()) {
          return error_result(STATUS_INVALID_SHAPE, "OCCT mixed profile first arc is ambiguous");
        }
        profile_arc = candidate;
      }
    }
    if (profile_arc.IsNull()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT mixed profile lost its first analytic arc");
    }
    BRepPrimAPI_MakePrism operation(profile, gp_Vec(0.0, 0.0, height), true, false);
    if (!operation.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT mixed profile prism builder did not complete");
    }
    const TopoDS_Shape result = operation.Shape();
    std::vector<HistoryRecord> history;
    history.push_back(history_record(
        "extrusion.bottom", "first_shape", "profile.face", result, operation.FirstShape()));
    history.push_back(history_record(
        "extrusion.top", "last_shape", "profile.face", result, operation.LastShape()));
    HistoryRecord arc_history{
        "extrusion.side(profile_edge=arc.0)", "generated", "profile.edge.arc.0", 0, false};
    const NCollection_List<TopoDS_Shape>& generated = operation.Generated(profile_arc);
    for (NCollection_List<TopoDS_Shape>::Iterator iterator(generated); iterator.More(); iterator.Next()) {
      const HistoryRecord candidate = history_record(
          "extrusion.side(profile_edge=arc.0)",
          "generated",
          "profile.edge.arc.0",
          result,
          iterator.Value());
      if (candidate.output_present) {
        arc_history = candidate;
        break;
      }
    }
    history.push_back(std::move(arc_history));
    return success_result(result, std::move(history));
  });
}

std::unique_ptr<NativeOperationResult> revolve_general_profile_native(
    rust::Slice<const double> segments,
    double axis_start_x, double axis_start_y,
    double axis_end_x, double axis_end_y,
    double angle_degrees) noexcept {
  return guarded([&] {
    if (segments.size() < 16 || segments.size() % 8 != 0
        || !std::isfinite(axis_start_x) || !std::isfinite(axis_start_y)
        || !std::isfinite(axis_end_x) || !std::isfinite(axis_end_y)
        || !std::isfinite(angle_degrees) || angle_degrees <= 0.0 || angle_degrees > 360.0) {
      return error_result(STATUS_INVALID_PARAMETER, "General revolve payload is malformed");
    }
    const gp_Vec axis_vector(
        axis_end_x - axis_start_x,
        axis_end_y - axis_start_y,
        0.0);
    if (axis_vector.Magnitude() <= 1.0e-12) {
      return error_result(STATUS_INVALID_PARAMETER, "General revolve axis is degenerate");
    }

    BRepBuilderAPI_MakeWire wire_builder;
    std::vector<TopoDS_Edge> profile_edges;
    profile_edges.reserve(segments.size() / 8);
    for (std::size_t offset = 0; offset < segments.size(); offset += 8) {
      const double kind = segments[offset];
      const gp_Pnt start(segments[offset + 1], segments[offset + 2], 0.0);
      const gp_Pnt end(segments[offset + 3], segments[offset + 4], 0.0);
      TopoDS_Edge edge;
      if (kind == 0.0) {
        edge = BRepBuilderAPI_MakeEdge(start, end).Edge();
      } else if (kind == 1.0) {
        const double center_x = segments[offset + 5];
        const double center_y = segments[offset + 6];
        const bool clockwise = segments[offset + 7] != 0.0;
        const double start_angle = std::atan2(start.Y() - center_y, start.X() - center_x);
        const double end_angle = std::atan2(end.Y() - center_y, end.X() - center_x);
        double sweep = end_angle - start_angle;
        const double tau = 2.0 * std::acos(-1.0);
        if (clockwise) {
          while (sweep >= 0.0) sweep -= tau;
        } else {
          while (sweep <= 0.0) sweep += tau;
        }
        const double radius = start.Distance(gp_Pnt(center_x, center_y, 0.0));
        const double middle_angle = start_angle + sweep / 2.0;
        const gp_Pnt middle(
            center_x + radius * std::cos(middle_angle),
            center_y + radius * std::sin(middle_angle),
            0.0);
        GC_MakeArcOfCircle arc_builder(start, middle, end);
        if (!arc_builder.IsDone()) {
          return error_result(STATUS_INVALID_SHAPE, "OCCT general revolve arc builder did not complete");
        }
        edge = BRepBuilderAPI_MakeEdge(arc_builder.Value()).Edge();
      } else {
        return error_result(STATUS_INVALID_PARAMETER, "General revolve segment kind is invalid");
      }
      if (edge.IsNull()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT general revolve edge is null");
      }
      profile_edges.push_back(edge);
      wire_builder.Add(edge);
    }
    if (!wire_builder.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT general revolve wire builder did not complete");
    }
    BRepBuilderAPI_MakeFace face_builder(wire_builder.Wire(), true);
    if (!face_builder.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT general revolve profile face builder did not complete");
    }
    const TopoDS_Face profile = face_builder.Face();
    std::vector<TopoDS_Edge> operation_edges;
    operation_edges.reserve(2);
    for (std::size_t source_index = 0; source_index < 2; ++source_index) {
      const std::size_t offset = source_index * 8;
      const double kind = segments[offset];
      const gp_Pnt expected_start(segments[offset + 1], segments[offset + 2], 0.0);
      const gp_Pnt expected_end(segments[offset + 3], segments[offset + 4], 0.0);
      gp_Pnt expected_middle(
          (expected_start.X() + expected_end.X()) / 2.0,
          (expected_start.Y() + expected_end.Y()) / 2.0,
          0.0);
      if (kind == 1.0) {
        const double center_x = segments[offset + 5];
        const double center_y = segments[offset + 6];
        const bool clockwise = segments[offset + 7] != 0.0;
        const double start_angle = std::atan2(
            expected_start.Y() - center_y, expected_start.X() - center_x);
        const double end_angle = std::atan2(
            expected_end.Y() - center_y, expected_end.X() - center_x);
        double sweep = end_angle - start_angle;
        const double tau = 2.0 * std::acos(-1.0);
        if (clockwise) {
          while (sweep >= 0.0) sweep -= tau;
        } else {
          while (sweep <= 0.0) sweep += tau;
        }
        const double radius = expected_start.Distance(gp_Pnt(center_x, center_y, 0.0));
        expected_middle = gp_Pnt(
            center_x + radius * std::cos(start_angle + sweep / 2.0),
            center_y + radius * std::sin(start_angle + sweep / 2.0),
            0.0);
      }
      TopoDS_Edge matched;
      for (TopExp_Explorer explorer(profile, TopAbs_EDGE); explorer.More(); explorer.Next()) {
        const TopoDS_Edge candidate = TopoDS::Edge(explorer.Current());
        BRepAdaptor_Curve curve(candidate);
        if ((kind == 0.0 && curve.GetType() != GeomAbs_Line)
            || (kind == 1.0 && curve.GetType() != GeomAbs_Circle)) {
          continue;
        }
        TopoDS_Vertex first;
        TopoDS_Vertex last;
        TopExp::Vertices(candidate, first, last);
        if (first.IsNull() || last.IsNull()) {
          continue;
        }
        const gp_Pnt first_point = BRep_Tool::Pnt(first);
        const gp_Pnt last_point = BRep_Tool::Pnt(last);
        const bool endpoints_match =
            (first_point.Distance(expected_start) <= 1.0e-9
                && last_point.Distance(expected_end) <= 1.0e-9)
            || (first_point.Distance(expected_end) <= 1.0e-9
                && last_point.Distance(expected_start) <= 1.0e-9);
        if (!endpoints_match) {
          continue;
        }
        if (kind == 1.0) {
          const gp_Pnt candidate_middle = curve.Value(
              (curve.FirstParameter() + curve.LastParameter()) / 2.0);
          if (candidate_middle.Distance(expected_middle) > 1.0e-8) {
            continue;
          }
        }
        matched = candidate;
        break;
      }
      if (matched.IsNull()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT general revolve profile edge identity was lost");
      }
      operation_edges.push_back(matched);
    }
    const double angle_radians = angle_degrees * std::acos(-1.0) / 180.0;
    BRepPrimAPI_MakeRevol operation(
        profile,
        gp_Ax1(gp_Pnt(axis_start_x, axis_start_y, 0.0), gp_Dir(axis_vector)),
        angle_radians,
        true);
    if (!operation.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT general revolve builder did not complete");
    }
    const TopoDS_Shape result = operation.Shape();
    std::vector<HistoryRecord> history;
    const char* side_roles[] = {"revolve.side.0", "revolve.side.1"};
    for (std::size_t index = 0; index < 2; ++index) {
      const std::string source = std::string("profile.edge.") + std::to_string(index);
      HistoryRecord record{side_roles[index], "generated", source, 0, false};
      const NCollection_List<TopoDS_Shape>& generated = operation.Generated(operation_edges[index]);
      for (NCollection_List<TopoDS_Shape>::Iterator iterator(generated); iterator.More(); iterator.Next()) {
        const HistoryRecord candidate = history_record(
            side_roles[index], "generated", source, result, iterator.Value());
        if (candidate.output_present) {
          record = candidate;
          break;
        }
      }
      history.push_back(std::move(record));
    }
    if (angle_degrees < 360.0) {
      history.push_back(history_record(
          "revolve.start", "first_shape", "profile.face", result, operation.FirstShape()));
      history.push_back(history_record(
          "revolve.end", "last_shape", "profile.face", result, operation.LastShape()));
    }
    return success_result(result, std::move(history));
  });
}

std::unique_ptr<NativeOperationResult> revolve_profile_native(
    rust::Slice<const double> points) noexcept {
  return guarded([&] {
    if (points.size() != 12) {
      return error_result(STATUS_INVALID_PARAMETER, "Bottle revolve requires six profile points");
    }
    std::vector<gp_Pnt> profile_points;
    profile_points.reserve(6);
    for (std::size_t index = 0; index < points.size(); index += 2) {
      const double radius = points[index];
      const double z = points[index + 1];
      if (!std::isfinite(radius) || !std::isfinite(z)) {
        return error_result(STATUS_NON_FINITE_PARAMETER, "Bottle revolve profile is non-finite");
      }
      profile_points.emplace_back(radius, 0.0, z);
    }

    std::vector<TopoDS_Edge> profile_edges;
    profile_edges.reserve(profile_points.size());
    BRepBuilderAPI_MakeWire wire_builder;
    for (std::size_t index = 0; index < profile_points.size(); ++index) {
      const std::size_t next = (index + 1) % profile_points.size();
      const TopoDS_Edge edge =
          BRepBuilderAPI_MakeEdge(profile_points[index], profile_points[next]).Edge();
      profile_edges.push_back(edge);
      wire_builder.Add(edge);
    }
    if (!wire_builder.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT revolve wire builder did not complete");
    }
    BRepBuilderAPI_MakeFace face_builder(wire_builder.Wire(), true);
    if (!face_builder.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT revolve profile face builder did not complete");
    }
    const TopoDS_Face profile = face_builder.Face();
    std::vector<TopoDS_Edge> operation_edges;
    operation_edges.reserve(5);
    for (std::size_t source_index = 0; source_index < 5; ++source_index) {
      TopoDS_Edge matched;
      const gp_Pnt expected_first = profile_points[source_index];
      const gp_Pnt expected_last = profile_points[source_index + 1];
      for (TopExp_Explorer explorer(profile, TopAbs_EDGE); explorer.More(); explorer.Next()) {
        const TopoDS_Edge candidate = TopoDS::Edge(explorer.Current());
        TopoDS_Vertex first;
        TopoDS_Vertex last;
        TopExp::Vertices(candidate, first, last);
        if (first.IsNull() || last.IsNull()) {
          continue;
        }
        const gp_Pnt first_point = BRep_Tool::Pnt(first);
        const gp_Pnt last_point = BRep_Tool::Pnt(last);
        const bool forward = first_point.Distance(expected_first) <= 1.0e-9
            && last_point.Distance(expected_last) <= 1.0e-9;
        const bool reverse = first_point.Distance(expected_last) <= 1.0e-9
            && last_point.Distance(expected_first) <= 1.0e-9;
        if (forward || reverse) {
          matched = candidate;
          break;
        }
      }
      if (matched.IsNull()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT revolve profile edge identity was lost");
      }
      operation_edges.push_back(matched);
    }
    BRepPrimAPI_MakeRevol operation(
        profile,
        gp_Ax1(gp_Pnt(0.0, 0.0, 0.0), gp_Dir(0.0, 0.0, 1.0)),
        2.0 * std::acos(-1.0),
        true);
    if (!operation.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT revolve builder did not complete");
    }
    const TopoDS_Shape result = operation.Shape();
    const char* roles[] = {
        "revolve.bottom", "revolve.body", "revolve.shoulder", "revolve.neck", "revolve.mouth"};
    std::vector<HistoryRecord> history;
    history.reserve(5);
    for (std::size_t index = 0; index < 5; ++index) {
      const std::string source = std::string("profile.edge.") + std::to_string(index);
      HistoryRecord record{roles[index], "generated", source, 0, false};
      const NCollection_List<TopoDS_Shape>& generated = operation.Generated(operation_edges[index]);
      for (NCollection_List<TopoDS_Shape>::Iterator iterator(generated); iterator.More(); iterator.Next()) {
        const HistoryRecord candidate = history_record(
            roles[index], "generated", source, result, iterator.Value());
        if (candidate.output_present) {
          record = candidate;
          break;
        }
      }
      history.push_back(std::move(record));
    }
    return success_result(result, std::move(history));
  });
}

std::unique_ptr<NativeOperationResult> shell_box_native(
    double width, double depth, double height, double thickness) noexcept {
  return guarded([&] {
    if (!std::isfinite(width) || !std::isfinite(depth) || !std::isfinite(height)
        || !std::isfinite(thickness) || width <= 0.0 || depth <= 0.0
        || height <= 0.0 || thickness <= 0.0
        || thickness * 2.0 >= std::min(width, depth) || thickness >= height) {
      return error_result(STATUS_INVALID_PARAMETER, "Box shell dimensions are outside the conservative envelope");
    }
    BRepPrimAPI_MakeBox outer_builder(width, depth, height);
    BRepPrimAPI_MakeBox cavity_builder(
        gp_Pnt(thickness, thickness, thickness),
        width - 2.0 * thickness,
        depth - 2.0 * thickness,
        height);
    const TopoDS_Shape outer = outer_builder.Shape();
    const TopoDS_Shape cavity = cavity_builder.Shape();
    if (!outer_builder.IsDone() || !cavity_builder.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT box shell operands did not complete");
    }
    BRepAlgoAPI_Cut operation(outer, cavity);
    operation.Build();
    if (!operation.IsDone() || operation.HasErrors()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT box shell cut did not complete");
    }
    const TopoDS_Shape result = operation.Shape();
    constexpr double tolerance = 1.0e-5;
    const auto close = [tolerance](double left, double right) {
      return std::abs(left - right) <= tolerance;
    };
    std::vector<HistoryRecord> history;
    for (TopExp_Explorer explorer(result, TopAbs_FACE); explorer.More(); explorer.Next()) {
      const TopoDS_Face face = TopoDS::Face(explorer.Current());
      Bnd_Box bounds;
      BRepBndLib::Add(face, bounds);
      double min_x = 0.0;
      double min_y = 0.0;
      double min_z = 0.0;
      double max_x = 0.0;
      double max_y = 0.0;
      double max_z = 0.0;
      bounds.Get(min_x, min_y, min_z, max_x, max_y, max_z);
      const char* role = nullptr;
      const char* source = nullptr;
      if (close(min_z, 0.0) && close(max_z, 0.0)
          && close(min_x, 0.0) && close(max_x, width)
          && close(min_y, 0.0) && close(max_y, depth)) {
        role = "shell.box.outer.bottom";
        source = "extrusion.bottom";
      } else if (close(min_x, width) && close(max_x, width)
          && close(min_y, 0.0) && close(max_y, depth)
          && close(min_z, 0.0) && close(max_z, height)) {
        role = "shell.box.outer.east";
        source = "extrusion.side(profile_edge=east)";
      } else if (close(min_z, height) && close(max_z, height)
          && close(min_x, 0.0) && close(max_x, width)
          && close(min_y, 0.0) && close(max_y, depth)) {
        role = "shell.box.rim";
        source = "extrusion.top";
      }
      if (role != nullptr) {
        history.push_back(history_record(
            role, "bounded_box_shell_classification", source, result, face));
      }
    }
    if (history.size() != 3) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT box shell did not produce three unambiguous semantic faces");
    }
    return success_result(result, std::move(history));
  });
}

std::unique_ptr<NativeOperationResult> finish_shell_box_native(
    double width, double depth, double height, double thickness,
    double amount, bool fillet) noexcept {
  return guarded([&] {
    if (!std::isfinite(amount) || amount <= 0.0) {
      return error_result(STATUS_INVALID_PARAMETER, "Box edge finish amount must be positive and finite");
    }
    auto base = shell_box_native(width, depth, height, thickness);
    if (!base || !base->valid()) {
      return error_result(STATUS_INVALID_SHAPE, "Box edge finish could not construct its shell input");
    }
    const TopoDS_Shape base_shape = base->impl().shape;
    constexpr double tolerance = 1.0e-5;
    TopTools_IndexedMapOfShape edges;
    TopExp::MapShapes(base_shape, TopAbs_EDGE, edges);
    TopoDS_Edge selected;
    for (Standard_Integer index = 1; index <= edges.Extent(); ++index) {
      const TopoDS_Edge edge = TopoDS::Edge(edges(index));
      Bnd_Box bounds;
      BRepBndLib::Add(edge, bounds);
      double min_x = 0.0;
      double min_y = 0.0;
      double min_z = 0.0;
      double max_x = 0.0;
      double max_y = 0.0;
      double max_z = 0.0;
      bounds.Get(min_x, min_y, min_z, max_x, max_y, max_z);
      if (std::abs(min_x - width) <= tolerance
          && std::abs(max_x - width) <= tolerance
          && std::abs(min_z - height) <= tolerance
          && std::abs(max_z - height) <= tolerance
          && std::abs(min_y) <= tolerance
          && std::abs(max_y - depth) <= tolerance) {
        if (!selected.IsNull()) {
          return error_result(STATUS_INVALID_SHAPE, "Box edge finish stable role is ambiguous");
        }
        selected = edge;
      }
    }
    if (selected.IsNull()) {
      return error_result(STATUS_INVALID_SHAPE, "Box edge finish stable role was not resolved");
    }
    const auto collect_finished = [&](auto& operation) -> std::unique_ptr<NativeOperationResult> {
      operation.Build();
      if (!operation.IsDone()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT box edge finish did not complete");
      }
      const TopoDS_Shape result = operation.Shape();
      if (result.IsNull()) {
        return error_result(STATUS_NULL_RESULT, "OCCT box edge finish returned a null shape");
      }
      std::vector<HistoryRecord> history;
      history.reserve(base->impl().history.size());
      for (const HistoryRecord& source_history : base->impl().history) {
        const TopoDS_Face source_face = face_at_ordinal(base_shape, source_history.output_ordinal);
        if (source_face.IsNull()) {
          return error_result(STATUS_INVALID_SHAPE, "Box edge finish lost a shell source face");
        }
        const TopTools_ListOfShape& modified = operation.Modified(source_face);
        if (modified.Extent() > 1) {
          return error_result(STATUS_INVALID_SHAPE, "Box edge finish produced ambiguous face history");
        }
        const TopoDS_Shape mapped_face = modified.IsEmpty() ? source_face : modified.First();
        HistoryRecord mapped = history_record(
            source_history.semantic_role,
            fillet ? "bounded_box_fillet_modified" : "bounded_box_chamfer_modified",
            source_history.source_element_id,
            result,
            mapped_face);
        if (!mapped.output_present) {
          return error_result(STATUS_INVALID_SHAPE, "Box edge finish history output is absent");
        }
        history.push_back(std::move(mapped));
      }
      return success_result(result, std::move(history));
    };
    if (fillet) {
      BRepFilletAPI_MakeFillet operation(base_shape);
      operation.Add(amount, selected);
      return collect_finished(operation);
    }
    BRepFilletAPI_MakeChamfer operation(base_shape);
    operation.Add(amount, selected);
    return collect_finished(operation);
  });
}

std::unique_ptr<NativeOperationResult> shell_revolve_profile_native(
    rust::Slice<const double> points, double thickness) noexcept {
  return guarded([&] {
    if (points.size() != 12 || !std::isfinite(thickness) || thickness <= 0.0) {
      return error_result(STATUS_INVALID_PARAMETER, "Bottle shell requires six points and positive finite thickness");
    }
    std::vector<gp_Pnt> outer_points;
    outer_points.reserve(6);
    for (std::size_t index = 0; index < points.size(); index += 2) {
      if (!std::isfinite(points[index]) || !std::isfinite(points[index + 1])) {
        return error_result(STATUS_NON_FINITE_PARAMETER, "Bottle shell profile is non-finite");
      }
      outer_points.emplace_back(points[index], 0.0, points[index + 1]);
    }
    const double body_radius = points[2];
    const double body_top_z = points[5];
    const double neck_radius = points[8];
    const double top_z = points[9];
    const double dr = points[6] - points[4];
    const double dz = points[7] - points[5];
    const double shoulder_length = std::hypot(dr, dz);
    if (body_radius != points[4]
        || neck_radius != points[6]
        || dr >= -1.0e-9
        || dz <= 1.0e-9
        || thickness >= body_radius * 0.5
        || thickness >= neck_radius * 0.5
        || thickness >= shoulder_length * 0.5) {
      return error_result(STATUS_INVALID_PARAMETER, "Bottle shell thickness is outside the conservative offset envelope");
    }
    const double shifted_radius = points[4] - dz / shoulder_length * thickness;
    const double shifted_z = body_top_z + dr / shoulder_length * thickness;
    const double inner_body_radius = body_radius - thickness;
    const double inner_neck_radius = neck_radius - thickness;
    const auto intersect_z = [&](double radius) {
      return shifted_z + (radius - shifted_radius) / dr * dz;
    };
    const double inner_body_top_z = intersect_z(inner_body_radius);
    const double inner_neck_bottom_z = intersect_z(inner_neck_radius);
    const double inner_bottom_z = points[1] + thickness;
    if (!(inner_bottom_z < inner_body_top_z
          && inner_body_top_z < inner_neck_bottom_z
          && inner_neck_bottom_z < top_z)) {
      return error_result(STATUS_DEGENERATE_OPERATION, "Bottle shell offset profile is degenerate");
    }
    const double cavity_extension = std::max(1.0, thickness);
    const std::vector<gp_Pnt> inner_points = {
        gp_Pnt(0.0, 0.0, inner_bottom_z),
        gp_Pnt(inner_body_radius, 0.0, inner_bottom_z),
        gp_Pnt(inner_body_radius, 0.0, inner_body_top_z),
        gp_Pnt(inner_neck_radius, 0.0, inner_neck_bottom_z),
        gp_Pnt(inner_neck_radius, 0.0, top_z + cavity_extension),
        gp_Pnt(0.0, 0.0, top_z + cavity_extension)};

    const auto revolve = [](const std::vector<gp_Pnt>& profile_points) -> TopoDS_Shape {
      BRepBuilderAPI_MakeWire wire_builder;
      for (std::size_t index = 0; index < profile_points.size(); ++index) {
        const std::size_t next = (index + 1) % profile_points.size();
        wire_builder.Add(BRepBuilderAPI_MakeEdge(profile_points[index], profile_points[next]).Edge());
      }
      if (!wire_builder.IsDone()) {
        throw Standard_Failure("OCCT shell profile wire builder did not complete");
      }
      BRepBuilderAPI_MakeFace face_builder(wire_builder.Wire(), true);
      if (!face_builder.IsDone()) {
        throw Standard_Failure("OCCT shell profile face builder did not complete");
      }
      BRepPrimAPI_MakeRevol operation(
          face_builder.Face(),
          gp_Ax1(gp_Pnt(0.0, 0.0, 0.0), gp_Dir(0.0, 0.0, 1.0)),
          2.0 * std::acos(-1.0),
          true);
      if (!operation.IsDone()) {
        throw Standard_Failure("OCCT shell revolve builder did not complete");
      }
      return operation.Shape();
    };

    const TopoDS_Shape outer = revolve(outer_points);
    const TopoDS_Shape cavity = revolve(inner_points);
    BRepAlgoAPI_Cut operation(outer, cavity);
    operation.Build();
    if (!operation.IsDone() || operation.HasErrors()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT bottle shell cut did not complete");
    }
    const TopoDS_Shape result = operation.Shape();
    if (result.IsNull()) {
      return error_result(STATUS_NULL_RESULT, "OCCT bottle shell returned a null shape");
    }

    constexpr double tolerance = 1.0e-5;
    const auto close = [tolerance](double left, double right) {
      return std::abs(left - right) <= tolerance;
    };
    std::vector<HistoryRecord> history;
    for (TopExp_Explorer explorer(result, TopAbs_FACE); explorer.More(); explorer.Next()) {
      const TopoDS_Face face = TopoDS::Face(explorer.Current());
      Bnd_Box bounds;
      BRepBndLib::Add(face, bounds);
      double min_x = 0.0;
      double min_y = 0.0;
      double min_z = 0.0;
      double max_x = 0.0;
      double max_y = 0.0;
      double max_z = 0.0;
      bounds.Get(min_x, min_y, min_z, max_x, max_y, max_z);
      const double radius = std::max({std::abs(min_x), std::abs(min_y), std::abs(max_x), std::abs(max_y)});
      const bool planar_z = close(min_z, max_z);
      const char* role = nullptr;
      const char* source = nullptr;
      if (planar_z && close(min_z, points[1]) && close(radius, body_radius)) {
        role = "shell.outer.bottom";
        source = "revolve.face.bottom";
      } else if (planar_z && close(min_z, top_z) && close(radius, neck_radius)) {
        role = "shell.rim";
        source = "revolve.face.mouth";
      } else if (planar_z && close(min_z, inner_bottom_z) && close(radius, inner_body_radius)) {
        role = "shell.inner.bottom";
        source = "shell.offset.bottom";
      } else if (!planar_z && close(radius, body_radius) && close(min_z, points[1]) && close(max_z, body_top_z)) {
        role = "shell.outer.body";
        source = "revolve.face.body";
      } else if (!planar_z && close(radius, body_radius) && close(min_z, body_top_z) && close(max_z, points[7])) {
        role = "shell.outer.shoulder";
        source = "revolve.face.shoulder";
      } else if (!planar_z && close(radius, neck_radius) && close(min_z, points[7]) && close(max_z, top_z)) {
        role = "shell.outer.neck";
        source = "revolve.face.neck";
      } else if (!planar_z && close(radius, inner_body_radius) && close(min_z, inner_bottom_z) && close(max_z, inner_body_top_z)) {
        role = "shell.inner.body";
        source = "shell.offset.body";
      } else if (!planar_z && close(radius, inner_body_radius) && close(min_z, inner_body_top_z) && close(max_z, inner_neck_bottom_z)) {
        role = "shell.inner.shoulder";
        source = "shell.offset.shoulder";
      } else if (!planar_z && close(radius, inner_neck_radius) && close(min_z, inner_neck_bottom_z) && close(max_z, top_z)) {
        role = "shell.inner.neck";
        source = "shell.offset.neck";
      }
      if (role != nullptr) {
        history.push_back(history_record(role, "bounded_shell_classification", source, result, face));
      }
    }
    if (history.size() != 9) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT bottle shell did not produce nine unambiguous semantic faces");
    }
    return success_result(result, std::move(history));
  });
}

std::unique_ptr<NativeOperationResult> finish_shell_revolve_profile_native(
    rust::Slice<const double> points, double thickness, double amount,
    bool fillet) noexcept {
  return guarded([&] {
    if (points.size() != 12 || !std::isfinite(amount) || amount <= 0.0) {
      return error_result(STATUS_INVALID_PARAMETER, "Bottle edge finish requires six points and positive finite amount");
    }
    auto base = shell_revolve_profile_native(points, thickness);
    if (!base || !base->valid()) {
      return error_result(STATUS_INVALID_SHAPE, "Bottle edge finish could not construct its shell input");
    }
    const TopoDS_Shape base_shape = base->impl().shape;
    constexpr double tolerance = 1.0e-5;
    std::vector<TopoDS_Edge> selected_edges;
    TopTools_IndexedMapOfShape unique_edges;
    TopExp::MapShapes(base_shape, TopAbs_EDGE, unique_edges);
    for (Standard_Integer edge_index = 1; edge_index <= unique_edges.Extent(); ++edge_index) {
      const TopoDS_Edge edge = TopoDS::Edge(unique_edges(edge_index));
      Bnd_Box bounds;
      BRepBndLib::Add(edge, bounds);
      double min_x = 0.0;
      double min_y = 0.0;
      double min_z = 0.0;
      double max_x = 0.0;
      double max_y = 0.0;
      double max_z = 0.0;
      bounds.Get(min_x, min_y, min_z, max_x, max_y, max_z);
      const double radius = std::max({std::abs(min_x), std::abs(min_y), std::abs(max_x), std::abs(max_y)});
      const bool lower = std::abs(min_z - points[5]) <= tolerance
          && std::abs(max_z - points[5]) <= tolerance
          && std::abs(radius - points[4]) <= tolerance;
      const bool upper = std::abs(min_z - points[7]) <= tolerance
          && std::abs(max_z - points[7]) <= tolerance
          && std::abs(radius - points[6]) <= tolerance;
      if (lower || upper) {
        selected_edges.push_back(edge);
      }
    }
    if (selected_edges.size() != 2) {
      return error_result(
          STATUS_INVALID_SHAPE,
          "Bottle edge finish resolved " + std::to_string(selected_edges.size()) + " shoulder edges instead of two");
    }

    const auto collect_finished = [&](auto& operation) -> std::unique_ptr<NativeOperationResult> {
      operation.Build();
      if (!operation.IsDone()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT bottle edge finish did not complete");
      }
      const TopoDS_Shape result = operation.Shape();
      if (result.IsNull()) {
        return error_result(STATUS_NULL_RESULT, "OCCT bottle edge finish returned a null shape");
      }
      std::vector<HistoryRecord> history;
      history.reserve(base->impl().history.size());
      for (const HistoryRecord& source_history : base->impl().history) {
        const TopoDS_Face source_face = face_at_ordinal(base_shape, source_history.output_ordinal);
        if (source_face.IsNull()) {
          return error_result(STATUS_INVALID_SHAPE, "Bottle edge finish lost a shell source face");
        }
        const TopTools_ListOfShape& modified = operation.Modified(source_face);
        if (modified.Extent() > 1) {
          return error_result(STATUS_INVALID_SHAPE, "Bottle edge finish produced ambiguous shell face history");
        }
        const TopoDS_Shape mapped_face = modified.IsEmpty() ? source_face : modified.First();
        HistoryRecord mapped = history_record(
            source_history.semantic_role,
            fillet ? "bounded_fillet_modified" : "bounded_chamfer_modified",
            source_history.source_element_id,
            result,
            mapped_face);
        if (!mapped.output_present) {
          return error_result(STATUS_INVALID_SHAPE, "Bottle edge finish history output is absent");
        }
        history.push_back(std::move(mapped));
      }
      return success_result(result, std::move(history));
    };

    if (fillet) {
      BRepFilletAPI_MakeFillet operation(base_shape);
      for (const TopoDS_Edge& edge : selected_edges) {
        operation.Add(amount, edge);
      }
      return collect_finished(operation);
    }
    BRepFilletAPI_MakeChamfer operation(base_shape);
    for (const TopoDS_Edge& edge : selected_edges) {
      operation.Add(amount, edge);
    }
    return collect_finished(operation);
  });
}

std::unique_ptr<NativeOperationResult> cut_box_native(
    const NativeOperationResult& base,
    double origin_x, double origin_y, double origin_z,
    double size_x, double size_y, double size_z) noexcept {
  return guarded([&] {
    if (!base.valid()) {
      return error_result(STATUS_INVALID_PARAMETER, "Cut base is not a valid exact body");
    }
    BRepPrimAPI_MakeBox tool_builder(
        gp_Pnt(origin_x, origin_y, origin_z), size_x, size_y, size_z);
    const TopoDS_Shape tool = tool_builder.Shape();
    if (!tool_builder.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT cut tool builder did not complete");
    }
    BRepAlgoAPI_Cut operation(base.impl().shape, tool);
    operation.Build();
    if (!operation.IsDone() || operation.HasErrors()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT cut operation did not complete");
    }
    const TopoDS_Shape result = operation.Shape();
    if (result.IsNull()) {
      return error_result(STATUS_NULL_RESULT, "OCCT cut returned a null shape");
    }

    GProp_GProps before_properties;
    GProp_GProps after_properties;
    BRepGProp::VolumeProperties(base.impl().shape, before_properties);
    BRepGProp::VolumeProperties(result, after_properties);
    const double scale = std::max(1.0, std::abs(before_properties.Mass()));
    if (std::abs(before_properties.Mass() - after_properties.Mass()) <=
        scale * 16.0 * std::numeric_limits<double>::epsilon()) {
      return error_result(STATUS_NO_GEOMETRIC_CHANGE, "Cut produced no measurable geometric change");
    }

    std::vector<HistoryRecord> history;
    append_cut_history(history, operation, result, base.impl().shape, "base.face.");
    append_cut_history(history, operation, result, tool, "tool.face.");
    return success_result(result, std::move(history));
  });
}

std::unique_ptr<NativeOperationResult> cut_cylinder_native(
    const NativeOperationResult& base,
    double center_x, double center_y, double origin_z,
    double radius, double height) noexcept {
  return guarded([&] {
    if (!base.valid()) {
      return error_result(STATUS_INVALID_PARAMETER, "Cylinder cut base is not a valid exact body");
    }
    BRepPrimAPI_MakeCylinder tool_builder(
        gp_Ax2(gp_Pnt(center_x, center_y, origin_z), gp_Dir(0.0, 0.0, 1.0)),
        radius,
        height);
    tool_builder.Build();
    if (!tool_builder.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT cylinder cut tool builder did not complete");
    }
    const TopoDS_Shape tool = tool_builder.Shape();
    BRepAlgoAPI_Cut operation(base.impl().shape, tool);
    operation.Build();
    if (!operation.IsDone() || operation.HasErrors()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT cylinder cut operation did not complete");
    }
    const TopoDS_Shape result = operation.Shape();
    if (result.IsNull()) {
      return error_result(STATUS_NULL_RESULT, "OCCT cylinder cut returned a null shape");
    }
    GProp_GProps before_properties;
    GProp_GProps after_properties;
    BRepGProp::VolumeProperties(base.impl().shape, before_properties);
    BRepGProp::VolumeProperties(result, after_properties);
    const double scale = std::max(1.0, std::abs(before_properties.Mass()));
    if (std::abs(before_properties.Mass() - after_properties.Mass()) <=
        scale * 16.0 * std::numeric_limits<double>::epsilon()) {
      return error_result(STATUS_NO_GEOMETRIC_CHANGE, "Cylinder cut produced no measurable geometric change");
    }
    std::vector<HistoryRecord> history;
    append_cut_history(history, operation, result, base.impl().shape, "base.face.");
    append_cut_history(history, operation, result, tool, "tool.face.");
    return success_result(result, std::move(history));
  });
}

std::unique_ptr<NativeOperationResult> fuse_box_native(
    const NativeOperationResult& base,
    double origin_x, double origin_y, double origin_z,
    double size_x, double size_y, double size_z) noexcept {
  return guarded([&] {
    if (!base.valid()) {
      return error_result(STATUS_INVALID_PARAMETER, "Fuse base is not a valid exact body");
    }
    BRepPrimAPI_MakeBox tool_builder(
        gp_Pnt(origin_x, origin_y, origin_z), size_x, size_y, size_z);
    const TopoDS_Shape tool = tool_builder.Shape();
    if (!tool_builder.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT fuse tool builder did not complete");
    }
    BRepAlgoAPI_Fuse operation(base.impl().shape, tool);
    operation.Build();
    if (operation.IsDone() && !operation.HasErrors()) {
      operation.SimplifyResult(true, true);
    }
    if (!operation.IsDone() || operation.HasErrors()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT fuse operation did not complete");
    }
    const TopoDS_Shape result = operation.Shape();
    if (result.IsNull()) {
      return error_result(STATUS_NULL_RESULT, "OCCT fuse returned a null shape");
    }

    GProp_GProps before_properties;
    GProp_GProps after_properties;
    BRepGProp::VolumeProperties(base.impl().shape, before_properties);
    BRepGProp::VolumeProperties(result, after_properties);
    const double scale = std::max(1.0, std::abs(before_properties.Mass()));
    if (after_properties.Mass() - before_properties.Mass() <=
        scale * 16.0 * std::numeric_limits<double>::epsilon()) {
      return error_result(STATUS_NO_GEOMETRIC_CHANGE, "Fuse produced no measurable geometric change");
    }

    return success_result(result, {});
  });
}

std::unique_ptr<NativeOperationResult> common_box_native(
    const NativeOperationResult& base,
    double origin_x, double origin_y, double origin_z,
    double size_x, double size_y, double size_z) noexcept {
  return guarded([&] {
    if (!base.valid()) {
      return error_result(STATUS_INVALID_PARAMETER, "Common base is not a valid exact body");
    }
    BRepPrimAPI_MakeBox tool_builder(
        gp_Pnt(origin_x, origin_y, origin_z), size_x, size_y, size_z);
    const TopoDS_Shape tool = tool_builder.Shape();
    if (!tool_builder.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT common tool builder did not complete");
    }
    BRepAlgoAPI_Common operation(base.impl().shape, tool);
    operation.Build();
    if (!operation.IsDone() || operation.HasErrors()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT common operation did not complete");
    }
    const TopoDS_Shape result = operation.Shape();
    if (result.IsNull()) {
      return error_result(STATUS_NULL_RESULT, "OCCT common returned a null shape");
    }
    return success_result(result, {});
  });
}

std::unique_ptr<NativeOperationResult> split_box_native(
    const NativeOperationResult& base,
    double origin_x, double origin_y, double origin_z,
    double size_x, double size_y, double size_z) noexcept {
  return guarded([&] {
    if (!base.valid()) {
      return error_result(STATUS_INVALID_PARAMETER, "Split base is not a valid exact body");
    }
    BRepPrimAPI_MakeBox tool_builder(
        gp_Pnt(origin_x, origin_y, origin_z), size_x, size_y, size_z);
    const TopoDS_Shape tool = tool_builder.Shape();
    if (!tool_builder.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT split tool builder did not complete");
    }
    BOPAlgo_Splitter operation;
    operation.AddArgument(base.impl().shape);
    operation.AddTool(tool);
    operation.Perform();
    if (operation.HasErrors()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT split operation did not complete");
    }
    const TopoDS_Shape result = operation.Shape();
    if (result.IsNull()) {
      return error_result(STATUS_NULL_RESULT, "OCCT split returned a null shape");
    }
    if (count_subshapes(result, TopAbs_SOLID) < 2) {
      return error_result(STATUS_NO_GEOMETRIC_CHANGE, "Split did not produce multiple target fragments");
    }

    GProp_GProps before_properties;
    GProp_GProps after_properties;
    BRepGProp::VolumeProperties(base.impl().shape, before_properties);
    BRepGProp::VolumeProperties(result, after_properties);
    const double scale = std::max(1.0, std::abs(before_properties.Mass()));
    if (std::abs(after_properties.Mass() - before_properties.Mass()) > scale * 1.0e-10) {
      return error_result(STATUS_INVALID_SHAPE, "Split did not preserve target volume");
    }
    return success_result(result, {}, true);
  });
}

std::unique_ptr<NativeOperationResult> exception_probe_native() noexcept {
  return guarded([]() -> std::unique_ptr<NativeOperationResult> {
    throw Standard_Failure("intentional A0 exception-boundary probe");
    return error_result(STATUS_BACKEND_EXCEPTION, "unreachable");
  });
}

std::unique_ptr<NativeOperationResult> import_step_native(rust::Str path) noexcept {
  return guarded([&] {
    const std::string native_path(path.data(), path.size());
    STEPControl_Reader reader;
    if (reader.ReadFile(native_path.c_str()) != IFSelect_RetDone) {
      return error_result(STATUS_INVALID_PARAMETER, "STEP reader could not read the fixture");
    }
    if (reader.TransferRoots() == 0) {
      return error_result(STATUS_INVALID_SHAPE, "STEP fixture contains no transferable roots");
    }
    const TopoDS_Shape shape = reader.OneShape();
    return success_result(shape, {}, count_subshapes(shape, TopAbs_SOLID) >= 2);
  });
}

rust::String step_length_unit_native(rust::Str path) noexcept {
  try {
    const std::string native_path(path.data(), path.size());
    STEPControl_Reader reader;
    if (reader.ReadFile(native_path.c_str()) != IFSelect_RetDone) {
      return rust::String();
    }
    NCollection_Sequence<TCollection_AsciiString> length_units;
    NCollection_Sequence<TCollection_AsciiString> angle_units;
    NCollection_Sequence<TCollection_AsciiString> solid_angle_units;
    reader.FileUnits(length_units, angle_units, solid_angle_units);
    if (length_units.Length() != 1) {
      return rust::String();
    }
    std::string unit(length_units.Value(1).ToCString());
    std::transform(unit.begin(), unit.end(), unit.begin(), [](unsigned char character) {
      return static_cast<char>(std::tolower(character));
    });
    return rust::String(unit);
  } catch (...) {
    return rust::String();
  }
}

std::unique_ptr<NativeOperationResult> transform_body_native(
    const NativeOperationResult& body, rust::Slice<const double> matrix) noexcept {
  return guarded([&] {
    if (!body.valid() || body.impl().shape.IsNull() || matrix.size() != 16) {
      return error_result(STATUS_INVALID_PARAMETER, "Exact body or affine transform is unavailable");
    }
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
    return success_result(
        operation.Shape(), {}, count_subshapes(body.impl().shape, TopAbs_SOLID) >= 2);
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

} // namespace ketchup::exact
