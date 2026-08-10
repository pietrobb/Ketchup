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
#include <BRepFilletAPI_MakeChamfer.hxx>
#include <BRepFilletAPI_MakeFillet.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <BRepPrimAPI_MakePrism.hxx>
#include <BRepPrimAPI_MakeRevol.hxx>
#include <BRep_Tool.hxx>
#include <Bnd_Box.hxx>
#include <GProp_GProps.hxx>
#include <Geom_Plane.hxx>
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
#include <TopoDS.hxx>
#include <TopoDS_Edge.hxx>
#include <TopoDS_Face.hxx>
#include <TopoDS_Shape.hxx>
#include <TopoDS_Vertex.hxx>
#include <gp_Ax1.hxx>
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
