#include "ketchup_exact.hxx"
#include "ketchup-exact/src/lib.rs.h"

#include <BRepAlgoAPI_Cut.hxx>
#include <BRepBndLib.hxx>
#include <BRepBuilderAPI_FindPlane.hxx>
#include <BRepBuilderAPI_MakeEdge.hxx>
#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepBuilderAPI_MakeWire.hxx>
#include <BRepCheck_Analyzer.hxx>
#include <BRepGProp.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <BRepPrimAPI_MakePrism.hxx>
#include <BRep_Tool.hxx>
#include <Bnd_Box.hxx>
#include <GProp_GProps.hxx>
#include <Geom_Plane.hxx>
#include <Standard_Failure.hxx>
#include <STEPControl_Reader.hxx>
#include <IFSelect_ReturnStatus.hxx>
#include <TopAbs_Orientation.hxx>
#include <TopAbs_ShapeEnum.hxx>
#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <NCollection_List.hxx>
#include <TopTools_IndexedDataMapOfShapeListOfShape.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Edge.hxx>
#include <TopoDS_Face.hxx>
#include <TopoDS_Shape.hxx>
#include <TopoDS_Vertex.hxx>
#include <gp_Dir.hxx>
#include <gp_Pln.hxx>
#include <gp_Pnt.hxx>
#include <gp_Vec.hxx>

#include <algorithm>
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
    std::vector<HistoryRecord> history) {
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
  BRepBndLib::Add(impl->shape, bounds);
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

  if (!analyzer.IsValid() || solids != 1 || !std::isfinite(volume) || volume <= 0.0) {
    impl->status = STATUS_INVALID_SHAPE;
    impl->diagnostic = "OCCT result failed the exact-body validity oracle";
  } else {
    impl->status = STATUS_OK;
    impl->diagnostic = "valid exact solid";
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
    return success_result(reader.OneShape(), {});
  });
}

} // namespace ketchup::exact
