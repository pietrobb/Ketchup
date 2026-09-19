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
#include <GeomLib_IsPlanarSurface.hxx>
#include <Geom_RectangularTrimmedSurface.hxx>
#include <BRepBuilderAPI_MakeEdge.hxx>
#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepBuilderAPI_MakeSolid.hxx>
#include <BRepBuilderAPI_MakeWire.hxx>
#include <BRepBuilderAPI_Sewing.hxx>
#include <BRepBuilderAPI_GTransform.hxx>
#include <BRepBuilderAPI_Transform.hxx>
#include <BRepCheck_Analyzer.hxx>
#include <BRepClass3d_SolidClassifier.hxx>
#include <BRepExtrema_DistShapeShape.hxx>
#include <BRep_Builder.hxx>
#include <BRepGProp.hxx>
#include <BRepLib.hxx>
#include <BRepMesh_IncrementalMesh.hxx>
#include <BRepTools.hxx>
#include <Poly_Triangulation.hxx>
#include <TopLoc_Location.hxx>
#include <BRepFilletAPI_MakeChamfer.hxx>
#include <BRepFilletAPI_MakeFillet.hxx>
#include <BRepOffsetAPI_MakeOffset.hxx>
#include <BRepOffsetAPI_MakeOffsetShape.hxx>
#include <BRepOffsetAPI_MakePipeShell.hxx>
#include <BRepOffsetAPI_MakeThickSolid.hxx>
#include <BRepOffsetAPI_ThruSections.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <BRepPrimAPI_MakeHalfSpace.hxx>
#include <BRepPrimAPI_MakePrism.hxx>
#include <BRepPrimAPI_MakeRevol.hxx>
#include <BRep_Tool.hxx>
#include <Bnd_Box.hxx>
#include <GProp_GProps.hxx>
#include <GC_MakeArcOfCircle.hxx>
#include <GeomAbs_Shape.hxx>
#include <GeomAbs_CurveType.hxx>
#include <GeomAbs_JoinType.hxx>
#include <GeomAbs_SurfaceType.hxx>
#include <GeomAPI_Interpolate.hxx>
#include <Geom_BezierCurve.hxx>
#include <Geom_Plane.hxx>
#include <Geom_TrimmedCurve.hxx>
#include <Standard_Failure.hxx>
#include <STEPCAFControl_Reader.hxx>
#include <STEPCAFControl_Writer.hxx>
#include <STEPControl_Reader.hxx>
#include <STEPControl_Writer.hxx>
#include <IFSelect_ReturnStatus.hxx>
#include <Quantity_Color.hxx>
#include <TDataStd_Name.hxx>
#include <TDF_Tool.hxx>
#include <TDocStd_Document.hxx>
#include <XCAFDoc_ColorTool.hxx>
#include <XCAFDoc_DocumentTool.hxx>
#include <XCAFDoc_ShapeTool.hxx>
#include <IGESCAFControl_Reader.hxx>
#include <IGESCAFControl_Writer.hxx>
#include <IGESControl_Controller.hxx>
#include <IGESControl_Reader.hxx>
#include <IGESControl_Writer.hxx>
#include <Interface_Static.hxx>
#include <IGESData_GlobalSection.hxx>
#include <IGESData_IGESModel.hxx>
#include <TopAbs_Orientation.hxx>
#include <TopAbs_ShapeEnum.hxx>
#include <TopAbs_State.hxx>
#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <TopLoc_Location.hxx>
#include <NCollection_Array1.hxx>
#include <NCollection_List.hxx>
#include <TopTools_IndexedDataMapOfShapeListOfShape.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TColgp_Array1OfPnt.hxx>
#include <TColgp_HArray1OfPnt.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Compound.hxx>
#include <TopoDS_Edge.hxx>
#include <TopoDS_Face.hxx>
#include <TopoDS_Shape.hxx>
#include <TopoDS_Shell.hxx>
#include <TopoDS_Solid.hxx>
#include <TopoDS_Vertex.hxx>
#include <TopoDS_Wire.hxx>
#include <gp_Ax1.hxx>
#include <gp_Ax2.hxx>
#include <gp_Circ.hxx>
#include <gp_Cylinder.hxx>
#include <gp_Dir.hxx>
#include <gp_GTrsf.hxx>
#include <gp_Pln.hxx>
#include <gp_Trsf.hxx>
#include <gp_Pnt.hxx>
#include <gp_Pnt2d.hxx>
#include <gp_Vec.hxx>

#include <algorithm>
#include <array>
#include <cctype>
#include <cmath>
#include <cstdint>
#include <cstring>
#include <exception>
#include <functional>
#include <iomanip>
#include <limits>
#include <memory>
#include <map>
#include <mutex>
#include <set>
#include <sstream>
#include <string>
#include <type_traits>
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

struct EdgeHistoryRecord {
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

} // namespace

struct NativeOperationResult::Impl {
  std::uint8_t status = STATUS_NULL_RESULT;
  std::string diagnostic = "Native operation did not produce a result";
  TopoDS_Shape shape;
  NativeTopologySummary summary{};
  std::vector<NativeFaceEvidence> faces;
  std::vector<NativeEdgeEvidence> edges;
  std::vector<NativeFaceEdgeEvidence> face_edges;
  std::vector<NativeEdgeFaceEvidence> edge_faces;
  std::vector<HistoryRecord> history;
  std::vector<EdgeHistoryRecord> edge_history;
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
  const GeomLib_IsPlanarSurface planar(trimmed, 1.0e-7);
  if (!planar.IsPlanar()) {
    return false;
  }
  gp_Pnt point;
  gp_Vec du, dv;
  surface->D1((u0 + u1) * 0.5, (v0 + v1) * 0.5, point, du, dv);
  const gp_Vec cross = du.Crossed(dv);
  if (cross.SquareMagnitude() <= 1.0e-24) {
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
    bool allow_multi_solid = false,
    bool allow_planar_face = false,
    std::vector<EdgeHistoryRecord> edge_history = {},
    bool allow_surface = false) {
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
      && std::abs(volume) <= 1.0e-12;
  const bool valid_surface = allow_surface
      && solids == 0
      && faces.Extent() >= 1
      && std::isfinite(volume)
      && std::abs(volume) <= 1.0e-12;
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

template <typename Operation>
void append_propagated_history(
    std::vector<HistoryRecord>& history,
    Operation& operation,
    const TopoDS_Shape& result,
    const NativeOperationResult::Impl& source) {
  for (const HistoryRecord& source_history : source.history) {
    if (!source_history.output_present) {
      continue;
    }
    const TopoDS_Face source_face =
        face_at_ordinal(source.shape, source_history.output_ordinal);
    if (source_face.IsNull()) {
      continue;
    }
    if (operation.IsDeleted(source_face)) {
      history.push_back(HistoryRecord{
          source_history.semantic_role,
          "deleted",
          source_history.source_element_id,
          0,
          false});
    }
    const NCollection_List<TopoDS_Shape>& modified = operation.Modified(source_face);
    for (NCollection_List<TopoDS_Shape>::Iterator iterator(modified);
         iterator.More(); iterator.Next()) {
      history.push_back(history_record(
          source_history.semantic_role,
          "modified",
          source_history.source_element_id,
          result,
          iterator.Value()));
    }
    const NCollection_List<TopoDS_Shape>& generated = operation.Generated(source_face);
    for (NCollection_List<TopoDS_Shape>::Iterator iterator(generated);
         iterator.More(); iterator.Next()) {
      history.push_back(history_record(
          source_history.semantic_role,
          "generated",
          source_history.source_element_id,
          result,
          iterator.Value()));
    }
    if (!operation.IsDeleted(source_face) && modified.IsEmpty() && generated.IsEmpty()) {
      history.push_back(history_record(
          source_history.semantic_role,
          "unchanged",
          source_history.source_element_id,
          result,
          source_face));
    }
  }
}

template <typename Operation>
void append_propagated_edge_history(
    std::vector<EdgeHistoryRecord>& history,
    Operation& operation,
    const TopoDS_Shape& result,
    const NativeOperationResult::Impl& source) {
  for (const EdgeHistoryRecord& source_history : source.edge_history) {
    if (!source_history.output_present) {
      continue;
    }
    const TopoDS_Edge source_edge =
        edge_at_ordinal(source.shape, source_history.output_ordinal);
    if (source_edge.IsNull()) {
      continue;
    }
    if (operation.IsDeleted(source_edge)) {
      history.push_back(EdgeHistoryRecord{
          source_history.semantic_role,
          "deleted",
          source_history.source_element_id,
          0,
          false});
    }
    const NCollection_List<TopoDS_Shape>& modified = operation.Modified(source_edge);
    for (NCollection_List<TopoDS_Shape>::Iterator iterator(modified);
         iterator.More(); iterator.Next()) {
      history.push_back(edge_history_record(
          source_history.semantic_role,
          "modified",
          source_history.source_element_id,
          result,
          iterator.Value()));
    }
    const NCollection_List<TopoDS_Shape>& generated = operation.Generated(source_edge);
    for (NCollection_List<TopoDS_Shape>::Iterator iterator(generated);
         iterator.More(); iterator.Next()) {
      history.push_back(edge_history_record(
          source_history.semantic_role,
          "generated",
          source_history.source_element_id,
          result,
          iterator.Value()));
    }
    if (!operation.IsDeleted(source_edge) && modified.IsEmpty() && generated.IsEmpty()) {
      history.push_back(edge_history_record(
          source_history.semantic_role,
          "unchanged",
          source_history.source_element_id,
          result,
          source_edge));
    }
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

std::unique_ptr<NativeOperationResult> make_box_native(
    double origin_x, double origin_y, double origin_z,
    double size_x, double size_y, double size_z) noexcept {
  return guarded([&] {
    BRepPrimAPI_MakeBox operation(gp_Pnt(origin_x, origin_y, origin_z), size_x, size_y, size_z);
    const TopoDS_Shape result = operation.Shape();
    if (!operation.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT box builder did not complete");
    }
    std::vector<HistoryRecord> history;
    for (const auto& [role, face] : std::array<std::pair<const char*, TopoDS_Face>, 6>{
             std::pair{"box.face.bottom", operation.BottomFace()},
             std::pair{"box.face.top", operation.TopFace()},
             std::pair{"box.face.left", operation.LeftFace()},
             std::pair{"box.face.right", operation.RightFace()},
             std::pair{"box.face.front", operation.FrontFace()},
             std::pair{"box.face.back", operation.BackFace()},
         }) {
      history.push_back(history_record(role, "generated", role, result, face));
    }
    return success_result(result, std::move(history));
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
    TopoDS_Vertex profile_north_east;
    for (TopExp_Explorer explorer(profile, TopAbs_EDGE); explorer.More(); explorer.Next()) {
      const TopoDS_Edge candidate = TopoDS::Edge(explorer.Current());
      TopoDS_Vertex first;
      TopoDS_Vertex last;
      TopExp::Vertices(candidate, first, last);
      for (const TopoDS_Vertex& vertex : {first, last}) {
        if (!vertex.IsNull()
            && std::abs(BRep_Tool::Pnt(vertex).X() - width) <= 1.0e-9
            && std::abs(BRep_Tool::Pnt(vertex).Y() - depth) <= 1.0e-9) {
          profile_north_east = vertex;
        }
      }
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
    if (profile_east.IsNull() || profile_north_east.IsNull()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT profile lacks stable east-edge or north-east-vertex identity");
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
    std::vector<EdgeHistoryRecord> edge_history;
    const NCollection_List<TopoDS_Shape>& north_east_generated =
        operation.Generated(profile_north_east);
    for (NCollection_List<TopoDS_Shape>::Iterator iterator(north_east_generated);
         iterator.More(); iterator.Next()) {
      const EdgeHistoryRecord candidate = edge_history_record(
          "extrusion.edge(profile_vertex=north_east)",
          "generated",
          "profile.vertex.north_east",
          result,
          iterator.Value());
      if (candidate.output_present) {
        edge_history.push_back(candidate);
      }
    }
    return success_result(result, std::move(history), false, false, std::move(edge_history));
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

TopoDS_Edge cubic_bezier_edge(
    rust::Slice<const double> segments, std::size_t offset, double z);

std::unique_ptr<NativeOperationResult> offset_planar_profile_native(
    rust::Slice<const double> segments, double distance) noexcept {
  return guarded([&] {
    if (segments.size() < 20 || segments.size() % 10 != 0 || segments.size() > 640
        || !std::isfinite(distance)
        || std::abs(distance) > 100000.0) {
      return error_result(STATUS_INVALID_PARAMETER, "Planar offset payload is malformed");
    }
    BRepBuilderAPI_MakeWire source_builder;
    bool line_only = true;
    for (std::size_t offset = 0; offset < segments.size(); offset += 10) {
      for (std::size_t index = 0; index < 10; ++index) {
        if (!std::isfinite(segments[offset + index])
            || std::abs(segments[offset + index]) > 1000000.0) {
          return error_result(STATUS_INVALID_PARAMETER, "Planar offset segment value is invalid");
        }
      }
      const double kind = segments[offset];
      const gp_Pnt start(segments[offset + 1], segments[offset + 2], 0.0);
      const gp_Pnt end(segments[offset + 3], segments[offset + 4], 0.0);
      const std::size_t next = (offset + 10) % segments.size();
      if (segments[offset + 3] != segments[next + 1]
          || segments[offset + 4] != segments[next + 2]) {
        return error_result(STATUS_INVALID_PARAMETER, "Planar offset source wire is open");
      }
      TopoDS_Edge edge;
      if (kind == 0.0) {
        if (segments[offset + 5] != 0.0 || segments[offset + 6] != 0.0
            || segments[offset + 7] != 0.0 || segments[offset + 8] != 0.0
            || segments[offset + 9] != 0.0) {
          return error_result(STATUS_INVALID_PARAMETER, "Planar offset line payload is malformed");
        }
        const double length = start.Distance(end);
        if (!std::isfinite(length) || length < 0.01 || length > 100000.0) {
          return error_result(STATUS_INVALID_PARAMETER, "Planar offset line length is invalid");
        }
        BRepBuilderAPI_MakeEdge edge_builder(start, end);
        if (!edge_builder.IsDone()) {
          return error_result(STATUS_INVALID_SHAPE, "OCCT planar offset line builder did not complete");
        }
        edge = edge_builder.Edge();
      } else if (kind == 1.0) {
        line_only = false;
        if (segments[offset + 7] != 0.0 || segments[offset + 8] != 0.0
            || (segments[offset + 9] != 0.0 && segments[offset + 9] != 1.0)) {
          return error_result(STATUS_INVALID_PARAMETER, "Planar offset arc payload is malformed");
        }
        const double center_x = segments[offset + 5];
        const double center_y = segments[offset + 6];
        const bool clockwise = segments[offset + 9] != 0.0;
        const gp_Pnt center(center_x, center_y, 0.0);
        const double radius = start.Distance(center);
        const double end_radius = end.Distance(center);
        if (!std::isfinite(radius) || radius < 0.01 || radius > 100000.0
            || std::abs(radius - end_radius) > 1.0e-9 * std::max({radius, end_radius, 1.0})
            || std::abs(center_x) + radius > 1000000.0
            || std::abs(center_y) + radius > 1000000.0) {
          return error_result(STATUS_INVALID_PARAMETER, "Planar offset arc radius is invalid");
        }
        const double start_angle = std::atan2(start.Y() - center_y, start.X() - center_x);
        const double end_angle = std::atan2(end.Y() - center_y, end.X() - center_x);
        double sweep = end_angle - start_angle;
        const double tau = 2.0 * std::acos(-1.0);
        if (clockwise) {
          while (sweep >= 0.0) sweep -= tau;
        } else {
          while (sweep <= 0.0) sweep += tau;
        }
        const double middle_angle = start_angle + sweep / 2.0;
        const gp_Pnt middle(
            center_x + radius * std::cos(middle_angle),
            center_y + radius * std::sin(middle_angle),
            0.0);
        GC_MakeArcOfCircle arc_builder(start, middle, end);
        if (!arc_builder.IsDone()) {
          return error_result(STATUS_INVALID_SHAPE, "OCCT planar offset arc builder did not complete");
        }
        BRepBuilderAPI_MakeEdge edge_builder(arc_builder.Value());
        if (!edge_builder.IsDone()) {
          return error_result(STATUS_INVALID_SHAPE, "OCCT planar offset arc edge did not complete");
        }
        edge = edge_builder.Edge();
      } else if (kind == 2.0) {
        line_only = false;
        if (segments[offset + 9] != 0.0) {
          return error_result(STATUS_INVALID_PARAMETER, "Planar offset cubic payload is malformed");
        }
        const gp_Pnt control_1(segments[offset + 5], segments[offset + 6], 0.0);
        const gp_Pnt control_2(segments[offset + 7], segments[offset + 8], 0.0);
        const double control_polygon_length =
            start.Distance(control_1) + control_1.Distance(control_2) + control_2.Distance(end);
        if (!std::isfinite(control_polygon_length) || control_polygon_length < 0.01
            || control_polygon_length > 100000.0) {
          return error_result(STATUS_INVALID_PARAMETER, "Planar offset cubic length is invalid");
        }
        edge = cubic_bezier_edge(segments, offset, 0.0);
      } else {
        return error_result(STATUS_INVALID_PARAMETER, "Planar offset segment kind is unsupported");
      }
      if (edge.IsNull()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT planar offset source edge is null");
      }
      source_builder.Add(edge);
    }
    if (!source_builder.IsDone() || (line_only && segments.size() < 30)) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT planar offset source wire did not complete");
    }
    BRepBuilderAPI_MakeFace source_face_builder(source_builder.Wire(), true);
    if (!source_face_builder.IsDone()
        || !BRepCheck_Analyzer(source_face_builder.Face()).IsValid()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT planar offset source face is invalid");
    }
    if (distance == 0.0) {
      const TopoDS_Face result = source_face_builder.Face();
      std::vector<HistoryRecord> history;
      history.push_back(history_record(
          "planar_surface.face", "generated", "profile.face", result, result));
      return success_result(result, std::move(history), false, true);
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
    if (!face_builder.IsDone() || !BRepCheck_Analyzer(face_builder.Face()).IsValid()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT planar offset face is invalid");
    }
    const TopoDS_Face result = face_builder.Face();
    std::vector<HistoryRecord> history;
    history.push_back(history_record(
        "planar_offset.face", "offset_generated", "profile.face", result, result));
    return success_result(result, std::move(history), false, true);
  });
}

std::unique_ptr<NativeOperationResult> planar_surface_profile_native(
    rust::Slice<const double> segments) noexcept {
  return offset_planar_profile_native(segments, 0.0);
}

std::unique_ptr<NativeOperationResult> trim_surface_native(
    const NativeOperationResult& target,
    const NativeOperationResult& cutter) noexcept {
  return guarded([&] {
    if (!target.valid() || !cutter.valid()
        || target.impl().shape.IsNull() || cutter.impl().shape.IsNull()
        || target.impl().summary.solid_count != 0 || cutter.impl().summary.solid_count != 0
        || target.impl().summary.face_count == 0 || cutter.impl().summary.face_count == 0) {
      return error_result(STATUS_INVALID_PARAMETER, "Surface trim requires valid non-solid target and cutter surfaces");
    }
    BRepAlgoAPI_Common operation(target.impl().shape, cutter.impl().shape);
    operation.SetNonDestructive(true);
    operation.Build();
    if (!operation.IsDone() || operation.HasErrors() || operation.HasWarnings()
        || operation.Shape().IsNull()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT surface trim did not produce an unambiguous intersection");
    }
    const TopoDS_Shape result = operation.Shape();
    if (!BRepCheck_Analyzer(result, true).IsValid()
        || count_subshapes(result, TopAbs_SOLID) != 0
        || count_subshapes(result, TopAbs_FACE) != 1
        || count_subshapes(result, TopAbs_WIRE) != 1) {
      return error_result(STATUS_INVALID_SHAPE, "Surface trim must produce one valid bounded non-solid face");
    }
    GProp_GProps target_properties;
    GProp_GProps result_properties;
    BRepGProp::SurfaceProperties(target.impl().shape, target_properties);
    BRepGProp::SurfaceProperties(result, result_properties);
    const double target_area = target_properties.Mass();
    const double result_area = result_properties.Mass();
    const double tolerance = 1.0e-9 * std::max({target_area, result_area, 1.0});
    if (!std::isfinite(target_area) || !std::isfinite(result_area)
        || result_area <= tolerance || result_area >= target_area - tolerance) {
      return error_result(STATUS_NO_GEOMETRIC_CHANGE, "Surface trim requires a finite proper subset of the target");
    }
    std::vector<HistoryRecord> history;
    append_propagated_history(history, operation, result, target.impl());
    history.push_back(history_record(
        "surface_trim.face", "trimmed", "surface.target", result, result));
    return success_result(result, std::move(history), false, true);
  });
}

std::unique_ptr<NativeOperationResult> extend_planar_surface_native(
    const NativeOperationResult& target, double distance) noexcept {
  return guarded([&] {
    if (!target.valid() || target.impl().shape.IsNull()
        || target.impl().summary.solid_count != 0
        || target.impl().summary.face_count != 1
        || target.impl().summary.wire_count != 1
        || !std::isfinite(distance) || distance < 0.01 || distance > 100000.0) {
      return error_result(STATUS_INVALID_PARAMETER, "Planar surface extend payload is outside the bounded envelope");
    }
    const TopoDS_Face source = face_at_ordinal(target.impl().shape, 0);
    if (source.IsNull()) {
      return error_result(STATUS_INVALID_SHAPE, "Planar surface extend source face is absent");
    }
    const BRepAdaptor_Surface surface(source);
    if (surface.GetType() != GeomAbs_Plane) {
      return error_result(STATUS_INVALID_PARAMETER, "Surface extend currently requires one planar face");
    }
    BRepOffsetAPI_MakeOffset operation(source, GeomAbs_Intersection, false);
    operation.Perform(distance);
    if (!operation.IsDone() || operation.Shape().IsNull()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT planar surface boundary extension did not complete");
    }
    TopoDS_Wire offset_wire;
    for (TopExp_Explorer wires(operation.Shape(), TopAbs_WIRE); wires.More(); wires.Next()) {
      if (!offset_wire.IsNull()) {
        return error_result(STATUS_INVALID_SHAPE, "Surface extend produced an ambiguous multi-wire boundary");
      }
      offset_wire = TopoDS::Wire(wires.Current());
    }
    if (offset_wire.IsNull()) {
      return error_result(STATUS_NULL_RESULT, "Surface extend produced no bounded wire");
    }
    BRepBuilderAPI_MakeFace face_builder(surface.Plane(), offset_wire, true);
    if (!face_builder.IsDone() || face_builder.Face().IsNull()
        || !BRepCheck_Analyzer(face_builder.Face(), true).IsValid()) {
      return error_result(STATUS_INVALID_SHAPE, "Surface extend produced an invalid planar face");
    }
    const TopoDS_Face result = face_builder.Face();
    GProp_GProps source_properties;
    GProp_GProps result_properties;
    BRepGProp::SurfaceProperties(source, source_properties);
    BRepGProp::SurfaceProperties(result, result_properties);
    const double source_area = source_properties.Mass();
    const double result_area = result_properties.Mass();
    const double tolerance = 1.0e-9 * std::max({source_area, result_area, 1.0});
    if (!std::isfinite(source_area) || !std::isfinite(result_area)
        || result_area <= source_area + tolerance) {
      return error_result(STATUS_NO_GEOMETRIC_CHANGE, "Surface extend did not increase the bounded face area");
    }
    std::vector<HistoryRecord> history;
    history.push_back(history_record(
        "surface_extend.face", "extended", "surface.target", result, result));
    return success_result(result, std::move(history), false, true);
  });
}

std::unique_ptr<NativeOperationResult> combine_surfaces_native(
    const NativeOperationResult& base,
    const NativeOperationResult& added) noexcept {
  return guarded([&] {
    if (!base.valid() || !added.valid()
        || base.impl().shape.IsNull() || added.impl().shape.IsNull()
        || base.impl().summary.solid_count != 0 || added.impl().summary.solid_count != 0
        || base.impl().summary.face_count == 0 || added.impl().summary.face_count == 0) {
      return error_result(
          STATUS_INVALID_PARAMETER,
          "Surface compound requires valid non-solid surface inputs");
    }
    const std::uint64_t face_count = static_cast<std::uint64_t>(base.impl().summary.face_count)
        + static_cast<std::uint64_t>(added.impl().summary.face_count);
    if (face_count > 256) {
      return error_result(STATUS_INVALID_PARAMETER, "Surface compound exceeds the face limit");
    }
    BRep_Builder builder;
    TopoDS_Compound compound;
    builder.MakeCompound(compound);
    builder.Add(compound, base.impl().shape);
    builder.Add(compound, added.impl().shape);
    return success_result(compound, {}, false, false, {}, true);
  });
}

std::unique_ptr<NativeOperationResult> knit_surface_compound_native(
    const NativeOperationResult& surfaces,
    double tolerance, bool make_solid) noexcept {
  return guarded([&] {
    if (!surfaces.valid() || surfaces.impl().shape.IsNull()
        || surfaces.impl().summary.solid_count != 0
        || surfaces.impl().summary.face_count < 2
        || surfaces.impl().summary.face_count > 256
        || !std::isfinite(tolerance)
        || tolerance < 1.0e-7 || tolerance > 10.0) {
      return error_result(
          STATUS_INVALID_PARAMETER,
          "Surface knit requires 2..256 non-solid faces and a tolerance from 1e-7 to 10 mm");
    }

    BRepBuilderAPI_Sewing sewing(tolerance, true, true, true, false);
    for (TopExp_Explorer faces(surfaces.impl().shape, TopAbs_FACE); faces.More(); faces.Next()) {
      sewing.Add(faces.Current());
    }
    sewing.Perform();
    const TopoDS_Shape sewed = sewing.SewedShape();
    if (sewed.IsNull() || !BRepCheck_Analyzer(sewed, true).IsValid()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT surface sewing produced no valid result");
    }
    if (sewing.NbMultipleEdges() != 0) {
      return error_result(
          STATUS_INVALID_SHAPE,
          "Surface knit is non-manifold: multiple_edges="
              + std::to_string(sewing.NbMultipleEdges()));
    }
    if (sewing.NbContigousEdges() == 0) {
      return error_result(
          STATUS_NO_GEOMETRIC_CHANGE,
          "Surface knit found no shared boundaries within tolerance="
              + std::to_string(tolerance));
    }
    if (count_subshapes(sewed, TopAbs_SHELL) != 1) {
      return error_result(
          STATUS_INVALID_SHAPE,
          "Surface knit must produce one connected shell; shells="
              + std::to_string(count_subshapes(sewed, TopAbs_SHELL)));
    }

    TopoDS_Shape result = sewed;
    if (make_solid) {
      if (sewing.NbFreeEdges() != 0) {
        return error_result(
            STATUS_INVALID_SHAPE,
            "Surface knit cannot create a solid from an open shell: free_edges="
                + std::to_string(sewing.NbFreeEdges())
                + ", tolerance_mm=" + std::to_string(tolerance));
      }
      TopoDS_Shell shell;
      for (TopExp_Explorer shells(sewed, TopAbs_SHELL); shells.More(); shells.Next()) {
        shell = TopoDS::Shell(shells.Current());
      }
      if (shell.IsNull()) {
        return error_result(STATUS_INVALID_SHAPE, "Surface knit produced no shell for solid conversion");
      }
      BRepBuilderAPI_MakeSolid solid_builder(shell);
      if (!solid_builder.IsDone()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT surface knit solid builder did not complete");
      }
      TopoDS_Solid solid = solid_builder.Solid();
      if (solid.IsNull() || !BRepLib::OrientClosedSolid(solid)
          || !BRepCheck_Analyzer(solid, true).IsValid()
          || count_subshapes(solid, TopAbs_SOLID) != 1) {
        return error_result(
            STATUS_INVALID_SHAPE,
            "Surface knit shell is not a valid closed watertight solid");
      }
      result = solid;
    }

    std::vector<HistoryRecord> history;
    for (TopExp_Explorer faces(result, TopAbs_FACE); faces.More(); faces.Next()) {
      history.push_back(history_record(
          "surface_knit.face", "sewn", "surface.input.face", result, faces.Current()));
    }
    return success_result(result, std::move(history), false, false, {}, !make_solid);
  });
}

std::unique_ptr<NativeOperationResult> thicken_surface_native(
    const NativeOperationResult& surface,
    double thickness, std::uint8_t direction) noexcept {
  return guarded([&] {
    if (!surface.valid() || surface.impl().shape.IsNull()
        || surface.impl().summary.solid_count != 0
        || surface.impl().summary.face_count == 0
        || surface.impl().summary.face_count > 256
        || !std::isfinite(thickness) || thickness < 0.01
        || thickness > 100000.0 || direction > 2) {
      return error_result(
          STATUS_INVALID_PARAMETER,
          "Surface thicken requires a valid non-solid surface, bounded positive thickness, and explicit direction");
    }

    const auto build_half = [&](BRepOffsetAPI_MakeThickSolid& operation, double offset) {
      operation.MakeThickSolidBySimple(surface.impl().shape, offset);
      return operation.IsDone() && !operation.Shape().IsNull();
    };
    const auto oriented_solid = [&](const TopoDS_Shape& candidate) {
      TopoDS_Solid solid;
      if (!candidate.IsNull() && count_subshapes(candidate, TopAbs_SOLID) == 1) {
        for (TopExp_Explorer solids(candidate, TopAbs_SOLID); solids.More(); solids.Next()) {
          solid = TopoDS::Solid(solids.Current());
        }
      }
      if (solid.IsNull()
          || count_subshapes(candidate, TopAbs_FACE) != count_subshapes(solid, TopAbs_FACE)
          || count_subshapes(candidate, TopAbs_EDGE) != count_subshapes(solid, TopAbs_EDGE)
          || !BRepLib::OrientClosedSolid(solid)
          || !BRepCheck_Analyzer(solid, true).IsValid()) {
        return TopoDS_Solid{};
      }
      GProp_GProps properties;
      BRepGProp::VolumeProperties(solid, properties);
      if (!std::isfinite(properties.Mass()) || properties.Mass() <= 1.0e-9) {
        return TopoDS_Solid{};
      }
      return solid;
    };
    const auto result_history = [&](const TopoDS_Shape& result) {
      std::vector<HistoryRecord> history;
      for (TopExp_Explorer faces(result, TopAbs_FACE); faces.More(); faces.Next()) {
        history.push_back(history_record(
            "surface_thicken.face", "thickened", "surface.target", result, faces.Current()));
      }
      return history;
    };

    if (direction != 2) {
      BRepOffsetAPI_MakeThickSolid operation;
      const double offset = direction == 0 ? -thickness : thickness;
      if (!build_half(operation, offset)) {
        return error_result(
            STATUS_INVALID_SHAPE,
            "OCCT surface thicken failed or produced a self-intersecting/non-solid result");
      }
      const TopoDS_Solid result = oriented_solid(operation.Shape());
      if (result.IsNull()) {
        return error_result(
            STATUS_INVALID_SHAPE,
            "OCCT surface thicken did not produce one orientable positive-volume solid");
      }
      return success_result(result, result_history(result));
    }

    BRepOffsetAPI_MakeThickSolid inward;
    BRepOffsetAPI_MakeThickSolid outward;
    if (!build_half(inward, -thickness * 0.5)
        || !build_half(outward, thickness * 0.5)) {
      return error_result(
          STATUS_INVALID_SHAPE,
          "OCCT symmetric surface thicken half failed or self-intersected");
    }
    const TopoDS_Solid inward_solid = oriented_solid(inward.Shape());
    const TopoDS_Solid outward_solid = oriented_solid(outward.Shape());
    if (inward_solid.IsNull() || outward_solid.IsNull()) {
      return error_result(
          STATUS_INVALID_SHAPE,
          "OCCT symmetric surface thicken half was not an orientable positive-volume solid");
    }
    BRepAlgoAPI_Fuse fusion(inward_solid, outward_solid);
    fusion.Build();
    if (!fusion.IsDone() || fusion.HasErrors() || fusion.Shape().IsNull()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT symmetric surface thicken fuse failed");
    }
    fusion.SimplifyResult(true, true);
    const TopoDS_Solid result = oriented_solid(fusion.Shape());
    if (result.IsNull()) {
      return error_result(
          STATUS_INVALID_SHAPE,
          "Symmetric surface thicken did not produce one valid positive-volume solid");
    }
    return success_result(result, result_history(result));
  });
}

std::unique_ptr<NativeOperationResult> offset_planar_region_native(
    rust::Slice<const double> segments,
    rust::Slice<const std::uint32_t> loop_segment_counts,
    double distance) noexcept {
  return guarded([&] {
    if (loop_segment_counts.size() < 2 || loop_segment_counts.size() > 65
        || segments.empty() || segments.size() % 10 != 0
        || !std::isfinite(distance) || std::abs(distance) < 0.01
        || std::abs(distance) > 100000.0) {
      return error_result(STATUS_INVALID_PARAMETER, "Planar region offset payload is malformed");
    }
    std::size_t declared_segments = 0;
    for (const std::uint32_t count : loop_segment_counts) {
      if (count == 0 || count > 64) {
        return error_result(STATUS_INVALID_PARAMETER, "Planar region offset loop count is invalid");
      }
      declared_segments += count;
    }
    if (declared_segments != segments.size() / 10 || declared_segments > 4096) {
      return error_result(STATUS_INVALID_PARAMETER, "Planar region offset segment counts do not match");
    }

    auto build_wire = [&](std::size_t first_segment, std::size_t segment_count) {
      BRepBuilderAPI_MakeWire wire_builder;
      bool line_only = true;
      for (std::size_t index = 0; index < segment_count; ++index) {
        const std::size_t offset = (first_segment + index) * 10;
        for (std::size_t value = 0; value < 10; ++value) {
          if (!std::isfinite(segments[offset + value])
              || std::abs(segments[offset + value]) > 1000000.0) {
            return TopoDS_Wire{};
          }
        }
        const double kind = segments[offset];
        TopoDS_Edge edge;
        if (kind == 0.0) {
          const gp_Pnt start(segments[offset + 1], segments[offset + 2], 0.0);
          const gp_Pnt end(segments[offset + 3], segments[offset + 4], 0.0);
          if (segments[offset + 5] != 0.0 || segments[offset + 6] != 0.0
              || segments[offset + 7] != 0.0 || segments[offset + 8] != 0.0
              || segments[offset + 9] != 0.0
              || start.Distance(end) < 0.01 || start.Distance(end) > 100000.0) {
            return TopoDS_Wire{};
          }
          BRepBuilderAPI_MakeEdge edge_builder(start, end);
          if (!edge_builder.IsDone()) {
            return TopoDS_Wire{};
          }
          edge = edge_builder.Edge();
        } else if (kind == 1.0) {
          line_only = false;
          const gp_Pnt start(segments[offset + 1], segments[offset + 2], 0.0);
          const gp_Pnt end(segments[offset + 3], segments[offset + 4], 0.0);
          const double center_x = segments[offset + 5];
          const double center_y = segments[offset + 6];
          const bool clockwise = segments[offset + 9] != 0.0;
          const gp_Pnt center(center_x, center_y, 0.0);
          const double radius = start.Distance(center);
          const double end_radius = end.Distance(center);
          if (segments[offset + 7] != 0.0 || segments[offset + 8] != 0.0
              || (segments[offset + 9] != 0.0 && segments[offset + 9] != 1.0)
              || radius < 0.01 || radius > 100000.0
              || std::abs(radius - end_radius) > 1.0e-9 * std::max({radius, end_radius, 1.0})
              || std::abs(center_x) + radius > 1000000.0
              || std::abs(center_y) + radius > 1000000.0) {
            return TopoDS_Wire{};
          }
          const double start_angle = std::atan2(start.Y() - center_y, start.X() - center_x);
          const double end_angle = std::atan2(end.Y() - center_y, end.X() - center_x);
          double sweep = end_angle - start_angle;
          const double tau = 2.0 * std::acos(-1.0);
          if (clockwise) {
            while (sweep >= 0.0) sweep -= tau;
          } else {
            while (sweep <= 0.0) sweep += tau;
          }
          const gp_Pnt middle(
              center_x + radius * std::cos(start_angle + sweep / 2.0),
              center_y + radius * std::sin(start_angle + sweep / 2.0),
              0.0);
          GC_MakeArcOfCircle arc_builder(start, middle, end);
          if (!arc_builder.IsDone()) {
            return TopoDS_Wire{};
          }
          BRepBuilderAPI_MakeEdge edge_builder(arc_builder.Value());
          if (!edge_builder.IsDone()) {
            return TopoDS_Wire{};
          }
          edge = edge_builder.Edge();
        } else if (kind == 2.0) {
          line_only = false;
          const gp_Pnt start(segments[offset + 1], segments[offset + 2], 0.0);
          const gp_Pnt end(segments[offset + 3], segments[offset + 4], 0.0);
          const gp_Pnt control_1(segments[offset + 5], segments[offset + 6], 0.0);
          const gp_Pnt control_2(segments[offset + 7], segments[offset + 8], 0.0);
          const double control_polygon_length =
              start.Distance(control_1) + control_1.Distance(control_2) + control_2.Distance(end);
          if (segments[offset + 9] != 0.0 || control_polygon_length < 0.01
              || control_polygon_length > 100000.0) {
            return TopoDS_Wire{};
          }
          edge = cubic_bezier_edge(segments, offset, 0.0);
        } else if (kind == 3.0 && segment_count == 1) {
          line_only = false;
          const double center_x = segments[offset + 1];
          const double center_y = segments[offset + 2];
          const double radius = segments[offset + 3];
          if (radius < 0.01 || radius > 100000.0
              || std::abs(center_x) + radius > 1000000.0
              || std::abs(center_y) + radius > 1000000.0
              || segments[offset + 4] != 0.0 || segments[offset + 5] != 0.0
              || segments[offset + 6] != 0.0 || segments[offset + 7] != 0.0
              || segments[offset + 8] != 0.0
              || (segments[offset + 9] != 0.0 && segments[offset + 9] != 1.0)) {
            return TopoDS_Wire{};
          }
          BRepBuilderAPI_MakeEdge edge_builder(
              gp_Circ(gp_Ax2(gp_Pnt(center_x, center_y, 0.0), gp_Dir(0.0, 0.0, 1.0)), radius));
          if (!edge_builder.IsDone()) {
            return TopoDS_Wire{};
          }
          edge = edge_builder.Edge();
          if (segments[offset + 9] != 0.0) {
            edge.Reverse();
          }
        } else {
          return TopoDS_Wire{};
        }
        if (edge.IsNull()) {
          return TopoDS_Wire{};
        }
        if (kind != 3.0) {
          const std::size_t next = (first_segment + (index + 1) % segment_count) * 10;
          if (segments[offset + 3] != segments[next + 1]
              || segments[offset + 4] != segments[next + 2]) {
            return TopoDS_Wire{};
          }
        }
        wire_builder.Add(edge);
      }
      if (!wire_builder.IsDone() || (line_only && segment_count < 3)) {
        return TopoDS_Wire{};
      }
      return wire_builder.Wire();
    };

    std::vector<TopoDS_Wire> source_wires;
    source_wires.reserve(loop_segment_counts.size());
    std::size_t first_segment = 0;
    for (const std::uint32_t count : loop_segment_counts) {
      TopoDS_Wire wire = build_wire(first_segment, count);
      if (wire.IsNull()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT planar region offset source wire is invalid");
      }
      source_wires.push_back(wire);
      first_segment += count;
    }
    BRepBuilderAPI_MakeFace source_face_builder(source_wires[0], true);
    for (std::size_t index = 1; index < source_wires.size(); ++index) {
      source_face_builder.Add(source_wires[index]);
    }
    source_face_builder.Build();
    if (!source_face_builder.IsDone()
        || !BRepCheck_Analyzer(source_face_builder.Face()).IsValid()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT planar region offset source face is invalid");
    }
    const TopoDS_Face source_face = source_face_builder.Face();

    std::vector<TopoDS_Wire> offset_wires;
    offset_wires.reserve(source_wires.size());
    std::size_t source_segment = 0;
    for (std::size_t index = 0; index < source_wires.size(); ++index) {
      const bool circle_hole = index != 0
          && loop_segment_counts[index] == 1
          && segments[source_segment * 10] == 3.0;
      TopoDS_Wire operation_wire = source_wires[index];
      if (circle_hole) {
        const std::size_t offset = source_segment * 10;
        BRepBuilderAPI_MakeEdge edge_builder(gp_Circ(gp_Ax2(
            gp_Pnt(segments[offset + 1], segments[offset + 2], 0.0),
            gp_Dir(0.0, 0.0, 1.0)), segments[offset + 3]));
        if (!edge_builder.IsDone()) {
          return error_result(STATUS_INVALID_SHAPE, "OCCT planar region circle hole is invalid");
        }
        BRepBuilderAPI_MakeWire wire_builder(edge_builder.Edge());
        if (!wire_builder.IsDone()) {
          return error_result(STATUS_INVALID_SHAPE, "OCCT planar region circle hole is invalid");
        }
        operation_wire = wire_builder.Wire();
      }
      BRepOffsetAPI_MakeOffset operation(operation_wire, GeomAbs_Intersection, false);
      operation.Perform(index == 0 ? distance : -distance);
      if (!operation.IsDone()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT planar region offset did not complete");
      }
      TopoDS_Wire offset_wire;
      for (TopExp_Explorer explorer(operation.Shape(), TopAbs_WIRE); explorer.More(); explorer.Next()) {
        if (!offset_wire.IsNull()) {
          return error_result(STATUS_INVALID_SHAPE, "OCCT planar region offset split a loop");
        }
        offset_wire = TopoDS::Wire(explorer.Current());
      }
      if (offset_wire.IsNull()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT planar region offset collapsed a loop");
      }
      if (circle_hole) {
        offset_wire.Reverse();
      }
      source_segment += loop_segment_counts[index];
      BRepBuilderAPI_MakeFace source_loop_face(source_wires[index], true);
      BRepBuilderAPI_MakeFace offset_loop_face(offset_wire, true);
      if (!source_loop_face.IsDone() || !offset_loop_face.IsDone()
          || !BRepCheck_Analyzer(offset_loop_face.Face()).IsValid()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT planar region offset loop face is invalid");
      }
      GProp_GProps source_loop_properties;
      GProp_GProps offset_loop_properties;
      BRepGProp::SurfaceProperties(source_loop_face.Face(), source_loop_properties);
      BRepGProp::SurfaceProperties(offset_loop_face.Face(), offset_loop_properties);
      const double source_loop_area = source_loop_properties.Mass();
      const double offset_loop_area = offset_loop_properties.Mass();
      const double loop_area_tolerance =
          1.0e-9 * std::max({source_loop_area, offset_loop_area, 1.0});
      const double loop_distance = index == 0 ? distance : -distance;
      if (!std::isfinite(source_loop_area) || !std::isfinite(offset_loop_area)
          || source_loop_area <= 0.0001 || offset_loop_area <= 0.0001
          || (loop_distance > 0.0 && offset_loop_area <= source_loop_area + loop_area_tolerance)
          || (loop_distance < 0.0 && offset_loop_area >= source_loop_area - loop_area_tolerance)) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT planar region offset loop violates signed semantics");
      }
      offset_wires.push_back(offset_wire);
    }

    BRepBuilderAPI_MakeFace result_builder(offset_wires[0], true);
    for (std::size_t index = 1; index < offset_wires.size(); ++index) {
      result_builder.Add(offset_wires[index]);
    }
    result_builder.Build();
    if (!result_builder.IsDone() || !BRepCheck_Analyzer(result_builder.Face()).IsValid()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT planar region offset face is invalid");
    }
    const TopoDS_Face result = result_builder.Face();
    TopTools_IndexedMapOfShape result_wires;
    TopExp::MapShapes(result, TopAbs_WIRE, result_wires);
    if (result_wires.Extent() != static_cast<Standard_Integer>(source_wires.size())) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT planar region offset changed loop topology");
    }
    GProp_GProps source_properties;
    GProp_GProps result_properties;
    BRepGProp::SurfaceProperties(source_face, source_properties);
    BRepGProp::SurfaceProperties(result, result_properties);
    const double source_area = source_properties.Mass();
    const double result_area = result_properties.Mass();
    const double area_tolerance = 1.0e-9 * std::max({source_area, result_area, 1.0});
    if (!std::isfinite(source_area) || !std::isfinite(result_area)
        || source_area <= 0.0001 || result_area <= 0.0001
        || (distance > 0.0 && result_area <= source_area + area_tolerance)
        || (distance < 0.0 && result_area >= source_area - area_tolerance)) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT planar region offset area violates signed semantics");
    }
    std::vector<HistoryRecord> history;
    history.push_back(history_record(
        "planar_offset.face", "offset_generated", "profile.face", result, result));
    return success_result(result, std::move(history), false, true);
  });
}

std::unique_ptr<NativeOperationResult> offset_planar_circle_native(
    double center_x, double center_y, double radius, double distance) noexcept {
  return guarded([&] {
    const double output_radius = radius + distance;
    if (!std::isfinite(center_x) || !std::isfinite(center_y)
        || !std::isfinite(radius) || !std::isfinite(distance)
        || radius < 0.01 || radius > 100000.0
        || std::abs(distance) < 0.01 || std::abs(distance) > 100000.0
        || output_radius < 0.01 || output_radius > 100000.0
        || std::abs(center_x) + radius > 1000000.0
        || std::abs(center_y) + radius > 1000000.0
        || std::abs(center_x) + output_radius > 1000000.0
        || std::abs(center_y) + output_radius > 1000000.0) {
      return error_result(STATUS_INVALID_PARAMETER, "Planar circle offset is outside the bounded envelope");
    }
    BRepBuilderAPI_MakeEdge edge_builder(
        gp_Circ(gp_Ax2(gp_Pnt(center_x, center_y, 0.0), gp_Dir(0.0, 0.0, 1.0)), radius));
    if (!edge_builder.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT planar circle edge did not complete");
    }
    BRepBuilderAPI_MakeWire source_builder(edge_builder.Edge());
    if (!source_builder.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT planar circle wire did not complete");
    }
    BRepBuilderAPI_MakeFace source_face_builder(source_builder.Wire(), true);
    if (!source_face_builder.IsDone()
        || !BRepCheck_Analyzer(source_face_builder.Face()).IsValid()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT planar circle source face is invalid");
    }

    BRepOffsetAPI_MakeOffset operation(source_builder.Wire(), GeomAbs_Intersection, false);
    operation.Perform(distance);
    if (!operation.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT planar circle offset did not complete");
    }
    TopoDS_Wire offset_wire;
    for (TopExp_Explorer explorer(operation.Shape(), TopAbs_WIRE); explorer.More(); explorer.Next()) {
      if (!offset_wire.IsNull()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT planar circle offset produced multiple wires");
      }
      offset_wire = TopoDS::Wire(explorer.Current());
    }
    if (offset_wire.IsNull()) {
      return error_result(STATUS_NULL_RESULT, "OCCT planar circle offset produced no wire");
    }
    BRepBuilderAPI_MakeFace face_builder(offset_wire, true);
    if (!face_builder.IsDone() || !BRepCheck_Analyzer(face_builder.Face()).IsValid()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT planar circle offset face is invalid");
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

std::unique_ptr<NativeOperationResult> sweep_spatial_profile_native_impl(
    rust::Slice<const double> profile_segments,
    rust::Slice<const double> path_segments) noexcept;

std::unique_ptr<NativeOperationResult> sweep_planar_profile_native(
    rust::Slice<const double> profile_segments,
    rust::Slice<const double> path_segments) noexcept {
  if (!path_segments.empty() && path_segments[0] >= 10.0) {
    return sweep_spatial_profile_native_impl(profile_segments, path_segments);
  }
  return guarded([&] {
    if (profile_segments.size() < 20 || profile_segments.size() % 10 != 0
        || path_segments.size() < 20 || path_segments.size() % 10 != 0
        || path_segments.size() > 640) {
      return error_result(STATUS_INVALID_PARAMETER, "OCCT curved Sweep payload is malformed");
    }
    for (double value : profile_segments) {
      if (!std::isfinite(value)) {
        return error_result(STATUS_NON_FINITE_PARAMETER, "OCCT curved Sweep profile is non-finite");
      }
    }
    for (double value : path_segments) {
      if (!std::isfinite(value)) {
        return error_result(STATUS_NON_FINITE_PARAMETER, "OCCT curved Sweep path is non-finite");
      }
    }
    for (std::size_t offset = 0; offset + 10 < path_segments.size(); offset += 10) {
      if (path_segments[offset + 3] != path_segments[offset + 11]
          || path_segments[offset + 4] != path_segments[offset + 12]) {
        return error_result(STATUS_INVALID_PARAMETER, "OCCT curved Sweep path is disconnected");
      }
    }
    const std::size_t last_path_offset = path_segments.size() - 10;
    if (path_segments[1] == path_segments[last_path_offset + 3]
        && path_segments[2] == path_segments[last_path_offset + 4]) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT curved Sweep path must remain geometrically open");
    }

    const auto path_metrics = [&](std::size_t offset, double& length,
                                  gp_Vec& start_tangent, gp_Vec& end_tangent) {
      const double kind = path_segments[offset];
      const double start_x = path_segments[offset + 1];
      const double start_y = path_segments[offset + 2];
      const double end_x = path_segments[offset + 3];
      const double end_y = path_segments[offset + 4];
      if (kind == 0.0) {
        const gp_Vec direction(end_x - start_x, end_y - start_y, 0.0);
        length = direction.Magnitude();
        if (length <= 1.0e-7) return false;
        start_tangent = direction.Normalized();
        end_tangent = start_tangent;
        return true;
      }
      if (kind == 2.0) {
        const double control_1_x = path_segments[offset + 5];
        const double control_1_y = path_segments[offset + 6];
        const double control_2_x = path_segments[offset + 7];
        const double control_2_y = path_segments[offset + 8];
        const gp_Vec chord(end_x - start_x, end_y - start_y, 0.0);
        const gp_Vec start_handle(control_1_x - start_x, control_1_y - start_y, 0.0);
        const gp_Vec middle(control_2_x - control_1_x, control_2_y - control_1_y, 0.0);
        const gp_Vec end_handle(end_x - control_2_x, end_y - control_2_y, 0.0);
        const double chord_squared = chord.SquareMagnitude();
        const double start_length = start_handle.Magnitude();
        const double end_length = end_handle.Magnitude();
        const gp_Vec control_2_from_start(control_2_x - start_x, control_2_y - start_y, 0.0);
        const double projection_1 = start_handle.Dot(chord);
        const double projection_2 = control_2_from_start.Dot(chord);
        length = start_length + middle.Magnitude() + end_length;
        if (start_length <= 1.0e-7 || end_length <= 1.0e-7
            || projection_1 <= 0.0 || projection_2 < projection_1
            || projection_2 >= chord_squared) {
          return false;
        }
        start_tangent = start_handle.Normalized();
        end_tangent = end_handle.Normalized();
        return true;
      }
      if (kind != 1.0) return false;
      const double center_x = path_segments[offset + 5];
      const double center_y = path_segments[offset + 6];
      const double start_dx = start_x - center_x;
      const double start_dy = start_y - center_y;
      const double end_dx = end_x - center_x;
      const double end_dy = end_y - center_y;
      const double radius = std::hypot(start_dx, start_dy);
      const double end_radius = std::hypot(end_dx, end_dy);
      if (radius <= 1.0e-7 || std::abs(radius - end_radius) > 1.0e-9
          || std::hypot(end_x - start_x, end_y - start_y) <= 1.0e-7) {
        return false;
      }
      const bool clockwise = path_segments[offset + 9] != 0.0;
      const double start_angle = std::atan2(start_dy, start_dx);
      const double end_angle = std::atan2(end_dy, end_dx);
      double sweep = end_angle - start_angle;
      const double tau = 2.0 * std::acos(-1.0);
      if (clockwise) {
        if (sweep >= 0.0) sweep -= tau;
      } else if (sweep <= 0.0) {
        sweep += tau;
      }
      length = radius * std::abs(sweep);
      if (length <= 1.0e-7) return false;
      const double sign = clockwise ? -1.0 : 1.0;
      start_tangent = gp_Vec(sign * -start_dy / radius, sign * start_dx / radius, 0.0);
      end_tangent = gp_Vec(sign * -end_dy / end_radius, sign * end_dx / end_radius, 0.0);
      return true;
    };

    const std::size_t path_segment_count = path_segments.size() / 10;
    std::vector<gp_Vec> start_tangents(path_segment_count);
    std::vector<gp_Vec> end_tangents(path_segment_count);
    std::vector<double> segment_lengths(path_segment_count);
    double path_length = 0.0;
    for (std::size_t index = 0; index < path_segment_count; ++index) {
      if (!path_metrics(
              index * 10, segment_lengths[index], start_tangents[index], end_tangents[index])) {
        return error_result(STATUS_INVALID_PARAMETER, "OCCT curved Sweep path violates its bounded segment contract");
      }
      path_length += segment_lengths[index];
      if (index > 0
          && (end_tangents[index - 1].Dot(start_tangents[index]) < 1.0 - 1.0e-9
              || end_tangents[index - 1].Crossed(start_tangents[index]).Magnitude()
                  > 1.0e-9)) {
        return error_result(STATUS_INVALID_PARAMETER, "OCCT curved Sweep path violates its bounded C1 contract");
      }
      const std::size_t offset = index * 10;
      const std::size_t previous = offset - 10;
      if (index > 0 && path_segments[previous] == 1.0 && path_segments[offset] == 1.0
          && path_segments[previous + 5] == path_segments[offset + 5]
          && path_segments[previous + 6] == path_segments[offset + 6]) {
        const double radius = std::hypot(
            path_segments[offset + 1] - path_segments[offset + 5],
            path_segments[offset + 2] - path_segments[offset + 6]);
        if (segment_lengths[index - 1] + segment_lengths[index]
            >= 2.0 * std::acos(-1.0) * radius - 1.0e-7) {
          return error_result(STATUS_INVALID_SHAPE, "OCCT curved Sweep adjacent arcs overlap");
        }
      }
    }
    if (path_length < 0.01 || path_length > 100000.0) {
      return error_result(STATUS_INVALID_PARAMETER, "OCCT curved Sweep path length is outside its bounded contract");
    }

    const auto path_edge = [&](std::size_t offset) {
      const gp_Pnt start(path_segments[offset + 1], path_segments[offset + 2], 0.0);
      const gp_Pnt end(path_segments[offset + 3], path_segments[offset + 4], 0.0);
      if (path_segments[offset] == 0.0) {
        BRepBuilderAPI_MakeEdge builder(start, end);
        return builder.IsDone() ? builder.Edge() : TopoDS_Edge{};
      }
      if (path_segments[offset] == 2.0) {
        return cubic_bezier_edge(path_segments, offset, 0.0);
      }
      const double center_x = path_segments[offset + 5];
      const double center_y = path_segments[offset + 6];
      const double start_angle = std::atan2(start.Y() - center_y, start.X() - center_x);
      const double end_angle = std::atan2(end.Y() - center_y, end.X() - center_x);
      const double radius = start.Distance(gp_Pnt(center_x, center_y, 0.0));
      const bool counterclockwise = path_segments[offset + 9] == 0.0;
      GC_MakeArcOfCircle arc_builder(
          gp_Circ(
              gp_Ax2(gp_Pnt(center_x, center_y, 0.0), gp_Dir(0.0, 0.0, 1.0)),
              radius),
          start_angle,
          end_angle,
          counterclockwise);
      if (!arc_builder.IsDone()) return TopoDS_Edge{};
      BRepBuilderAPI_MakeEdge edge_builder(arc_builder.Value());
      return edge_builder.IsDone() ? edge_builder.Edge() : TopoDS_Edge{};
    };

    BRepBuilderAPI_MakeWire spine_builder;
    std::vector<TopoDS_Edge> path_edges;
    std::vector<Bnd_Box> path_edge_bounds;
    path_edges.reserve(path_segment_count);
    path_edge_bounds.reserve(path_segment_count);
    for (std::size_t offset = 0; offset < path_segments.size(); offset += 10) {
      const TopoDS_Edge edge = path_edge(offset);
      if (edge.IsNull()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT curved Sweep path edge is null");
      }
      path_edges.push_back(edge);
      Bnd_Box edge_bounds;
      BRepBndLib::AddOptimal(edge, edge_bounds, false, false);
      path_edge_bounds.push_back(edge_bounds);
      spine_builder.Add(edge);
    }
    for (std::size_t left = 0; left < path_edges.size(); ++left) {
      for (std::size_t right = left + 1; right < path_edges.size(); ++right) {
        if (path_edge_bounds[left].IsOut(path_edge_bounds[right])) continue;
        BRepExtrema_DistShapeShape distance(path_edges[left], path_edges[right]);
        distance.Perform();
        if (!distance.IsDone()) {
          return error_result(STATUS_INVALID_SHAPE, "OCCT curved Sweep path intersection check failed");
        }
        if (distance.Value() <= 1.0e-7) {
          bool shared_endpoint_only = false;
          if (right == left + 1 && distance.NbSolution() > 0) {
            const gp_Pnt shared(
                path_segments[right * 10 + 1], path_segments[right * 10 + 2], 0.0);
            shared_endpoint_only = true;
            for (Standard_Integer solution = 1; solution <= distance.NbSolution(); ++solution) {
              if (distance.PointOnShape1(solution).Distance(shared) > 1.0e-7
                  || distance.PointOnShape2(solution).Distance(shared) > 1.0e-7) {
                shared_endpoint_only = false;
                break;
              }
            }
          }
          if (!shared_endpoint_only) {
            return error_result(STATUS_INVALID_SHAPE, "OCCT curved Sweep path self-intersects");
          }
        }
      }
    }
    if (!spine_builder.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT curved Sweep spine wire did not complete");
    }
    const TopoDS_Wire spine = spine_builder.Wire();
    if (!BRepCheck_Analyzer(spine).IsValid()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT curved Sweep spine wire is invalid");
    }

    const double path_start_x = path_segments[1];
    const double path_start_y = path_segments[2];
    const double section_x = start_tangents.front().Y();
    const double section_y = -start_tangents.front().X();
    const auto section_point = [&](double u, double v) {
      return gp_Pnt(
          path_start_x + section_x * u,
          path_start_y + section_y * u,
          v);
    };
    BRepBuilderAPI_MakeWire profile_builder;
    for (std::size_t offset = 0; offset < profile_segments.size(); offset += 10) {
      const double kind = profile_segments[offset];
      const gp_Pnt start = section_point(
          profile_segments[offset + 1], profile_segments[offset + 2]);
      const gp_Pnt end = section_point(
          profile_segments[offset + 3], profile_segments[offset + 4]);
      TopoDS_Edge edge;
      if (kind == 0.0) {
        BRepBuilderAPI_MakeEdge edge_builder(start, end);
        if (edge_builder.IsDone()) edge = edge_builder.Edge();
      } else if (kind == 1.0) {
        const double center_u = profile_segments[offset + 5];
        const double center_v = profile_segments[offset + 6];
        const double start_angle = std::atan2(
            profile_segments[offset + 2] - center_v,
            profile_segments[offset + 1] - center_u);
        const double end_angle = std::atan2(
            profile_segments[offset + 4] - center_v,
            profile_segments[offset + 3] - center_u);
        double sweep = end_angle - start_angle;
        const double tau = 2.0 * std::acos(-1.0);
        if (profile_segments[offset + 9] != 0.0) {
          if (sweep >= 0.0) sweep -= tau;
        } else if (sweep <= 0.0) {
          sweep += tau;
        }
        const double radius = std::hypot(
            profile_segments[offset + 1] - center_u,
            profile_segments[offset + 2] - center_v);
        const double middle_angle = start_angle + sweep / 2.0;
        const gp_Pnt middle = section_point(
            center_u + radius * std::cos(middle_angle),
            center_v + radius * std::sin(middle_angle));
        GC_MakeArcOfCircle arc_builder(start, middle, end);
        if (arc_builder.IsDone()) {
          BRepBuilderAPI_MakeEdge edge_builder(arc_builder.Value());
          if (edge_builder.IsDone()) edge = edge_builder.Edge();
        }
      } else if (kind == 2.0) {
        TColgp_Array1OfPnt poles(1, 4);
        poles.SetValue(1, start);
        poles.SetValue(2, section_point(
            profile_segments[offset + 5], profile_segments[offset + 6]));
        poles.SetValue(3, section_point(
            profile_segments[offset + 7], profile_segments[offset + 8]));
        poles.SetValue(4, end);
        occ::handle<Geom_BezierCurve> curve = new Geom_BezierCurve(poles);
        BRepBuilderAPI_MakeEdge edge_builder(curve);
        if (edge_builder.IsDone()) edge = edge_builder.Edge();
      } else {
        return error_result(STATUS_INVALID_PARAMETER, "OCCT curved Sweep profile kind is invalid");
      }
      if (edge.IsNull()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT curved Sweep profile edge is null");
      }
      profile_builder.Add(edge);
    }
    if (!profile_builder.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT curved Sweep profile wire did not complete");
    }
    const TopoDS_Wire profile = profile_builder.Wire();
    if (!BRepCheck_Analyzer(profile).IsValid()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT curved Sweep profile wire is invalid");
    }

    BRepOffsetAPI_MakePipeShell operation(spine);
    operation.SetMode(gp_Dir(0.0, 0.0, 1.0));
    operation.SetTolerance(1.0e-7, 1.0e-7, 1.0e-9);
    operation.Add(profile, false, false);
    if (!operation.IsReady()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT curved Sweep pipe is not ready");
    }
    operation.Build();
    if (!operation.IsDone() || !operation.MakeSolid()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT curved Sweep pipe did not produce a solid");
    }
    const TopoDS_Shape result = operation.Shape();
    if (result.IsNull() || !BRepCheck_Analyzer(result).IsValid()
        || count_subshapes(result, TopAbs_SOLID) != 1) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT curved Sweep result is not one valid solid");
    }
    std::vector<HistoryRecord> history;
    history.push_back(history_record(
        "sweep.start", "first_shape", "profile.wire", result, operation.FirstShape()));
    history.push_back(history_record(
        "sweep.end", "last_shape", "profile.wire", result, operation.LastShape()));
    return success_result(result, std::move(history));
  });
}

std::unique_ptr<NativeOperationResult> sweep_spatial_profile_native_impl(
    rust::Slice<const double> profile_segments,
    rust::Slice<const double> path_segments) noexcept {
  return guarded([&] {
    constexpr std::size_t spatial_stride = 14;
    constexpr double epsilon = 1.0e-9;
    constexpr double minimum_segment_length = 1.0e-7;
    constexpr double coordinate_limit = 1000000.0;
    const double tau = 2.0 * std::acos(-1.0);
    const auto bounded = [&](double value) {
      return std::isfinite(value) && std::abs(value) <= coordinate_limit;
    };
    if (profile_segments.size() < 20 || profile_segments.size() > 640
        || profile_segments.size() % 10 != 0
        || path_segments.size() < spatial_stride
        || path_segments.size() > 64 * spatial_stride
        || path_segments.size() % spatial_stride != 0) {
      return error_result(STATUS_INVALID_PARAMETER, "OCCT spatial Sweep payload is malformed");
    }
    for (double value : profile_segments) {
      if (!bounded(value)) {
        return error_result(
            std::isfinite(value) ? STATUS_INVALID_PARAMETER : STATUS_NON_FINITE_PARAMETER,
            "OCCT spatial Sweep profile value is outside its bounded contract");
      }
    }
    for (double value : path_segments) {
      if (!bounded(value)) {
        return error_result(
            std::isfinite(value) ? STATUS_INVALID_PARAMETER : STATUS_NON_FINITE_PARAMETER,
            "OCCT spatial Sweep path value is outside its bounded contract");
      }
    }

    const std::size_t path_count = path_segments.size() / spatial_stride;
    std::vector<double> lengths(path_count);
    std::vector<gp_Vec> start_tangents(path_count);
    std::vector<gp_Vec> end_tangents(path_count);
    const auto point_at = [&](std::size_t offset, std::size_t first) {
      return gp_Pnt(
          path_segments[offset + first],
          path_segments[offset + first + 1],
          path_segments[offset + first + 2]);
    };
    const auto positive_remainder = [&](double value) {
      const double remainder = std::fmod(value, tau);
      return remainder < 0.0 ? remainder + tau : remainder;
    };
    const auto metrics = [&](std::size_t offset, double& length,
                             gp_Vec& start_tangent, gp_Vec& end_tangent) {
      const double kind = path_segments[offset];
      const gp_Pnt start = point_at(offset, 1);
      const gp_Pnt end = point_at(offset, 4);
      if (kind == 10.0) {
        const gp_Vec direction(start, end);
        length = direction.Magnitude();
        if (!std::isfinite(length) || length <= minimum_segment_length) return false;
        start_tangent = direction.Normalized();
        end_tangent = start_tangent;
        return true;
      }
      if (kind == 12.0) {
        const gp_Pnt control_1 = point_at(offset, 7);
        const gp_Pnt control_2 = point_at(offset, 10);
        const gp_Vec chord(start, end);
        const gp_Vec first(start, control_1);
        const gp_Vec middle(control_1, control_2);
        const gp_Vec last(control_2, end);
        const gp_Vec control_2_from_start(start, control_2);
        const double first_length = first.Magnitude();
        const double last_length = last.Magnitude();
        const double projection_1 = first.Dot(chord);
        const double projection_2 = control_2_from_start.Dot(chord);
        length = first_length + middle.Magnitude() + last_length;
        if (!std::isfinite(length) || first_length <= minimum_segment_length
            || last_length <= minimum_segment_length || projection_1 <= 0.0
            || projection_2 < projection_1
            || projection_2 >= chord.SquareMagnitude()) {
          return false;
        }
        start_tangent = first.Normalized();
        end_tangent = last.Normalized();
        return true;
      }
      if (kind != 11.0 || (path_segments[offset + 13] != 0.0
                           && path_segments[offset + 13] != 1.0)) {
        return false;
      }
      const gp_Pnt center = point_at(offset, 7);
      const gp_Vec normal(
          path_segments[offset + 10],
          path_segments[offset + 11],
          path_segments[offset + 12]);
      const double normal_length = normal.Magnitude();
      if (!std::isfinite(normal_length) || std::abs(normal_length - 1.0) > epsilon) {
        return false;
      }
      const gp_Vec unit_normal = normal.Normalized();
      const gp_Vec start_radius(center, start);
      const gp_Vec end_radius(center, end);
      const double radius = start_radius.Magnitude();
      const double end_radius_length = end_radius.Magnitude();
      if (radius <= minimum_segment_length || start.Distance(end) == 0.0
          || std::abs(radius - end_radius_length) > epsilon
          || std::abs(start_radius.Dot(unit_normal)) > epsilon
          || std::abs(end_radius.Dot(unit_normal)) > epsilon) {
        return false;
      }
      const double signed_angle = std::atan2(
          unit_normal.Dot(start_radius.Crossed(end_radius)),
          start_radius.Dot(end_radius));
      const bool clockwise = path_segments[offset + 13] != 0.0;
      const double angle = positive_remainder(clockwise ? -signed_angle : signed_angle);
      length = radius * angle;
      if (!std::isfinite(length) || length <= minimum_segment_length) return false;
      const double sign = clockwise ? -1.0 : 1.0;
      start_tangent = unit_normal.Crossed(start_radius).Multiplied(sign).Normalized();
      end_tangent = unit_normal.Crossed(end_radius).Multiplied(sign).Normalized();
      return true;
    };

    double path_length = 0.0;
    for (std::size_t index = 0; index < path_count; ++index) {
      const std::size_t offset = index * spatial_stride;
      if (!metrics(offset, lengths[index], start_tangents[index], end_tangents[index])) {
        return error_result(
            STATUS_INVALID_PARAMETER,
            "OCCT spatial Sweep path violates its bounded segment contract");
      }
      path_length += lengths[index];
      if (index == 0) continue;
      const std::size_t previous = offset - spatial_stride;
      if (point_at(previous, 4).Distance(point_at(offset, 1)) != 0.0) {
        return error_result(STATUS_INVALID_PARAMETER, "OCCT spatial Sweep path is disconnected");
      }
      if (end_tangents[index - 1].Dot(start_tangents[index]) < 1.0 - epsilon
          || end_tangents[index - 1].Crossed(start_tangents[index]).Magnitude() > epsilon) {
        return error_result(
            STATUS_INVALID_PARAMETER,
            "OCCT spatial Sweep path violates its bounded C1 contract");
      }
      if (path_segments[previous] == 11.0 && path_segments[offset] == 11.0
          && point_at(previous, 7).Distance(point_at(offset, 7)) == 0.0) {
        const double radius = point_at(offset, 1).Distance(point_at(offset, 7));
        if (lengths[index - 1] + lengths[index] >= tau * radius - epsilon) {
          return error_result(STATUS_INVALID_SHAPE, "OCCT spatial Sweep adjacent arcs overlap");
        }
      }
    }
    if (!std::isfinite(path_length) || path_length < 0.01 || path_length > 100000.0) {
      return error_result(
          STATUS_INVALID_PARAMETER,
          "OCCT spatial Sweep path length is outside its bounded contract");
    }
    const bool closed =
        point_at(0, 1).Distance(point_at((path_count - 1) * spatial_stride, 4)) == 0.0;
    if (closed
        && (end_tangents.back().Dot(start_tangents.front()) < 1.0 - epsilon
            || end_tangents.back().Crossed(start_tangents.front()).Magnitude() > epsilon)) {
      return error_result(
          STATUS_INVALID_PARAMETER, "OCCT closed spatial Sweep seam violates its C1 contract");
    }

    const auto make_path_edge = [&](std::size_t offset) {
      const gp_Pnt start = point_at(offset, 1);
      const gp_Pnt end = point_at(offset, 4);
      if (path_segments[offset] == 10.0) {
        BRepBuilderAPI_MakeEdge builder(start, end);
        return builder.IsDone() ? builder.Edge() : TopoDS_Edge{};
      }
      if (path_segments[offset] == 12.0) {
        TColgp_Array1OfPnt poles(1, 4);
        poles.SetValue(1, start);
        poles.SetValue(2, point_at(offset, 7));
        poles.SetValue(3, point_at(offset, 10));
        poles.SetValue(4, end);
        occ::handle<Geom_BezierCurve> curve = new Geom_BezierCurve(poles);
        BRepBuilderAPI_MakeEdge builder(curve);
        return builder.IsDone() ? builder.Edge() : TopoDS_Edge{};
      }
      const gp_Pnt center = point_at(offset, 7);
      const gp_Vec normal(
          path_segments[offset + 10],
          path_segments[offset + 11],
          path_segments[offset + 12]);
      const gp_Vec start_radius(center, start);
      const gp_Vec end_radius(center, end);
      const double signed_angle = std::atan2(
          normal.Dot(start_radius.Crossed(end_radius)), start_radius.Dot(end_radius));
      const bool clockwise = path_segments[offset + 13] != 0.0;
      const double angle = positive_remainder(clockwise ? -signed_angle : signed_angle);
      const double rotation = clockwise ? -angle : angle;
      const gp_Vec middle_radius = start_radius.Rotated(
          gp_Ax1(center, gp_Dir(normal)), rotation / 2.0);
      const gp_Pnt middle = center.Translated(middle_radius);
      GC_MakeArcOfCircle arc_builder(start, middle, end);
      if (!arc_builder.IsDone()) return TopoDS_Edge{};
      BRepBuilderAPI_MakeEdge edge_builder(arc_builder.Value());
      return edge_builder.IsDone() ? edge_builder.Edge() : TopoDS_Edge{};
    };

    BRepBuilderAPI_MakeWire spine_builder;
    std::vector<TopoDS_Edge> path_edges;
    path_edges.reserve(path_count);
    for (std::size_t index = 0; index < path_count; ++index) {
      const TopoDS_Edge edge = make_path_edge(index * spatial_stride);
      if (edge.IsNull()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT spatial Sweep path edge is null");
      }
      path_edges.push_back(edge);
      spine_builder.Add(edge);
    }
    for (std::size_t left = 0; left < path_edges.size(); ++left) {
      for (std::size_t right = left + 1; right < path_edges.size(); ++right) {
        BRepExtrema_DistShapeShape distance(path_edges[left], path_edges[right]);
        distance.Perform();
        if (!distance.IsDone()) {
          return error_result(
              STATUS_INVALID_SHAPE, "OCCT spatial Sweep path intersection check failed");
        }
        if (distance.Value() > minimum_segment_length) continue;
        bool shared_endpoint_only = false;
        const bool adjacent = right == left + 1
            || (closed && left == 0 && right + 1 == path_edges.size());
        if (adjacent && distance.NbSolution() > 0) {
          const gp_Pnt shared = right == left + 1
              ? point_at(right * spatial_stride, 1)
              : point_at(0, 1);
          shared_endpoint_only = true;
          for (Standard_Integer solution = 1; solution <= distance.NbSolution(); ++solution) {
            if (distance.PointOnShape1(solution).Distance(shared) > minimum_segment_length
                || distance.PointOnShape2(solution).Distance(shared) > minimum_segment_length) {
              shared_endpoint_only = false;
              break;
            }
          }
        }
        if (!shared_endpoint_only) {
          return error_result(STATUS_INVALID_SHAPE, "OCCT spatial Sweep path self-intersects");
        }
      }
    }
    if (!spine_builder.IsDone() || !BRepCheck_Analyzer(spine_builder.Wire()).IsValid()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT spatial Sweep spine wire is invalid");
    }
    TopoDS_Wire spine = spine_builder.Wire();
    spine.Closed(closed);

    const gp_Pnt path_start = point_at(0, 1);
    const gp_Vec tangent = start_tangents.front();
    gp_Vec reference(0.0, 0.0, 1.0);
    if (tangent.Crossed(reference).SquareMagnitude() <= epsilon * epsilon) {
      reference = gp_Vec(0.0, 1.0, 0.0);
    }
    const gp_Vec frame_u = tangent.Crossed(reference).Normalized();
    const gp_Vec frame_v = frame_u.Crossed(tangent).Normalized();
    const auto section_point = [&](double u, double v) {
      return path_start.Translated(frame_u.Multiplied(u).Added(frame_v.Multiplied(v)));
    };

    BRepBuilderAPI_MakeWire profile_builder;
    for (std::size_t offset = 0; offset < profile_segments.size(); offset += 10) {
      const std::size_t next = (offset + 10) % profile_segments.size();
      if (profile_segments[offset + 3] != profile_segments[next + 1]
          || profile_segments[offset + 4] != profile_segments[next + 2]) {
        return error_result(STATUS_INVALID_PARAMETER, "OCCT spatial Sweep profile is open");
      }
      const double kind = profile_segments[offset];
      const gp_Pnt start = section_point(profile_segments[offset + 1], profile_segments[offset + 2]);
      const gp_Pnt end = section_point(profile_segments[offset + 3], profile_segments[offset + 4]);
      TopoDS_Edge edge;
      if (kind == 0.0) {
        BRepBuilderAPI_MakeEdge builder(start, end);
        if (builder.IsDone()) edge = builder.Edge();
      } else if (kind == 1.0) {
        const double center_u = profile_segments[offset + 5];
        const double center_v = profile_segments[offset + 6];
        const double start_angle = std::atan2(
            profile_segments[offset + 2] - center_v,
            profile_segments[offset + 1] - center_u);
        const double end_angle = std::atan2(
            profile_segments[offset + 4] - center_v,
            profile_segments[offset + 3] - center_u);
        double sweep = end_angle - start_angle;
        if (profile_segments[offset + 9] != 0.0) {
          if (sweep >= 0.0) sweep -= tau;
        } else if (sweep <= 0.0) {
          sweep += tau;
        }
        const double radius = std::hypot(
            profile_segments[offset + 1] - center_u,
            profile_segments[offset + 2] - center_v);
        const gp_Pnt middle = section_point(
            center_u + radius * std::cos(start_angle + sweep / 2.0),
            center_v + radius * std::sin(start_angle + sweep / 2.0));
        GC_MakeArcOfCircle arc_builder(start, middle, end);
        if (arc_builder.IsDone()) {
          BRepBuilderAPI_MakeEdge builder(arc_builder.Value());
          if (builder.IsDone()) edge = builder.Edge();
        }
      } else if (kind == 2.0) {
        TColgp_Array1OfPnt poles(1, 4);
        poles.SetValue(1, start);
        poles.SetValue(2, section_point(profile_segments[offset + 5], profile_segments[offset + 6]));
        poles.SetValue(3, section_point(profile_segments[offset + 7], profile_segments[offset + 8]));
        poles.SetValue(4, end);
        occ::handle<Geom_BezierCurve> curve = new Geom_BezierCurve(poles);
        BRepBuilderAPI_MakeEdge builder(curve);
        if (builder.IsDone()) edge = builder.Edge();
      } else {
        return error_result(STATUS_INVALID_PARAMETER, "OCCT spatial Sweep profile kind is invalid");
      }
      if (edge.IsNull()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT spatial Sweep profile edge is null");
      }
      profile_builder.Add(edge);
    }
    if (!profile_builder.IsDone() || !BRepCheck_Analyzer(profile_builder.Wire()).IsValid()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT spatial Sweep profile wire is invalid");
    }
    const TopoDS_Wire profile = profile_builder.Wire();

    BRepOffsetAPI_MakePipeShell operation(spine);
    // Corrected Frenet is OCCT's deterministic minimum-twist transport mode.
    operation.SetMode(false);
    operation.SetTolerance(1.0e-7, 1.0e-7, 1.0e-9);
    operation.Add(profile, false, false);
    if (!operation.IsReady()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT spatial Sweep pipe is not ready");
    }
    operation.Build();
    if (!operation.IsDone() || !operation.MakeSolid()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT spatial Sweep pipe did not produce a solid");
    }
    const TopoDS_Shape result = operation.Shape();
    if (result.IsNull() || !BRepCheck_Analyzer(result).IsValid()
        || count_subshapes(result, TopAbs_SOLID) != 1) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT spatial Sweep result is not one valid solid");
    }
    std::vector<HistoryRecord> history;
    history.push_back(history_record(
        "sweep.start", "first_shape", "profile.wire", result, operation.FirstShape()));
    history.push_back(history_record(
        "sweep.end", "last_shape", "profile.wire", result, operation.LastShape()));
    return success_result(result, std::move(history));
  });
}

std::unique_ptr<NativeOperationResult> loft_framed_profiles_native(
    rust::Slice<const double> values,
    rust::Slice<const double> guide_segments,
    std::uint8_t continuity,
    bool make_solid) noexcept {
  return guarded([&] {
    if (values.empty() || !std::isfinite(values[0])) {
      return error_result(STATUS_INVALID_PARAMETER, "OCCT framed Loft payload is malformed");
    }
    const std::size_t section_count = static_cast<std::size_t>(values[0]);
    if (section_count < 2 || section_count > 16
        || values[0] != static_cast<double>(section_count)) {
      return error_result(STATUS_INVALID_PARAMETER, "OCCT framed Loft section count is invalid");
    }
    std::size_t cursor = 1;
    double previous_elevation = -std::numeric_limits<double>::infinity();
    std::vector<TopoDS_Wire> wires;
    std::vector<TopoDS_Edge> section_edges;
    wires.reserve(section_count);
    section_edges.reserve(section_count);
    for (std::size_t section = 0; section < section_count; ++section) {
      if (cursor + 15 > values.size()) {
        return error_result(STATUS_INVALID_PARAMETER, "OCCT framed Loft section payload is truncated");
      }
      const double kind = values[cursor++];
      const double elevation = values[cursor++];
      const std::size_t count = static_cast<std::size_t>(values[cursor++]);
      const double* frame = values.data() + cursor;
      cursor += 12;
      if (!std::isfinite(kind) || !std::isfinite(elevation)
          || elevation <= previous_elevation || count == 0 || count > 64) {
        return error_result(STATUS_INVALID_PARAMETER, "OCCT framed Loft section header is invalid");
      }
      previous_elevation = elevation;
      for (std::size_t index = 0; index < 12; ++index) {
        if (!std::isfinite(frame[index]) || std::abs(frame[index]) > 1000000.0) {
          return error_result(STATUS_INVALID_PARAMETER, "OCCT framed Loft frame is invalid");
        }
      }
      const gp_Vec x_axis(frame[3], frame[4], frame[5]);
      const gp_Vec y_axis(frame[6], frame[7], frame[8]);
      const gp_Vec normal(frame[9], frame[10], frame[11]);
      if (std::abs(x_axis.Magnitude() - 1.0) > 1.0e-8
          || std::abs(y_axis.Magnitude() - 1.0) > 1.0e-8
          || std::abs(normal.Magnitude() - 1.0) > 1.0e-8
          || std::abs(x_axis.Dot(y_axis)) > 1.0e-8
          || std::abs(x_axis.Dot(normal)) > 1.0e-8
          || std::abs(y_axis.Dot(normal)) > 1.0e-8
          || x_axis.Crossed(y_axis).Dot(normal) < 1.0 - 1.0e-8) {
        return error_result(STATUS_INVALID_PARAMETER, "OCCT framed Loft frame is not right-handed orthonormal");
      }
      const auto world_point = [&](double x, double y) {
        return gp_Pnt(
            frame[0] + frame[3] * x + frame[6] * y + frame[9] * elevation,
            frame[1] + frame[4] * x + frame[7] * y + frame[10] * elevation,
            frame[2] + frame[5] * x + frame[8] * y + frame[11] * elevation);
      };
      BRepBuilderAPI_MakeWire wire_builder;
      TopoDS_Edge first_edge;
      if (kind == 0.0) {
        if (cursor + count * 10 > values.size() || count < 2) {
          return error_result(STATUS_INVALID_PARAMETER, "OCCT framed Loft boundary payload is invalid");
        }
        bool line_only = true;
        for (std::size_t index = 0; index < count; ++index) {
          const std::size_t offset = cursor + index * 10;
          for (std::size_t value = 0; value < 10; ++value) {
            if (!std::isfinite(values[offset + value])
                || std::abs(values[offset + value]) > 1000000.0) {
              return error_result(STATUS_INVALID_PARAMETER, "OCCT framed Loft boundary value is invalid");
            }
          }
          const double segment_kind = values[offset];
          const gp_Pnt start = world_point(values[offset + 1], values[offset + 2]);
          const gp_Pnt end = world_point(values[offset + 3], values[offset + 4]);
          TopoDS_Edge edge;
          if (segment_kind == 0.0) {
            if (values[offset + 5] != 0.0 || values[offset + 6] != 0.0
                || values[offset + 7] != 0.0 || values[offset + 8] != 0.0
                || values[offset + 9] != 0.0 || start.Distance(end) < 0.01
                || start.Distance(end) > 100000.0) {
              return error_result(STATUS_INVALID_PARAMETER, "OCCT framed Loft line is invalid");
            }
            BRepBuilderAPI_MakeEdge edge_builder(start, end);
            if (edge_builder.IsDone()) edge = edge_builder.Edge();
          } else if (segment_kind == 1.0) {
            line_only = false;
            const double center_x = values[offset + 5];
            const double center_y = values[offset + 6];
            const double radius = std::hypot(values[offset + 1] - center_x,
                                             values[offset + 2] - center_y);
            const double end_radius = std::hypot(values[offset + 3] - center_x,
                                                 values[offset + 4] - center_y);
            if (values[offset + 7] != 0.0 || values[offset + 8] != 0.0
                || (values[offset + 9] != 0.0 && values[offset + 9] != 1.0)
                || radius < 0.01 || radius > 100000.0
                || std::abs(radius - end_radius) > 1.0e-9 * std::max({radius, end_radius, 1.0})) {
              return error_result(STATUS_INVALID_PARAMETER, "OCCT framed Loft arc is invalid");
            }
            const double tau = 2.0 * std::acos(-1.0);
            const double start_angle = std::atan2(values[offset + 2] - center_y,
                                                  values[offset + 1] - center_x);
            const double end_angle = std::atan2(values[offset + 4] - center_y,
                                                values[offset + 3] - center_x);
            double sweep = end_angle - start_angle;
            if (values[offset + 9] != 0.0) {
              while (sweep >= 0.0) sweep -= tau;
            } else {
              while (sweep <= 0.0) sweep += tau;
            }
            const double middle_angle = start_angle + sweep / 2.0;
            const gp_Pnt middle = world_point(
                center_x + radius * std::cos(middle_angle),
                center_y + radius * std::sin(middle_angle));
            GC_MakeArcOfCircle arc_builder(start, middle, end);
            if (arc_builder.IsDone()) {
              BRepBuilderAPI_MakeEdge edge_builder(arc_builder.Value());
              if (edge_builder.IsDone()) edge = edge_builder.Edge();
            }
          } else if (segment_kind == 2.0) {
            line_only = false;
            if (values[offset + 9] != 0.0) {
              return error_result(STATUS_INVALID_PARAMETER, "OCCT framed Loft cubic is invalid");
            }
            const gp_Pnt control_1 = world_point(values[offset + 5], values[offset + 6]);
            const gp_Pnt control_2 = world_point(values[offset + 7], values[offset + 8]);
            const double length = start.Distance(control_1) + control_1.Distance(control_2)
                                  + control_2.Distance(end);
            if (!std::isfinite(length) || length < 0.01 || length > 100000.0) {
              return error_result(STATUS_INVALID_PARAMETER, "OCCT framed Loft cubic length is invalid");
            }
            TColgp_Array1OfPnt poles(1, 4);
            poles.SetValue(1, start);
            poles.SetValue(2, control_1);
            poles.SetValue(3, control_2);
            poles.SetValue(4, end);
            occ::handle<Geom_BezierCurve> curve = new Geom_BezierCurve(poles);
            BRepBuilderAPI_MakeEdge edge_builder(curve);
            if (edge_builder.IsDone()) edge = edge_builder.Edge();
          } else {
            return error_result(STATUS_INVALID_PARAMETER, "OCCT framed Loft boundary kind is invalid");
          }
          const std::size_t next = cursor + ((index + 1) % count) * 10;
          if (values[offset + 3] != values[next + 1]
              || values[offset + 4] != values[next + 2]) {
            return error_result(STATUS_INVALID_PARAMETER, "OCCT framed Loft boundary is open");
          }
          if (edge.IsNull()) {
            return error_result(STATUS_INVALID_SHAPE, "OCCT framed Loft boundary edge is null");
          }
          if (first_edge.IsNull()) first_edge = edge;
          wire_builder.Add(edge);
        }
        if (line_only && count < 3) {
          return error_result(STATUS_INVALID_SHAPE, "OCCT framed Loft polygon is degenerate");
        }
        cursor += count * 10;
      } else if (kind == 1.0) {
        if (count != 1 || cursor + 3 > values.size()) {
          return error_result(STATUS_INVALID_PARAMETER, "OCCT framed Loft circle payload is invalid");
        }
        const double center_x = values[cursor++];
        const double center_y = values[cursor++];
        const double radius = values[cursor++];
        if (!std::isfinite(center_x) || !std::isfinite(center_y) || !std::isfinite(radius)
            || radius < 0.01 || radius > 100000.0) {
          return error_result(STATUS_INVALID_PARAMETER, "OCCT framed Loft circle is invalid");
        }
        BRepBuilderAPI_MakeEdge edge_builder(gp_Circ(
            gp_Ax2(world_point(center_x, center_y), gp_Dir(normal), gp_Dir(x_axis)), radius));
        if (!edge_builder.IsDone()) {
          return error_result(STATUS_INVALID_SHAPE, "OCCT framed Loft circle edge is null");
        }
        first_edge = edge_builder.Edge();
        wire_builder.Add(first_edge);
      } else if (kind == 2.0) {
        if (count < 4 || cursor + count * 2 > values.size()) {
          return error_result(STATUS_INVALID_PARAMETER, "OCCT framed Loft spline payload is invalid");
        }
        occ::handle<TColgp_HArray1OfPnt> points =
            new TColgp_HArray1OfPnt(1, static_cast<Standard_Integer>(count));
        for (std::size_t point = 0; point < count; ++point) {
          const double x = values[cursor++];
          const double y = values[cursor++];
          if (!std::isfinite(x) || !std::isfinite(y)
              || std::abs(x) > 1000000.0 || std::abs(y) > 1000000.0) {
            return error_result(STATUS_INVALID_PARAMETER, "OCCT framed Loft spline point is invalid");
          }
          points->SetValue(static_cast<Standard_Integer>(point + 1), world_point(x, y));
        }
        GeomAPI_Interpolate interpolation(points, true, 1.0e-9);
        interpolation.Perform();
        if (!interpolation.IsDone()) {
          return error_result(STATUS_INVALID_SHAPE, "OCCT framed Loft spline interpolation failed");
        }
        BRepBuilderAPI_MakeEdge edge_builder(interpolation.Curve());
        if (!edge_builder.IsDone()) {
          return error_result(STATUS_INVALID_SHAPE, "OCCT framed Loft spline edge is null");
        }
        first_edge = edge_builder.Edge();
        wire_builder.Add(first_edge);
      } else {
        return error_result(STATUS_INVALID_PARAMETER, "OCCT framed Loft profile kind is invalid");
      }
      if (!wire_builder.IsDone() || !BRepCheck_Analyzer(wire_builder.Wire()).IsValid()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT framed Loft wire is invalid");
      }
      wires.push_back(wire_builder.Wire());
      section_edges.push_back(first_edge);
    }
    if (cursor != values.size()) {
      return error_result(STATUS_INVALID_PARAMETER, "OCCT framed Loft payload has trailing values");
    }
    if (continuity > 2 || (!guide_segments.empty() && continuity == 2)) {
      return error_result(STATUS_INVALID_PARAMETER, "OCCT framed Loft continuity is unsupported");
    }
    if (!guide_segments.empty()) {
      constexpr std::size_t spatial_stride = 14;
      if (guide_segments.size() % spatial_stride != 0) {
        return error_result(STATUS_INVALID_PARAMETER, "OCCT guided Loft path payload is malformed");
      }
      BRepBuilderAPI_MakeWire spine_builder;
      gp_Pnt previous_end;
      bool has_previous = false;
      for (std::size_t offset = 0; offset < guide_segments.size(); offset += spatial_stride) {
        for (std::size_t index = 0; index < spatial_stride; ++index) {
          if (!std::isfinite(guide_segments[offset + index])) {
            return error_result(STATUS_INVALID_PARAMETER, "OCCT guided Loft path is not finite");
          }
        }
        const gp_Pnt start(
            guide_segments[offset + 1], guide_segments[offset + 2], guide_segments[offset + 3]);
        const gp_Pnt end(
            guide_segments[offset + 4], guide_segments[offset + 5], guide_segments[offset + 6]);
        if (has_previous && previous_end.Distance(start) > 1.0e-7) {
          return error_result(STATUS_INVALID_PARAMETER, "OCCT guided Loft path is disconnected");
        }
        TopoDS_Edge edge;
        if (guide_segments[offset] == 10.0) {
          BRepBuilderAPI_MakeEdge builder(start, end);
          if (builder.IsDone()) edge = builder.Edge();
        } else if (guide_segments[offset] == 12.0) {
          TColgp_Array1OfPnt poles(1, 4);
          poles.SetValue(1, start);
          poles.SetValue(2, gp_Pnt(
              guide_segments[offset + 7], guide_segments[offset + 8], guide_segments[offset + 9]));
          poles.SetValue(3, gp_Pnt(
              guide_segments[offset + 10], guide_segments[offset + 11], guide_segments[offset + 12]));
          poles.SetValue(4, end);
          occ::handle<Geom_BezierCurve> curve = new Geom_BezierCurve(poles);
          BRepBuilderAPI_MakeEdge builder(curve);
          if (builder.IsDone()) edge = builder.Edge();
        } else if (guide_segments[offset] == 11.0) {
          const gp_Pnt center(
              guide_segments[offset + 7], guide_segments[offset + 8], guide_segments[offset + 9]);
          const gp_Vec normal(
              guide_segments[offset + 10], guide_segments[offset + 11], guide_segments[offset + 12]);
          const gp_Vec start_radius(center, start);
          const gp_Vec end_radius(center, end);
          if (normal.SquareMagnitude() <= 1.0e-18
              || start_radius.SquareMagnitude() <= 1.0e-18) {
            return error_result(STATUS_INVALID_PARAMETER, "OCCT guided Loft arc is degenerate");
          }
          const double signed_angle = std::atan2(
              normal.Dot(start_radius.Crossed(end_radius)), start_radius.Dot(end_radius));
          const bool clockwise = guide_segments[offset + 13] != 0.0;
          const double full_turn = 2.0 * std::acos(-1.0);
          double angle = std::fmod(clockwise ? -signed_angle : signed_angle, full_turn);
          if (angle <= 0.0) angle += full_turn;
          const gp_Vec middle_radius = start_radius.Rotated(
              gp_Ax1(center, gp_Dir(normal)), (clockwise ? -angle : angle) / 2.0);
          GC_MakeArcOfCircle arc_builder(start, center.Translated(middle_radius), end);
          if (arc_builder.IsDone()) {
            BRepBuilderAPI_MakeEdge builder(arc_builder.Value());
            if (builder.IsDone()) edge = builder.Edge();
          }
        } else {
          return error_result(STATUS_INVALID_PARAMETER, "OCCT guided Loft path kind is invalid");
        }
        if (edge.IsNull()) {
          return error_result(STATUS_INVALID_SHAPE, "OCCT guided Loft path edge is null");
        }
        spine_builder.Add(edge);
        previous_end = end;
        has_previous = true;
      }
      if (!spine_builder.IsDone() || !BRepCheck_Analyzer(spine_builder.Wire()).IsValid()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT guided Loft spine is invalid");
      }
      BRepOffsetAPI_MakePipeShell operation(spine_builder.Wire());
      operation.SetMode(false);
      operation.SetTolerance(1.0e-7, 1.0e-7, 1.0e-9);
      operation.SetForceApproxC1(continuity == 1);
      for (const TopoDS_Wire& wire : wires) operation.Add(wire, false, false);
      if (!operation.IsReady()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT guided Loft pipe is not ready");
      }
      operation.Build();
      if (!operation.IsDone() || (make_solid && !operation.MakeSolid())) {
        return error_result(
            STATUS_INVALID_SHAPE,
            make_solid
                ? "OCCT guided Loft pipe did not produce a solid"
                : "OCCT guided Loft pipe did not produce a surface");
      }
      const TopoDS_Shape result = operation.Shape();
      const std::uint32_t solid_count = count_subshapes(result, TopAbs_SOLID);
      if (result.IsNull() || !BRepCheck_Analyzer(result).IsValid()
          || (make_solid ? solid_count != 1 : solid_count != 0)) {
        return error_result(
            STATUS_INVALID_SHAPE,
            make_solid
                ? "OCCT guided Loft result is not one valid solid"
                : "OCCT guided Loft result is not a valid open surface");
      }
      std::vector<HistoryRecord> history;
      history.push_back(history_record(
          "loft.start", "first_shape", "profile.wire", result, operation.FirstShape()));
      history.push_back(history_record(
          "loft.end", "last_shape", "profile.wire", result, operation.LastShape()));
      return success_result(
          result, std::move(history), false, false, {}, !make_solid);
    }
    BRepOffsetAPI_ThruSections operation(make_solid, false, 1.0e-6);
    operation.CheckCompatibility(true);
    operation.SetMutableInput(false);
    operation.SetContinuity(
        continuity == 0 ? GeomAbs_C0 : (continuity == 1 ? GeomAbs_C1 : GeomAbs_C2));
    for (const TopoDS_Wire& wire : wires) operation.AddWire(wire);
    operation.Build();
    if (!operation.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT framed Loft builder did not complete");
    }
    const TopoDS_Shape result = operation.Shape();
    const std::uint32_t solid_count = count_subshapes(result, TopAbs_SOLID);
    if (result.IsNull() || !BRepCheck_Analyzer(result).IsValid()
        || (make_solid ? solid_count != 1 : solid_count != 0)) {
      return error_result(
          STATUS_INVALID_SHAPE,
          make_solid
              ? "OCCT framed Loft did not produce one valid solid"
              : "OCCT framed Loft did not produce a valid open surface");
    }
    std::vector<HistoryRecord> history;
    history.push_back(history_record(
        "loft.start", "first_shape", "profile.wire", result, operation.FirstShape()));
    history.push_back(history_record(
        "loft.end", "last_shape", "profile.wire", result, operation.LastShape()));
    history.push_back(history_record(
        "loft.side", "generated_face", "profile.edge", result,
        operation.GeneratedFace(section_edges.front())));
    return success_result(
        result, std::move(history), false, false, {}, !make_solid);
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

std::unique_ptr<NativeOperationResult> loft_planar_profiles_native(
    rust::Slice<const double> segments,
    rust::Slice<const std::uint32_t> section_segment_counts,
    rust::Slice<const double> elevations) noexcept {
  return guarded([&] {
    if (section_segment_counts.size() < 2 || section_segment_counts.size() > 16
        || section_segment_counts.size() != elevations.size()
        || segments.empty() || segments.size() % 10 != 0) {
      return error_result(STATUS_INVALID_PARAMETER, "OCCT planar Loft payload is malformed");
    }
    std::size_t declared_segments = 0;
    double previous_elevation = -std::numeric_limits<double>::infinity();
    for (std::size_t section = 0; section < elevations.size(); ++section) {
      const std::uint32_t count = section_segment_counts[section];
      const double elevation = elevations[section];
      if (count == 0 || count > 64 || !std::isfinite(elevation)
          || std::abs(elevation) > 1000000.0 || elevation <= previous_elevation) {
        return error_result(STATUS_INVALID_PARAMETER, "OCCT planar Loft section is invalid");
      }
      declared_segments += count;
      previous_elevation = elevation;
    }
    if (declared_segments != segments.size() / 10) {
      return error_result(STATUS_INVALID_PARAMETER, "OCCT planar Loft segment counts do not match");
    }

    std::vector<TopoDS_Wire> wires;
    std::vector<TopoDS_Edge> section_edges;
    wires.reserve(section_segment_counts.size());
    section_edges.reserve(section_segment_counts.size());
    std::size_t first_segment = 0;
    const double tau = 2.0 * std::acos(-1.0);
    for (std::size_t section = 0; section < section_segment_counts.size(); ++section) {
      const std::size_t segment_count = section_segment_counts[section];
      const double elevation = elevations[section];
      BRepBuilderAPI_MakeWire wire_builder;
      bool line_only = true;
      TopoDS_Edge first_edge;
      for (std::size_t index = 0; index < segment_count; ++index) {
        const std::size_t offset = (first_segment + index) * 10;
        for (std::size_t value = 0; value < 10; ++value) {
          if (!std::isfinite(segments[offset + value])
              || std::abs(segments[offset + value]) > 1000000.0) {
            return error_result(STATUS_INVALID_PARAMETER, "OCCT planar Loft segment value is invalid");
          }
        }
        const double kind = segments[offset];
        TopoDS_Edge edge;
        if (kind == 0.0) {
          const gp_Pnt start(segments[offset + 1], segments[offset + 2], elevation);
          const gp_Pnt end(segments[offset + 3], segments[offset + 4], elevation);
          if (segments[offset + 5] != 0.0 || segments[offset + 6] != 0.0
              || segments[offset + 7] != 0.0 || segments[offset + 8] != 0.0
              || segments[offset + 9] != 0.0
              || start.Distance(end) < 0.01 || start.Distance(end) > 100000.0) {
            return error_result(STATUS_INVALID_PARAMETER, "OCCT planar Loft line is invalid");
          }
          BRepBuilderAPI_MakeEdge edge_builder(start, end);
          if (edge_builder.IsDone()) edge = edge_builder.Edge();
        } else if (kind == 1.0) {
          line_only = false;
          const gp_Pnt start(segments[offset + 1], segments[offset + 2], elevation);
          const gp_Pnt end(segments[offset + 3], segments[offset + 4], elevation);
          const double center_x = segments[offset + 5];
          const double center_y = segments[offset + 6];
          const gp_Pnt center(center_x, center_y, elevation);
          const double radius = start.Distance(center);
          const double end_radius = end.Distance(center);
          if (segments[offset + 7] != 0.0 || segments[offset + 8] != 0.0
              || (segments[offset + 9] != 0.0 && segments[offset + 9] != 1.0)
              || radius < 0.01 || radius > 100000.0
              || std::abs(radius - end_radius) > 1.0e-9 * std::max({radius, end_radius, 1.0})) {
            return error_result(STATUS_INVALID_PARAMETER, "OCCT planar Loft arc is invalid");
          }
          const double start_angle = std::atan2(start.Y() - center_y, start.X() - center_x);
          const double end_angle = std::atan2(end.Y() - center_y, end.X() - center_x);
          double sweep = end_angle - start_angle;
          if (segments[offset + 9] != 0.0) {
            while (sweep >= 0.0) sweep -= tau;
          } else {
            while (sweep <= 0.0) sweep += tau;
          }
          const double middle_angle = start_angle + sweep / 2.0;
          const gp_Pnt middle(
              center_x + radius * std::cos(middle_angle),
              center_y + radius * std::sin(middle_angle), elevation);
          GC_MakeArcOfCircle arc_builder(start, middle, end);
          if (arc_builder.IsDone()) {
            BRepBuilderAPI_MakeEdge edge_builder(arc_builder.Value());
            if (edge_builder.IsDone()) edge = edge_builder.Edge();
          }
        } else if (kind == 2.0) {
          line_only = false;
          if (segments[offset + 9] != 0.0) {
            return error_result(STATUS_INVALID_PARAMETER, "OCCT planar Loft cubic is invalid");
          }
          edge = cubic_bezier_edge(segments, offset, elevation);
        } else if (kind == 3.0 && segment_count == 1) {
          line_only = false;
          const double center_x = segments[offset + 1];
          const double center_y = segments[offset + 2];
          const double radius = segments[offset + 3];
          if (radius < 0.01 || radius > 100000.0
              || segments[offset + 4] != 0.0 || segments[offset + 5] != 0.0
              || segments[offset + 6] != 0.0 || segments[offset + 7] != 0.0
              || segments[offset + 8] != 0.0 || segments[offset + 9] != 0.0) {
            return error_result(STATUS_INVALID_PARAMETER, "OCCT planar Loft circle is invalid");
          }
          BRepBuilderAPI_MakeEdge edge_builder(gp_Circ(
              gp_Ax2(gp_Pnt(center_x, center_y, elevation), gp_Dir(0.0, 0.0, 1.0)), radius));
          if (edge_builder.IsDone()) edge = edge_builder.Edge();
        } else {
          return error_result(STATUS_INVALID_PARAMETER, "OCCT planar Loft segment kind is invalid");
        }
        if (edge.IsNull()) {
          return error_result(STATUS_INVALID_SHAPE, "OCCT planar Loft edge is null");
        }
        if (kind != 3.0) {
          const std::size_t next = (first_segment + (index + 1) % segment_count) * 10;
          if (segments[offset + 3] != segments[next + 1]
              || segments[offset + 4] != segments[next + 2]) {
            return error_result(STATUS_INVALID_PARAMETER, "OCCT planar Loft section is open");
          }
        }
        if (first_edge.IsNull()) first_edge = edge;
        wire_builder.Add(edge);
      }
      if (!wire_builder.IsDone() || (line_only && segment_count < 3)
          || !BRepCheck_Analyzer(wire_builder.Wire()).IsValid()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT planar Loft wire is invalid");
      }
      wires.push_back(wire_builder.Wire());
      section_edges.push_back(first_edge);
      first_segment += segment_count;
    }

    BRepOffsetAPI_ThruSections operation(true, false, 1.0e-6);
    operation.CheckCompatibility(true);
    operation.SetMutableInput(false);
    for (const TopoDS_Wire& wire : wires) operation.AddWire(wire);
    operation.Build();
    if (!operation.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT planar Loft builder did not complete");
    }
    const TopoDS_Shape result = operation.Shape();
    if (result.IsNull() || !BRepCheck_Analyzer(result).IsValid()
        || count_subshapes(result, TopAbs_SOLID) != 1) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT planar Loft did not produce one valid solid");
    }
    std::vector<HistoryRecord> history;
    history.push_back(history_record(
        "loft.start", "first_shape", "profile.wire", result, operation.FirstShape()));
    history.push_back(history_record(
        "loft.end", "last_shape", "profile.wire", result, operation.LastShape()));
    history.push_back(history_record(
        "loft.side", "generated_face", "profile.edge", result,
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

TopoDS_Edge cubic_bezier_edge(
    rust::Slice<const double> segments, std::size_t offset, double z) {
  TColgp_Array1OfPnt poles(1, 4);
  poles.SetValue(1, gp_Pnt(segments[offset + 1], segments[offset + 2], z));
  poles.SetValue(2, gp_Pnt(segments[offset + 5], segments[offset + 6], z));
  poles.SetValue(3, gp_Pnt(segments[offset + 7], segments[offset + 8], z));
  poles.SetValue(4, gp_Pnt(segments[offset + 3], segments[offset + 4], z));
  occ::handle<Geom_BezierCurve> curve = new Geom_BezierCurve(poles);
  BRepBuilderAPI_MakeEdge edge_builder(curve);
  return edge_builder.IsDone() ? edge_builder.Edge() : TopoDS_Edge{};
}

std::unique_ptr<NativeOperationResult> sweep_axial_tool_native(
    rust::Slice<const double> values) noexcept {
  return guarded([&] {
    if (values.size() != 13
        || !std::all_of(values.begin(), values.end(), [](double value) { return std::isfinite(value); })
        || (values[0] != 0.0 && values[0] != 1.0)
        || values[11] <= 0.0 || values[12] <= 0.0) {
      return error_result(STATUS_INVALID_PARAMETER, "Axial tool sweep payload is malformed");
    }
    const gp_Pnt start(values[1], values[2], values[3]);
    const gp_Pnt end(values[4], values[5], values[6]);
    const double radius = values[11];
    const double length = values[12];
    const auto cylinder = [&](const gp_Pnt& point) {
      BRepPrimAPI_MakeCylinder builder(
          gp_Ax2(point, gp_Dir(0.0, 0.0, 1.0)), radius, length);
      builder.Build();
      return builder.IsDone() ? builder.Shape() : TopoDS_Shape{};
    };
    const auto fuse = [](const TopoDS_Shape& left, const TopoDS_Shape& right) {
      if (left.IsNull() || right.IsNull()) return TopoDS_Shape{};
      BRepAlgoAPI_Fuse operation(left, right);
      operation.Build();
      if (!operation.IsDone() || operation.HasErrors()) return TopoDS_Shape{};
      operation.SimplifyResult(true, true);
      return operation.Shape();
    };

    TopoDS_Shape result;
    if (values[0] == 0.0) {
      const double dx = end.X() - start.X();
      const double dy = end.Y() - start.Y();
      const double dz = end.Z() - start.Z();
      const double planar_length = std::hypot(dx, dy);
      if (planar_length <= 1.0e-9) {
        const gp_Pnt bottom(start.X(), start.Y(), std::min(start.Z(), end.Z()));
        result = cylinder(bottom);
        if (std::abs(dz) > 1.0e-9) {
          BRepPrimAPI_MakeCylinder builder(
              gp_Ax2(bottom, gp_Dir(0.0, 0.0, 1.0)),
              radius,
              length + std::abs(dz));
          builder.Build();
          result = builder.IsDone() ? builder.Shape() : TopoDS_Shape{};
        }
      } else {
        if (std::abs(dz) > 1.0e-9) {
          return error_result(
              STATUS_INVALID_PARAMETER,
              "Axial tool line sweep must be horizontal or vertical");
        }
        const gp_Dir along(dx, dy, 0.0);
        const gp_Dir perpendicular(-dy, dx, 0.0);
        BRepPrimAPI_MakeBox bridge(
            gp_Ax2(
                gp_Pnt(
                    start.X() - radius * perpendicular.X(),
                    start.Y() - radius * perpendicular.Y(),
                    start.Z()),
                gp_Dir(0.0, 0.0, 1.0),
                along),
            planar_length,
            2.0 * radius,
            length);
        bridge.Build();
        if (!bridge.IsDone()) {
          return error_result(STATUS_INVALID_SHAPE, "OCCT axial line sweep bridge failed");
        }
        result = fuse(fuse(bridge.Shape(), cylinder(start)), cylinder(end));
      }
    } else {
      const gp_Pnt center(values[7], values[8], values[9]);
      if (std::abs(start.Z() - end.Z()) > 1.0e-9
          || std::abs(start.Z() - center.Z()) > 1.0e-9
          || (values[10] != 0.0 && values[10] != 1.0)) {
        return error_result(STATUS_INVALID_PARAMETER, "Axial tool arc sweep is malformed");
      }
      const double centerline_radius = std::hypot(start.X() - center.X(), start.Y() - center.Y());
      const double end_radius = std::hypot(end.X() - center.X(), end.Y() - center.Y());
      if (centerline_radius <= 1.0e-9
          || std::abs(centerline_radius - end_radius) > 1.0e-7
          || start.Distance(end) <= 1.0e-9) {
        return error_result(STATUS_DEGENERATE_OPERATION, "Axial tool arc sweep is degenerate");
      }
      const bool clockwise = values[10] != 0.0;
      const double tau = 2.0 * std::acos(-1.0);
      const double start_angle = std::atan2(start.Y() - center.Y(), start.X() - center.X());
      const double end_angle = std::atan2(end.Y() - center.Y(), end.X() - center.X());
      double sweep = end_angle - start_angle;
      if (clockwise) {
        while (sweep >= 0.0) sweep -= tau;
      } else {
        while (sweep <= 0.0) sweep += tau;
      }
      const auto point = [&](double radial, double angle) {
        return gp_Pnt(
            center.X() + radial * std::cos(angle),
            center.Y() + radial * std::sin(angle),
            start.Z());
      };
      const auto arc = [&](double radial, double angle, double arc_sweep) {
        GC_MakeArcOfCircle builder(
            point(radial, angle),
            point(radial, angle + arc_sweep * 0.5),
            point(radial, angle + arc_sweep));
        if (!builder.IsDone()) return TopoDS_Edge{};
        BRepBuilderAPI_MakeEdge edge(builder.Value());
        return edge.IsDone() ? edge.Edge() : TopoDS_Edge{};
      };
      const double outer_radius = centerline_radius + radius;
      const double inner_radius = centerline_radius - radius;
      const TopoDS_Edge outer = arc(outer_radius, start_angle, sweep);
      BRepBuilderAPI_MakeWire wire;
      wire.Add(outer);
      if (inner_radius > 1.0e-9) {
        const TopoDS_Edge end_cap = BRepBuilderAPI_MakeEdge(
            point(outer_radius, start_angle + sweep),
            point(inner_radius, start_angle + sweep)).Edge();
        const TopoDS_Edge inner = arc(inner_radius, start_angle + sweep, -sweep);
        const TopoDS_Edge start_cap = BRepBuilderAPI_MakeEdge(
            point(inner_radius, start_angle),
            point(outer_radius, start_angle)).Edge();
        if (outer.IsNull() || end_cap.IsNull() || inner.IsNull() || start_cap.IsNull()) {
          return error_result(STATUS_INVALID_SHAPE, "OCCT axial arc sweep boundary failed");
        }
        wire.Add(end_cap);
        wire.Add(inner);
        wire.Add(start_cap);
      } else {
        const gp_Pnt arc_center(center.X(), center.Y(), start.Z());
        const TopoDS_Edge end_cap = BRepBuilderAPI_MakeEdge(
            point(outer_radius, start_angle + sweep), arc_center).Edge();
        const TopoDS_Edge start_cap = BRepBuilderAPI_MakeEdge(
            arc_center, point(outer_radius, start_angle)).Edge();
        if (outer.IsNull() || end_cap.IsNull() || start_cap.IsNull()) {
          return error_result(STATUS_INVALID_SHAPE, "OCCT axial broad arc sweep boundary failed");
        }
        wire.Add(end_cap);
        wire.Add(start_cap);
      }
      if (!wire.IsDone()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT axial arc sweep wire failed");
      }
      BRepBuilderAPI_MakeFace face(wire.Wire());
      if (!face.IsDone()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT axial arc sweep face failed");
      }
      BRepPrimAPI_MakePrism prism(face.Face(), gp_Vec(0.0, 0.0, length));
      prism.Build();
      if (!prism.IsDone()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT axial arc sweep prism failed");
      }
      result = fuse(fuse(prism.Shape(), cylinder(start)), cylinder(end));
    }
    if (result.IsNull() || !BRepCheck_Analyzer(result, true).IsValid()
        || count_subshapes(result, TopAbs_SOLID) != 1) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT axial tool sweep is not one valid solid");
    }
    return success_result(result, {});
  });
}

std::unique_ptr<NativeOperationResult> extrude_mixed_profile_native(
    rust::Slice<const double> segments, double height) noexcept {
  return guarded([&] {
    if (segments.size() < 16 || segments.size() % 10 != 0) {
      return error_result(STATUS_INVALID_PARAMETER, "Mixed profile segment payload is malformed");
    }
    BRepBuilderAPI_MakeWire wire_builder;
    std::vector<TopoDS_Edge> profile_edges;
    profile_edges.reserve(segments.size() / 10);
    std::size_t first_arc_index = profile_edges.capacity();
    std::size_t first_line_index = profile_edges.capacity();
    std::size_t first_cubic_index = profile_edges.capacity();
    for (std::size_t offset = 0; offset < segments.size(); offset += 10) {
      const double kind = segments[offset];
      const gp_Pnt start(segments[offset + 1], segments[offset + 2], 0.0);
      const gp_Pnt end(segments[offset + 3], segments[offset + 4], 0.0);
      TopoDS_Edge edge;
      if (kind == 0.0) {
        edge = BRepBuilderAPI_MakeEdge(start, end).Edge();
        if (first_line_index == profile_edges.capacity()) {
          first_line_index = profile_edges.size();
        }
      } else if (kind == 1.0) {
        const double center_x = segments[offset + 5];
        const double center_y = segments[offset + 6];
        const bool clockwise = segments[offset + 9] != 0.0;
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
      } else if (kind == 2.0) {
        edge = cubic_bezier_edge(segments, offset, 0.0);
        if (first_cubic_index == profile_edges.capacity()) {
          first_cubic_index = profile_edges.size();
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
    if (!wire_builder.IsDone()
        || (first_arc_index >= profile_edges.size()
            && first_line_index >= profile_edges.size()
            && first_cubic_index >= profile_edges.size())) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT segmented profile wire is incomplete");
    }
    BRepBuilderAPI_MakeFace face_builder(wire_builder.Wire(), true);
    if (!face_builder.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT mixed profile face builder did not complete");
    }
    const TopoDS_Face profile = face_builder.Face();
    const bool reference_is_arc = first_arc_index < profile_edges.size();
    const bool reference_is_line = first_line_index < profile_edges.size();
    const std::size_t reference_index = reference_is_arc
        ? first_arc_index
        : (reference_is_line ? first_line_index : first_cubic_index);
    TopoDS_Edge profile_reference;
    if (!reference_is_arc && !reference_is_line) {
      profile_reference = profile_edges[reference_index];
    } else {
      const std::size_t reference_offset = reference_index * 10;
      const gp_Pnt expected_reference_start(
          segments[reference_offset + 1], segments[reference_offset + 2], 0.0);
      const gp_Pnt expected_reference_end(
          segments[reference_offset + 3], segments[reference_offset + 4], 0.0);
      for (TopExp_Explorer explorer(profile, TopAbs_EDGE); explorer.More(); explorer.Next()) {
        const TopoDS_Edge candidate = TopoDS::Edge(explorer.Current());
        const GeomAbs_CurveType expected_type = reference_is_arc ? GeomAbs_Circle : GeomAbs_Line;
        if (BRepAdaptor_Curve(candidate).GetType() != expected_type) {
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
            (first_point.Distance(expected_reference_start) <= 1.0e-9
                && last_point.Distance(expected_reference_end) <= 1.0e-9)
            || (first_point.Distance(expected_reference_end) <= 1.0e-9
                && last_point.Distance(expected_reference_start) <= 1.0e-9);
        if (endpoints_match) {
          if (!profile_reference.IsNull()) {
            return error_result(STATUS_INVALID_SHAPE, "OCCT segmented profile reference edge is ambiguous");
          }
          profile_reference = candidate;
        }
      }
    }
    if (profile_reference.IsNull()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT segmented profile lost its reference edge");
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
    const std::string side_role = reference_is_arc
        ? "extrusion.side(profile_edge=arc.0)"
        : (reference_is_line
            ? "extrusion.side(profile_edge=line.0)"
            : "extrusion.side(profile_edge=spline.0)");
    const std::string side_source = reference_is_arc
        ? "profile.edge.arc.0"
        : (reference_is_line ? "profile.edge.line.0" : "profile.edge.spline.0");
    HistoryRecord side_history{side_role, "generated", side_source, 0, false};
    const NCollection_List<TopoDS_Shape>& generated = operation.Generated(profile_reference);
    for (NCollection_List<TopoDS_Shape>::Iterator iterator(generated); iterator.More(); iterator.Next()) {
      const HistoryRecord candidate = history_record(
          side_role,
          "generated",
          side_source,
          result,
          iterator.Value());
      if (candidate.output_present) {
        side_history = candidate;
        break;
      }
    }
    history.push_back(std::move(side_history));
    return success_result(result, std::move(history));
  });
}

std::unique_ptr<NativeOperationResult> extrude_planar_region_native(
    rust::Slice<const double> segments,
    rust::Slice<const std::uint32_t> loop_segment_counts,
    double height) noexcept {
  return guarded([&] {
    if (loop_segment_counts.size() < 2 || loop_segment_counts.size() > 65
        || segments.empty() || segments.size() % 10 != 0
        || !std::isfinite(height) || height <= 0.0) {
      return error_result(STATUS_INVALID_PARAMETER, "Planar region payload is malformed");
    }
    std::size_t declared_segments = 0;
    for (const std::uint32_t count : loop_segment_counts) {
      declared_segments += count;
    }
    if (declared_segments != segments.size() / 10 || declared_segments > 4096) {
      return error_result(STATUS_INVALID_PARAMETER, "Planar region segment counts do not match");
    }

    auto build_wire = [&](std::size_t first_segment, std::size_t segment_count) {
      BRepBuilderAPI_MakeWire wire_builder;
      for (std::size_t index = 0; index < segment_count; ++index) {
        const std::size_t offset = (first_segment + index) * 10;
        const double kind = segments[offset];
        TopoDS_Edge edge;
        if (kind == 0.0) {
          const gp_Pnt start(segments[offset + 1], segments[offset + 2], 0.0);
          const gp_Pnt end(segments[offset + 3], segments[offset + 4], 0.0);
          if (start.Distance(end) <= Precision::Confusion()) {
            return TopoDS_Wire{};
          }
          BRepBuilderAPI_MakeEdge edge_builder(start, end);
          if (!edge_builder.IsDone()) {
            return TopoDS_Wire{};
          }
          edge = edge_builder.Edge();
        } else if (kind == 1.0) {
          const gp_Pnt start(segments[offset + 1], segments[offset + 2], 0.0);
          const gp_Pnt end(segments[offset + 3], segments[offset + 4], 0.0);
          const double center_x = segments[offset + 5];
          const double center_y = segments[offset + 6];
          const bool clockwise = segments[offset + 9] != 0.0;
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
            return TopoDS_Wire{};
          }
          BRepBuilderAPI_MakeEdge edge_builder(arc_builder.Value());
          if (!edge_builder.IsDone()) {
            return TopoDS_Wire{};
          }
          edge = edge_builder.Edge();
        } else if (kind == 2.0) {
          edge = cubic_bezier_edge(segments, offset, 0.0);
        } else if (kind == 3.0 && segment_count == 1) {
          const double center_x = segments[offset + 1];
          const double center_y = segments[offset + 2];
          const double radius = segments[offset + 3];
          if (!std::isfinite(center_x) || !std::isfinite(center_y)
              || !std::isfinite(radius) || radius <= Precision::Confusion()) {
            return TopoDS_Wire{};
          }
          BRepBuilderAPI_MakeEdge edge_builder(
              gp_Circ(gp_Ax2(gp_Pnt(center_x, center_y, 0.0), gp_Dir(0.0, 0.0, 1.0)), radius));
          if (!edge_builder.IsDone()) {
            return TopoDS_Wire{};
          }
          edge = edge_builder.Edge();
          if (segments[offset + 9] != 0.0) {
            edge.Reverse();
          }
        } else {
          return TopoDS_Wire{};
        }
        if (edge.IsNull()) {
          return TopoDS_Wire{};
        }
        wire_builder.Add(edge);
      }
      if (!wire_builder.IsDone()) {
        return TopoDS_Wire{};
      }
      return wire_builder.Wire();
    };

    std::size_t first_segment = 0;
    TopoDS_Wire outer = build_wire(first_segment, loop_segment_counts[0]);
    if (outer.IsNull()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT planar region outer wire is invalid");
    }
    first_segment += loop_segment_counts[0];
    BRepBuilderAPI_MakeFace face_builder(outer, true);
    for (std::size_t index = 1; index < loop_segment_counts.size(); ++index) {
      TopoDS_Wire hole = build_wire(first_segment, loop_segment_counts[index]);
      if (hole.IsNull()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT planar region hole wire is invalid");
      }
      face_builder.Add(hole);
      first_segment += loop_segment_counts[index];
    }
    face_builder.Build();
    if (!face_builder.IsDone() || !BRepCheck_Analyzer(face_builder.Face()).IsValid()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT planar region face is invalid");
    }
    const TopoDS_Face profile = face_builder.Face();
    BRepPrimAPI_MakePrism operation(profile, gp_Vec(0.0, 0.0, height), true, false);
    if (!operation.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT planar region prism builder did not complete");
    }
    const TopoDS_Shape result = operation.Shape();
    if (result.IsNull() || !BRepCheck_Analyzer(result).IsValid()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT planar region prism is invalid");
    }
    std::vector<HistoryRecord> history;
    history.push_back(history_record(
        "extrusion.bottom", "first_shape", "profile.face", result, operation.FirstShape()));
    history.push_back(history_record(
        "extrusion.top", "last_shape", "profile.face", result, operation.LastShape()));
    return success_result(result, std::move(history));
  });
}

std::unique_ptr<NativeOperationResult> revolve_planar_region_native(
    rust::Slice<const double> segments,
    rust::Slice<const std::uint32_t> loop_segment_counts,
    double axis_start_x, double axis_start_y,
    double axis_end_x, double axis_end_y,
    double angle_degrees) noexcept {
  return guarded([&] {
    if (loop_segment_counts.size() < 2 || loop_segment_counts.size() > 65
        || segments.empty() || segments.size() % 10 != 0
        || !std::isfinite(axis_start_x) || !std::isfinite(axis_start_y)
        || !std::isfinite(axis_end_x) || !std::isfinite(axis_end_y)
        || !std::isfinite(angle_degrees) || angle_degrees <= 0.0 || angle_degrees > 360.0) {
      return error_result(STATUS_INVALID_PARAMETER, "Planar region revolve payload is malformed");
    }
    std::size_t declared_segments = 0;
    for (const std::uint32_t count : loop_segment_counts) {
      declared_segments += count;
    }
    if (declared_segments != segments.size() / 10 || declared_segments > 4096) {
      return error_result(STATUS_INVALID_PARAMETER, "Planar region revolve segment counts do not match");
    }
    const gp_Vec axis_vector(
        axis_end_x - axis_start_x,
        axis_end_y - axis_start_y,
        0.0);
    if (axis_vector.Magnitude() <= 1.0e-12) {
      return error_result(STATUS_INVALID_PARAMETER, "Planar region revolve axis is degenerate");
    }

    auto build_wire = [&](std::size_t first_segment, std::size_t segment_count) {
      BRepBuilderAPI_MakeWire wire_builder;
      for (std::size_t index = 0; index < segment_count; ++index) {
        const std::size_t offset = (first_segment + index) * 10;
        const double kind = segments[offset];
        TopoDS_Edge edge;
        if (kind == 0.0) {
          const gp_Pnt start(segments[offset + 1], segments[offset + 2], 0.0);
          const gp_Pnt end(segments[offset + 3], segments[offset + 4], 0.0);
          if (start.Distance(end) <= Precision::Confusion()) {
            return TopoDS_Wire{};
          }
          BRepBuilderAPI_MakeEdge edge_builder(start, end);
          if (!edge_builder.IsDone()) {
            return TopoDS_Wire{};
          }
          edge = edge_builder.Edge();
        } else if (kind == 1.0) {
          const gp_Pnt start(segments[offset + 1], segments[offset + 2], 0.0);
          const gp_Pnt end(segments[offset + 3], segments[offset + 4], 0.0);
          const double center_x = segments[offset + 5];
          const double center_y = segments[offset + 6];
          const bool clockwise = segments[offset + 9] != 0.0;
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
          const gp_Pnt middle(
              center_x + radius * std::cos(start_angle + sweep / 2.0),
              center_y + radius * std::sin(start_angle + sweep / 2.0),
              0.0);
          GC_MakeArcOfCircle arc_builder(start, middle, end);
          if (!arc_builder.IsDone()) {
            return TopoDS_Wire{};
          }
          BRepBuilderAPI_MakeEdge edge_builder(arc_builder.Value());
          if (!edge_builder.IsDone()) {
            return TopoDS_Wire{};
          }
          edge = edge_builder.Edge();
        } else if (kind == 2.0) {
          edge = cubic_bezier_edge(segments, offset, 0.0);
        } else if (kind == 3.0 && segment_count == 1) {
          const double center_x = segments[offset + 1];
          const double center_y = segments[offset + 2];
          const double radius = segments[offset + 3];
          if (!std::isfinite(center_x) || !std::isfinite(center_y)
              || !std::isfinite(radius) || radius <= Precision::Confusion()) {
            return TopoDS_Wire{};
          }
          BRepBuilderAPI_MakeEdge edge_builder(
              gp_Circ(gp_Ax2(gp_Pnt(center_x, center_y, 0.0), gp_Dir(0.0, 0.0, 1.0)), radius));
          if (!edge_builder.IsDone()) {
            return TopoDS_Wire{};
          }
          edge = edge_builder.Edge();
          if (segments[offset + 9] != 0.0) {
            edge.Reverse();
          }
        } else {
          return TopoDS_Wire{};
        }
        if (edge.IsNull()) {
          return TopoDS_Wire{};
        }
        wire_builder.Add(edge);
      }
      if (!wire_builder.IsDone()) {
        return TopoDS_Wire{};
      }
      return wire_builder.Wire();
    };

    std::size_t first_segment = 0;
    TopoDS_Wire outer = build_wire(first_segment, loop_segment_counts[0]);
    if (outer.IsNull()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT planar region revolve outer wire is invalid");
    }
    first_segment += loop_segment_counts[0];
    BRepBuilderAPI_MakeFace face_builder(outer, true);
    for (std::size_t index = 1; index < loop_segment_counts.size(); ++index) {
      TopoDS_Wire hole = build_wire(first_segment, loop_segment_counts[index]);
      if (hole.IsNull()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT planar region revolve hole wire is invalid");
      }
      face_builder.Add(hole);
      first_segment += loop_segment_counts[index];
    }
    face_builder.Build();
    if (!face_builder.IsDone() || !BRepCheck_Analyzer(face_builder.Face()).IsValid()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT planar region revolve face is invalid");
    }
    const TopoDS_Face profile = face_builder.Face();
    const double angle_radians = angle_degrees * std::acos(-1.0) / 180.0;
    BRepPrimAPI_MakeRevol operation(
        profile,
        gp_Ax1(gp_Pnt(axis_start_x, axis_start_y, 0.0), gp_Dir(axis_vector)),
        angle_radians,
        true);
    if (!operation.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT planar region revolve builder did not complete");
    }
    const TopoDS_Shape result = operation.Shape();
    if (result.IsNull() || !BRepCheck_Analyzer(result).IsValid()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT planar region revolve is invalid");
    }
    std::vector<HistoryRecord> history;
    if (angle_degrees < 360.0) {
      history.push_back(history_record(
          "revolve.start", "first_shape", "profile.face", result, operation.FirstShape()));
      history.push_back(history_record(
          "revolve.end", "last_shape", "profile.face", result, operation.LastShape()));
    }
    return success_result(result, std::move(history));
  });
}

std::unique_ptr<NativeOperationResult> revolve_general_profile_native(
    rust::Slice<const double> segments,
    double axis_start_x, double axis_start_y,
    double axis_end_x, double axis_end_y,
    double angle_degrees) noexcept {
  return guarded([&] {
    if (segments.size() < 16 || segments.size() % 10 != 0
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
    profile_edges.reserve(segments.size() / 10);
    for (std::size_t offset = 0; offset < segments.size(); offset += 10) {
      const double kind = segments[offset];
      const gp_Pnt start(segments[offset + 1], segments[offset + 2], 0.0);
      const gp_Pnt end(segments[offset + 3], segments[offset + 4], 0.0);
      TopoDS_Edge edge;
      if (kind == 0.0) {
        edge = BRepBuilderAPI_MakeEdge(start, end).Edge();
      } else if (kind == 1.0) {
        const double center_x = segments[offset + 5];
        const double center_y = segments[offset + 6];
        const bool clockwise = segments[offset + 9] != 0.0;
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
      const std::size_t offset = source_index * 10;
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
        const bool clockwise = segments[offset + 9] != 0.0;
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
          1.0e-6,
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
            1.0e-6,
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
        || !std::isfinite(distance) || std::abs(distance) < 1.0e-9) {
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
    append_propagated_history(history, operation, result, base.impl());
    append_cut_history(history, operation, result, base.impl().shape, "base.face.");
    append_cut_history(history, operation, result, tool, "tool.face.");
    return success_result(result, std::move(history));
  });
}

std::unique_ptr<NativeOperationResult> cut_mixed_profile_native(
    const NativeOperationResult& base, rust::Slice<const double> segments,
    double origin_z, double height) noexcept {
  return guarded([&] {
    if (!base.valid() || segments.size() < 16 || segments.size() % 10 != 0) {
      return error_result(STATUS_INVALID_PARAMETER, "Mixed cut payload is malformed");
    }
    BRepBuilderAPI_MakeWire wire_builder;
    std::vector<TopoDS_Edge> profile_edges;
    profile_edges.reserve(segments.size() / 10);
    std::size_t first_line_index = profile_edges.capacity();
    std::size_t first_arc_index = profile_edges.capacity();
    std::size_t line_count = 0;
    std::size_t arc_count = 0;
    for (std::size_t offset = 0; offset < segments.size(); offset += 10) {
      const double kind = segments[offset];
      const gp_Pnt start(segments[offset + 1], segments[offset + 2], origin_z);
      const gp_Pnt end(segments[offset + 3], segments[offset + 4], origin_z);
      TopoDS_Edge edge;
      if (kind == 0.0) {
        edge = BRepBuilderAPI_MakeEdge(start, end).Edge();
        if (first_line_index == profile_edges.capacity()) {
          first_line_index = profile_edges.size();
        }
        ++line_count;
      } else if (kind == 1.0) {
        const double center_x = segments[offset + 5];
        const double center_y = segments[offset + 6];
        const bool clockwise = segments[offset + 9] != 0.0;
        const double start_angle = std::atan2(start.Y() - center_y, start.X() - center_x);
        const double end_angle = std::atan2(end.Y() - center_y, end.X() - center_x);
        double sweep = end_angle - start_angle;
        const double tau = 2.0 * std::acos(-1.0);
        if (clockwise) {
          while (sweep >= 0.0) sweep -= tau;
        } else {
          while (sweep <= 0.0) sweep += tau;
        }
        const double radius = start.Distance(gp_Pnt(center_x, center_y, origin_z));
        const double middle_angle = start_angle + sweep / 2.0;
        const gp_Pnt middle(
            center_x + radius * std::cos(middle_angle),
            center_y + radius * std::sin(middle_angle),
            origin_z);
        GC_MakeArcOfCircle arc_builder(start, middle, end);
        if (!arc_builder.IsDone()) {
          return error_result(STATUS_INVALID_SHAPE, "OCCT mixed cut arc builder did not complete");
        }
        edge = BRepBuilderAPI_MakeEdge(arc_builder.Value()).Edge();
        if (first_arc_index == profile_edges.capacity()) {
          first_arc_index = profile_edges.size();
        }
        ++arc_count;
      } else {
        return error_result(STATUS_INVALID_PARAMETER, "Mixed cut segment kind is invalid");
      }
      if (edge.IsNull()) {
        return error_result(STATUS_INVALID_SHAPE, "Mixed cut edge is null");
      }
      profile_edges.push_back(edge);
      wire_builder.Add(edge);
    }
    if (arc_count == 0 && line_count < 3) {
      return error_result(STATUS_INVALID_PARAMETER, "Mixed cut profile shape is unsupported");
    }
    if (!wire_builder.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "Polygon cut wire did not complete");
    }
    BRepBuilderAPI_MakeFace face_builder(wire_builder.Wire(), true);
    if (!face_builder.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "Mixed cut face did not complete");
    }
    const TopoDS_Face profile = face_builder.Face();
    TopoDS_Edge profile_reference;
    const std::size_t reference_offset = first_line_index * 10;
    const gp_Pnt expected_reference_start(
        segments[reference_offset + 1], segments[reference_offset + 2], origin_z);
    const gp_Pnt expected_reference_end(
        segments[reference_offset + 3], segments[reference_offset + 4], origin_z);
    for (TopExp_Explorer explorer(profile, TopAbs_EDGE); explorer.More(); explorer.Next()) {
      const TopoDS_Edge candidate = TopoDS::Edge(explorer.Current());
      if (BRepAdaptor_Curve(candidate).GetType() != GeomAbs_Line) {
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
          (first_point.Distance(expected_reference_start) <= 1.0e-9
              && last_point.Distance(expected_reference_end) <= 1.0e-9)
          || (first_point.Distance(expected_reference_end) <= 1.0e-9
              && last_point.Distance(expected_reference_start) <= 1.0e-9);
      if (endpoints_match) {
        if (!profile_reference.IsNull()) {
          return error_result(STATUS_INVALID_SHAPE, "OCCT mixed cut reference edge is ambiguous");
        }
        profile_reference = candidate;
      }
    }
    if (profile_reference.IsNull()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT mixed cut lost its reference edge");
    }
    TopoDS_Edge profile_arc_reference;
    if (first_arc_index != profile_edges.capacity()) {
      const std::size_t arc_offset = first_arc_index * 10;
      const gp_Pnt expected_arc_start(
          segments[arc_offset + 1], segments[arc_offset + 2], origin_z);
      const gp_Pnt expected_arc_end(
          segments[arc_offset + 3], segments[arc_offset + 4], origin_z);
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
          if (!profile_arc_reference.IsNull()) {
            return error_result(STATUS_INVALID_SHAPE, "OCCT mixed cut arc reference edge is ambiguous");
          }
          profile_arc_reference = candidate;
        }
      }
      if (profile_arc_reference.IsNull()) {
        return error_result(STATUS_INVALID_SHAPE, "OCCT mixed cut lost its arc reference edge");
      }
    }
    BRepPrimAPI_MakePrism prism(profile, gp_Vec(0.0, 0.0, height), true, false);
    if (!prism.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "Polygon cut prism did not complete");
    }
    const TopoDS_Shape tool = prism.Shape();
    const NCollection_List<TopoDS_Shape>& generated_side = prism.Generated(profile_reference);
    if (generated_side.IsEmpty()) {
      return error_result(STATUS_INVALID_SHAPE, "Polygon cut reference wall is missing");
    }
    const TopoDS_Shape reference_side = generated_side.First();
    BRepAlgoAPI_Cut operation(base.impl().shape, tool);
    operation.Build();
    if (!operation.IsDone() || operation.HasErrors() || operation.Shape().IsNull()) {
      return error_result(STATUS_INVALID_SHAPE, "Polygon cut operation did not complete");
    }
    const TopoDS_Shape result = operation.Shape();
    GProp_GProps before_properties;
    GProp_GProps after_properties;
    BRepGProp::VolumeProperties(base.impl().shape, before_properties);
    BRepGProp::VolumeProperties(result, after_properties);
    const double scale = std::max(1.0, std::abs(before_properties.Mass()));
    if (std::abs(before_properties.Mass() - after_properties.Mass()) <=
        scale * 16.0 * std::numeric_limits<double>::epsilon()) {
      return error_result(STATUS_NO_GEOMETRIC_CHANGE, "Polygon cut produced no measurable geometric change");
    }
    std::vector<HistoryRecord> history;
    append_propagated_history(history, operation, result, base.impl());
    HistoryRecord wall{"through_cut.wall.line.0", "modified", "cut_profile.edge.line.0", 0, false};
    const NCollection_List<TopoDS_Shape>& modified = operation.Modified(reference_side);
    for (NCollection_List<TopoDS_Shape>::Iterator iterator(modified); iterator.More(); iterator.Next()) {
      const HistoryRecord candidate = history_record(
          "through_cut.wall.line.0", "modified", "cut_profile.edge.line.0", result, iterator.Value());
      if (candidate.output_present) {
        wall = candidate;
        break;
      }
    }
    history.push_back(std::move(wall));
    if (!profile_arc_reference.IsNull()) {
      const NCollection_List<TopoDS_Shape>& generated_arc_side = prism.Generated(profile_arc_reference);
      if (!generated_arc_side.IsEmpty()) {
        const TopoDS_Shape arc_side = generated_arc_side.First();
        const HistoryRecord retained = history_record(
            "through_cut.wall.arc.0", "retained", "cut_profile.edge.arc.0",
            result, arc_side);
        if (retained.output_present) {
          history.push_back(retained);
        } else {
          const NCollection_List<TopoDS_Shape>& modified_arc_side = operation.Modified(arc_side);
          for (NCollection_List<TopoDS_Shape>::Iterator iterator(modified_arc_side);
               iterator.More(); iterator.Next()) {
            const HistoryRecord candidate = history_record(
                "through_cut.wall.arc.0", "modified", "cut_profile.edge.arc.0",
                result, iterator.Value());
            if (candidate.output_present) {
              history.push_back(candidate);
              break;
            }
          }
        }
      }
    }
    return success_result(result, std::move(history));
  });
}

std::unique_ptr<NativeOperationResult> fuse_mixed_profile_native(
    const NativeOperationResult& base, rust::Slice<const double> segments,
    double origin_z, double height) noexcept {
  return guarded([&] {
    if (!base.valid() || segments.size() < 16 || segments.size() % 10 != 0) {
      return error_result(STATUS_INVALID_PARAMETER, "Polygon union payload is malformed");
    }
    BRepBuilderAPI_MakeWire wire_builder;
    std::size_t line_count = 0;
    std::size_t arc_count = 0;
    for (std::size_t offset = 0; offset < segments.size(); offset += 10) {
      const double kind = segments[offset];
      const gp_Pnt start(segments[offset + 1], segments[offset + 2], origin_z);
      const gp_Pnt end(segments[offset + 3], segments[offset + 4], origin_z);
      TopoDS_Edge edge;
      if (kind == 0.0) {
        edge = BRepBuilderAPI_MakeEdge(start, end).Edge();
        ++line_count;
      } else if (kind == 1.0) {
        const double center_x = segments[offset + 5];
        const double center_y = segments[offset + 6];
        const bool clockwise = segments[offset + 9] != 0.0;
        const double start_angle = std::atan2(start.Y() - center_y, start.X() - center_x);
        const double end_angle = std::atan2(end.Y() - center_y, end.X() - center_x);
        double sweep = end_angle - start_angle;
        const double tau = 2.0 * std::acos(-1.0);
        if (clockwise) {
          while (sweep >= 0.0) sweep -= tau;
        } else {
          while (sweep <= 0.0) sweep += tau;
        }
        const double radius = start.Distance(gp_Pnt(center_x, center_y, origin_z));
        const double middle_angle = start_angle + sweep / 2.0;
        GC_MakeArcOfCircle arc_builder(
            start,
            gp_Pnt(center_x + radius * std::cos(middle_angle),
                   center_y + radius * std::sin(middle_angle), origin_z),
            end);
        if (!arc_builder.IsDone()) {
          return error_result(STATUS_INVALID_SHAPE, "Polygon union arc builder did not complete");
        }
        edge = BRepBuilderAPI_MakeEdge(arc_builder.Value()).Edge();
        ++arc_count;
      } else {
        return error_result(STATUS_INVALID_PARAMETER, "Polygon union segment kind is invalid");
      }
      if (edge.IsNull()) {
        return error_result(STATUS_INVALID_SHAPE, "Polygon union edge is null");
      }
      wire_builder.Add(edge);
    }
    if (arc_count == 0 && line_count < 3) {
      return error_result(STATUS_INVALID_PARAMETER, "Polygon union profile shape is unsupported");
    }
    if (!wire_builder.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "Polygon union wire did not complete");
    }
    BRepBuilderAPI_MakeFace face_builder(wire_builder.Wire(), true);
    if (!face_builder.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "Polygon union face did not complete");
    }
    BRepPrimAPI_MakePrism prism(face_builder.Face(), gp_Vec(0.0, 0.0, height), true, false);
    if (!prism.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "Polygon union prism did not complete");
    }
    BRepAlgoAPI_Fuse operation(base.impl().shape, prism.Shape());
    operation.Build();
    if (operation.IsDone() && !operation.HasErrors()) {
      operation.SimplifyResult(true, true);
    }
    if (!operation.IsDone() || operation.HasErrors() || operation.Shape().IsNull()) {
      return error_result(STATUS_INVALID_SHAPE, "Polygon union operation did not complete");
    }
    const TopoDS_Shape result = operation.Shape();
    GProp_GProps before_properties;
    GProp_GProps after_properties;
    BRepGProp::VolumeProperties(base.impl().shape, before_properties);
    BRepGProp::VolumeProperties(result, after_properties);
    const double scale = std::max(1.0, std::abs(before_properties.Mass()));
    if (after_properties.Mass() - before_properties.Mass() <=
        scale * 16.0 * std::numeric_limits<double>::epsilon()) {
      return error_result(STATUS_NO_GEOMETRIC_CHANGE, "Polygon union produced no measurable geometric change");
    }
    return success_result(result, {});
  });
}

std::unique_ptr<NativeOperationResult> common_mixed_profile_native(
    const NativeOperationResult& base, rust::Slice<const double> segments,
    double origin_z, double height) noexcept {
  return guarded([&] {
    if (!base.valid() || segments.size() < 16 || segments.size() % 10 != 0) {
      return error_result(STATUS_INVALID_PARAMETER, "Polygon intersection payload is malformed");
    }
    BRepBuilderAPI_MakeWire wire_builder;
    std::size_t line_count = 0;
    std::size_t arc_count = 0;
    for (std::size_t offset = 0; offset < segments.size(); offset += 10) {
      const double kind = segments[offset];
      const gp_Pnt start(segments[offset + 1], segments[offset + 2], origin_z);
      const gp_Pnt end(segments[offset + 3], segments[offset + 4], origin_z);
      TopoDS_Edge edge;
      if (kind == 0.0) {
        edge = BRepBuilderAPI_MakeEdge(start, end).Edge();
        ++line_count;
      } else if (kind == 1.0) {
        const double center_x = segments[offset + 5];
        const double center_y = segments[offset + 6];
        const bool clockwise = segments[offset + 9] != 0.0;
        const double start_angle = std::atan2(start.Y() - center_y, start.X() - center_x);
        const double end_angle = std::atan2(end.Y() - center_y, end.X() - center_x);
        double sweep = end_angle - start_angle;
        const double tau = 2.0 * std::acos(-1.0);
        if (clockwise) {
          while (sweep >= 0.0) sweep -= tau;
        } else {
          while (sweep <= 0.0) sweep += tau;
        }
        const double radius = start.Distance(gp_Pnt(center_x, center_y, origin_z));
        const double middle_angle = start_angle + sweep / 2.0;
        GC_MakeArcOfCircle arc_builder(
            start,
            gp_Pnt(center_x + radius * std::cos(middle_angle),
                   center_y + radius * std::sin(middle_angle), origin_z),
            end);
        if (!arc_builder.IsDone()) {
          return error_result(STATUS_INVALID_SHAPE, "Polygon intersection arc builder did not complete");
        }
        edge = BRepBuilderAPI_MakeEdge(arc_builder.Value()).Edge();
        ++arc_count;
      } else {
        return error_result(STATUS_INVALID_PARAMETER, "Polygon intersection segment kind is invalid");
      }
      if (edge.IsNull()) {
        return error_result(STATUS_INVALID_SHAPE, "Polygon intersection edge is null");
      }
      wire_builder.Add(edge);
    }
    if (arc_count == 0 && line_count < 3) {
      return error_result(STATUS_INVALID_PARAMETER, "Polygon intersection profile shape is unsupported");
    }
    if (!wire_builder.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "Polygon intersection wire did not complete");
    }
    BRepBuilderAPI_MakeFace face_builder(wire_builder.Wire(), true);
    if (!face_builder.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "Polygon intersection face did not complete");
    }
    BRepPrimAPI_MakePrism prism(face_builder.Face(), gp_Vec(0.0, 0.0, height), true, false);
    if (!prism.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "Polygon intersection prism did not complete");
    }
    BRepAlgoAPI_Common operation(base.impl().shape, prism.Shape());
    operation.Build();
    if (!operation.IsDone() || operation.HasErrors() || operation.Shape().IsNull()) {
      return error_result(STATUS_INVALID_SHAPE, "Polygon intersection operation did not complete");
    }
    const TopoDS_Shape result = operation.Shape();
    GProp_GProps before_properties;
    GProp_GProps after_properties;
    BRepGProp::VolumeProperties(base.impl().shape, before_properties);
    BRepGProp::VolumeProperties(result, after_properties);
    const double scale = std::max(1.0, std::abs(before_properties.Mass()));
    if (before_properties.Mass() - after_properties.Mass() <=
        scale * 16.0 * std::numeric_limits<double>::epsilon()) {
      return error_result(STATUS_NO_GEOMETRIC_CHANGE, "Polygon intersection produced no measurable geometric change");
    }
    return success_result(result, {});
  });
}

std::unique_ptr<NativeOperationResult> split_mixed_profile_native(
    const NativeOperationResult& base, rust::Slice<const double> segments,
    double origin_z, double height) noexcept {
  return guarded([&] {
    if (!base.valid() || segments.size() < 16 || segments.size() % 10 != 0) {
      return error_result(STATUS_INVALID_PARAMETER, "Polygon split payload is malformed");
    }
    BRepBuilderAPI_MakeWire wire_builder;
    std::size_t line_count = 0;
    std::size_t arc_count = 0;
    for (std::size_t offset = 0; offset < segments.size(); offset += 10) {
      const double kind = segments[offset];
      const gp_Pnt start(segments[offset + 1], segments[offset + 2], origin_z);
      const gp_Pnt end(segments[offset + 3], segments[offset + 4], origin_z);
      TopoDS_Edge edge;
      if (kind == 0.0) {
        edge = BRepBuilderAPI_MakeEdge(start, end).Edge();
        ++line_count;
      } else if (kind == 1.0) {
        const double center_x = segments[offset + 5];
        const double center_y = segments[offset + 6];
        const bool clockwise = segments[offset + 9] != 0.0;
        const double start_angle = std::atan2(start.Y() - center_y, start.X() - center_x);
        const double end_angle = std::atan2(end.Y() - center_y, end.X() - center_x);
        double sweep = end_angle - start_angle;
        const double tau = 2.0 * std::acos(-1.0);
        if (clockwise) {
          while (sweep >= 0.0) sweep -= tau;
        } else {
          while (sweep <= 0.0) sweep += tau;
        }
        const double radius = start.Distance(gp_Pnt(center_x, center_y, origin_z));
        const double middle_angle = start_angle + sweep / 2.0;
        GC_MakeArcOfCircle arc_builder(
            start,
            gp_Pnt(center_x + radius * std::cos(middle_angle),
                   center_y + radius * std::sin(middle_angle), origin_z),
            end);
        if (!arc_builder.IsDone()) {
          return error_result(STATUS_INVALID_SHAPE, "Polygon split arc builder did not complete");
        }
        edge = BRepBuilderAPI_MakeEdge(arc_builder.Value()).Edge();
        ++arc_count;
      } else {
        return error_result(STATUS_INVALID_PARAMETER, "Polygon split segment kind is invalid");
      }
      if (edge.IsNull()) {
        return error_result(STATUS_INVALID_SHAPE, "Polygon split edge is null");
      }
      wire_builder.Add(edge);
    }
    if (arc_count == 0 && line_count < 3) {
      return error_result(STATUS_INVALID_PARAMETER, "Polygon split profile shape is unsupported");
    }
    if (!wire_builder.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "Polygon split wire did not complete");
    }
    BRepBuilderAPI_MakeFace face_builder(wire_builder.Wire(), true);
    if (!face_builder.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "Polygon split face did not complete");
    }
    BRepPrimAPI_MakePrism prism(face_builder.Face(), gp_Vec(0.0, 0.0, height), true, false);
    if (!prism.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "Polygon split prism did not complete");
    }
    BOPAlgo_Splitter operation;
    operation.AddArgument(base.impl().shape);
    operation.AddTool(prism.Shape());
    operation.Perform();
    if (operation.HasErrors()) {
      return error_result(STATUS_INVALID_SHAPE, "Polygon split operation did not complete");
    }
    const TopoDS_Shape result = operation.Shape();
    if (result.IsNull()) {
      return error_result(STATUS_NULL_RESULT, "Polygon split returned a null shape");
    }
    if (count_subshapes(result, TopAbs_SOLID) < 2) {
      return error_result(STATUS_NO_GEOMETRIC_CHANGE, "Polygon split did not produce multiple target fragments");
    }
    GProp_GProps before_properties;
    GProp_GProps after_properties;
    BRepGProp::VolumeProperties(base.impl().shape, before_properties);
    BRepGProp::VolumeProperties(result, after_properties);
    const double scale = std::max(1.0, std::abs(before_properties.Mass()));
    if (std::abs(after_properties.Mass() - before_properties.Mass()) > scale * 1.0e-10) {
      return error_result(STATUS_INVALID_SHAPE, "Polygon split did not preserve target volume");
    }
    std::vector<HistoryRecord> history;
    append_propagated_history(history, operation, result, base.impl());
    return success_result(result, std::move(history), true);
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
    Bnd_Box base_bounds;
    BRepBndLib::AddOptimal(base.impl().shape, base_bounds, false, false);
    double min_x = 0.0;
    double min_y = 0.0;
    double min_z = 0.0;
    double max_x = 0.0;
    double max_y = 0.0;
    double max_z = 0.0;
    base_bounds.Get(min_x, min_y, min_z, max_x, max_y, max_z);
    gp_Dir seam_direction(1.0, 0.0, 0.0);
    if (center_x - radius < min_x) {
      seam_direction = gp_Dir(-1.0, 0.0, 0.0);
    } else if (center_x + radius > max_x) {
      seam_direction = gp_Dir(1.0, 0.0, 0.0);
    } else if (center_y - radius < min_y) {
      seam_direction = gp_Dir(0.0, -1.0, 0.0);
    } else if (center_y + radius > max_y) {
      seam_direction = gp_Dir(0.0, 1.0, 0.0);
    }
    BRepPrimAPI_MakeCylinder tool_builder(
        gp_Ax2(
            gp_Pnt(center_x, center_y, origin_z),
            gp_Dir(0.0, 0.0, 1.0),
            seam_direction),
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
    append_propagated_history(history, operation, result, base.impl());
    append_cut_history(history, operation, result, base.impl().shape, "base.face.");
    append_cut_history(history, operation, result, tool, "tool.face.");
    return success_result(result, std::move(history));
  });
}

std::unique_ptr<NativeOperationResult> fuse_cylinder_native(
    const NativeOperationResult& base,
    double center_x, double center_y, double origin_z,
    double radius, double height) noexcept {
  return guarded([&] {
    if (!base.valid()) {
      return error_result(STATUS_INVALID_PARAMETER, "Cylinder fuse base is not a valid exact body");
    }
    BRepPrimAPI_MakeCylinder tool_builder(
        gp_Ax2(gp_Pnt(center_x, center_y, origin_z), gp_Dir(0.0, 0.0, 1.0)),
        radius,
        height);
    tool_builder.Build();
    if (!tool_builder.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT cylinder fuse tool builder did not complete");
    }
    BRepAlgoAPI_Fuse operation(base.impl().shape, tool_builder.Shape());
    operation.Build();
    if (operation.IsDone() && !operation.HasErrors()) {
      operation.SimplifyResult(true, true);
    }
    if (!operation.IsDone() || operation.HasErrors()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT cylinder fuse operation did not complete");
    }
    const TopoDS_Shape result = operation.Shape();
    if (result.IsNull()) {
      return error_result(STATUS_NULL_RESULT, "OCCT cylinder fuse returned a null shape");
    }
    GProp_GProps before_properties;
    GProp_GProps after_properties;
    BRepGProp::VolumeProperties(base.impl().shape, before_properties);
    BRepGProp::VolumeProperties(result, after_properties);
    const double scale = std::max(1.0, std::abs(before_properties.Mass()));
    if (after_properties.Mass() - before_properties.Mass() <=
        scale * 16.0 * std::numeric_limits<double>::epsilon()) {
      return error_result(STATUS_NO_GEOMETRIC_CHANGE, "Cylinder fuse produced no measurable geometric change");
    }
    return success_result(result, {});
  });
}

std::unique_ptr<NativeOperationResult> common_cylinder_native(
    const NativeOperationResult& base,
    double center_x, double center_y, double origin_z,
    double radius, double height) noexcept {
  return guarded([&] {
    if (!base.valid()) {
      return error_result(STATUS_INVALID_PARAMETER, "Cylinder common base is not a valid exact body");
    }
    BRepPrimAPI_MakeCylinder tool_builder(
        gp_Ax2(gp_Pnt(center_x, center_y, origin_z), gp_Dir(0.0, 0.0, 1.0)),
        radius,
        height);
    tool_builder.Build();
    if (!tool_builder.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT cylinder common tool builder did not complete");
    }
    BRepAlgoAPI_Common operation(base.impl().shape, tool_builder.Shape());
    operation.Build();
    if (!operation.IsDone() || operation.HasErrors()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT cylinder common operation did not complete");
    }
    const TopoDS_Shape result = operation.Shape();
    if (result.IsNull()) {
      return error_result(STATUS_NULL_RESULT, "OCCT cylinder common returned a null shape");
    }
    return success_result(result, {});
  });
}

std::unique_ptr<NativeOperationResult> split_cylinder_native(
    const NativeOperationResult& base,
    double center_x, double center_y, double origin_z,
    double radius, double height) noexcept {
  return guarded([&] {
    if (!base.valid()) {
      return error_result(STATUS_INVALID_PARAMETER, "Cylinder split base is not a valid exact body");
    }
    BRepPrimAPI_MakeCylinder tool_builder(
        gp_Ax2(gp_Pnt(center_x, center_y, origin_z), gp_Dir(0.0, 0.0, 1.0)),
        radius,
        height);
    tool_builder.Build();
    if (!tool_builder.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT cylinder split tool builder did not complete");
    }
    BOPAlgo_Splitter operation;
    operation.AddArgument(base.impl().shape);
    operation.AddTool(tool_builder.Shape());
    operation.Perform();
    if (operation.HasErrors()) {
      return error_result(STATUS_INVALID_SHAPE, "OCCT cylinder split operation did not complete");
    }
    const TopoDS_Shape result = operation.Shape();
    if (result.IsNull()) {
      return error_result(STATUS_NULL_RESULT, "OCCT cylinder split returned a null shape");
    }
    if (count_subshapes(result, TopAbs_SOLID) < 2) {
      return error_result(STATUS_NO_GEOMETRIC_CHANGE, "Cylinder split did not produce multiple target fragments");
    }
    GProp_GProps before_properties;
    GProp_GProps after_properties;
    BRepGProp::VolumeProperties(base.impl().shape, before_properties);
    BRepGProp::VolumeProperties(result, after_properties);
    const double scale = std::max(1.0, std::abs(before_properties.Mass()));
    if (std::abs(after_properties.Mass() - before_properties.Mass()) > scale * 1.0e-10) {
      return error_result(STATUS_INVALID_SHAPE, "Cylinder split did not preserve target volume");
    }
    std::vector<HistoryRecord> history;
    append_propagated_history(history, operation, result, base.impl());
    return success_result(result, std::move(history), true);
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
    std::vector<HistoryRecord> history;
    append_propagated_history(history, operation, result, base.impl());
    return success_result(result, std::move(history), true);
  });
}

std::unique_ptr<NativeOperationResult> exception_probe_native() noexcept {
  return guarded([]() -> std::unique_ptr<NativeOperationResult> {
    throw Standard_Failure("intentional A0 exception-boundary probe");
    return error_result(STATUS_BACKEND_EXCEPTION, "unreachable");
  });
}

namespace {

constexpr std::size_t MAX_STEP_XDE_NODES = 1024;
constexpr std::size_t MAX_STEP_XDE_DEPTH = 64;

class IgesBrepModeGuard {
public:
  IgesBrepModeGuard() : lock_(mutex()) {
    IGESControl_Controller::Init();
    previous_ = Interface_Static::IVal("write.iges.brep.mode");
    active_ = Interface_Static::SetIVal("write.iges.brep.mode", 1);
  }

  ~IgesBrepModeGuard() {
    if (active_) {
      Interface_Static::SetIVal("write.iges.brep.mode", previous_);
    }
  }

  bool active() const noexcept { return active_; }

private:
  static std::mutex& mutex() {
    static std::mutex value;
    return value;
  }

  std::unique_lock<std::mutex> lock_;
  int previous_ = 0;
  bool active_ = false;
};

struct StepXdeNode {
  std::uint32_t id;
  std::int32_t parent_id;
  std::int32_t part_index;
  std::string name;
  bool name_from_source;
  std::string color;
  gp_Trsf transform;
};

struct StepXdeData {
  occ::handle<TDocStd_Document> document;
  occ::handle<XCAFDoc_ShapeTool> shapes;
  occ::handle<XCAFDoc_ColorTool> colors;
  std::vector<TDF_Label> parts;
  std::vector<StepXdeNode> nodes;
};

std::string hex_encode(const std::string& value) {
  static constexpr char digits[] = "0123456789abcdef";
  std::string encoded;
  encoded.reserve(value.size() * 2);
  for (const unsigned char byte : value) {
    encoded.push_back(digits[byte >> 4]);
    encoded.push_back(digits[byte & 0x0f]);
  }
  return encoded;
}

std::string label_entry(const TDF_Label& label) {
  TCollection_AsciiString entry;
  TDF_Tool::Entry(label, entry);
  return entry.ToCString();
}

std::string label_name(const TDF_Label& label) {
  occ::handle<TDataStd_Name> attribute;
  if (!label.FindAttribute(TDataStd_Name::GetID(), attribute) || attribute.IsNull()) {
    return {};
  }
  const TCollection_ExtendedString& value = attribute->Get();
  std::vector<char> utf8(static_cast<std::size_t>(value.LengthOfCString()) + 1, '\0');
  Standard_PCharacter output = utf8.data();
  const int length = value.ToUTF8CString(output);
  return length > 0 ? std::string(utf8.data(), static_cast<std::size_t>(length)) : std::string();
}

std::string label_color(
    const occ::handle<XCAFDoc_ColorTool>& colors,
    const TDF_Label& primary,
    const TDF_Label& fallback) {
  Quantity_Color color;
  const auto read = [&](const TDF_Label& label) {
    return XCAFDoc_ColorTool::GetColor(label, XCAFDoc_ColorGen, color)
        || XCAFDoc_ColorTool::GetColor(label, XCAFDoc_ColorSurf, color);
  };
  if (!read(primary) && (fallback.IsNull() || !read(fallback))) {
    return "-";
  }
  double red = 0.0;
  double green = 0.0;
  double blue = 0.0;
  color.Values(red, green, blue, Quantity_TOC_sRGB);
  const auto byte = [](double value) {
    return static_cast<unsigned int>(std::lround(std::clamp(value, 0.0, 1.0) * 255.0));
  };
  std::ostringstream encoded;
  encoded << std::hex << std::setfill('0')
          << std::setw(2) << byte(red)
          << std::setw(2) << byte(green)
          << std::setw(2) << byte(blue);
  return encoded.str();
}

template <typename Reader>
bool load_xde(const std::string& path, StepXdeData& data, std::string& error) {
  data.document = new TDocStd_Document(TCollection_ExtendedString("BinXCAF"));
  Reader reader;
  reader.SetColorMode(true);
  reader.SetNameMode(true);
  if (reader.ReadFile(path.c_str()) != IFSelect_RetDone) {
    error = "STEP XDE reader could not read the source";
    return false;
  }
  if (!reader.Transfer(data.document)) {
    error = "STEP XDE source contains no transferable roots";
    return false;
  }
  data.shapes = XCAFDoc_DocumentTool::ShapeTool(data.document->Main());
  data.colors = XCAFDoc_DocumentTool::ColorTool(data.document->Main());
  if (data.shapes.IsNull() || data.colors.IsNull()) {
    error = "STEP XDE document tools are unavailable";
    return false;
  }

  std::map<std::string, std::uint32_t> part_indices;
  std::set<std::string> active_assemblies;
  std::function<bool(const TDF_Label&, const TDF_Label&, std::int32_t, const gp_Trsf&, std::size_t)>
      visit;
  visit = [&](const TDF_Label& instance,
              const TDF_Label& referred,
              std::int32_t parent_id,
              const gp_Trsf& transform,
              std::size_t depth) {
    if (depth > MAX_STEP_XDE_DEPTH || data.nodes.size() >= MAX_STEP_XDE_NODES) {
      error = "STEP XDE hierarchy exceeds the bounded envelope";
      return false;
    }
    const bool assembly = XCAFDoc_ShapeTool::IsAssembly(referred);
    std::int32_t part_index = -1;
    if (!assembly) {
      const std::string entry = label_entry(referred);
      const auto [iterator, inserted] = part_indices.emplace(entry, part_indices.size());
      if (inserted) {
        if (data.parts.size() >= MAX_STEP_XDE_NODES) {
          error = "STEP XDE part count exceeds the bounded envelope";
          return false;
        }
        data.parts.push_back(referred);
      }
      part_index = static_cast<std::int32_t>(iterator->second);
    }
    std::string name = label_name(instance);
    if (name.empty()) {
      name = label_name(referred);
    }
    const bool name_from_source = !name.empty();
    if (!name_from_source) {
      name = assembly ? "Imported STEP assembly" : "Imported STEP part";
    }
    const std::uint32_t node_id = static_cast<std::uint32_t>(data.nodes.size());
    data.nodes.push_back(StepXdeNode{
        node_id,
        parent_id,
        part_index,
        std::move(name),
        name_from_source,
        label_color(data.colors, instance, referred),
        transform,
    });
    if (!assembly) {
      return true;
    }

    const std::string assembly_entry = label_entry(referred);
    if (!active_assemblies.insert(assembly_entry).second) {
      error = "STEP XDE hierarchy contains an assembly cycle";
      return false;
    }
    NCollection_Sequence<TDF_Label> components;
    if (!XCAFDoc_ShapeTool::GetComponents(referred, components, false)) {
      active_assemblies.erase(assembly_entry);
      error = "STEP XDE assembly has no readable components";
      return false;
    }
    for (int index = 1; index <= components.Length(); ++index) {
      const TDF_Label component = components.Value(index);
      TDF_Label target;
      if (!XCAFDoc_ShapeTool::GetReferredShape(component, target) || target.IsNull()) {
        active_assemblies.erase(assembly_entry);
        error = "STEP XDE component has no referred definition";
        return false;
      }
      const gp_Trsf local = XCAFDoc_ShapeTool::GetLocation(component).Transformation();
      if (!visit(component, target, static_cast<std::int32_t>(node_id), local, depth + 1)) {
        active_assemblies.erase(assembly_entry);
        return false;
      }
    }
    active_assemblies.erase(assembly_entry);
    return true;
  };

  NCollection_Sequence<TDF_Label> roots;
  data.shapes->GetFreeShapes(roots);
  if (roots.IsEmpty()) {
    error = "STEP XDE source contains no free shapes";
    return false;
  }
  for (int index = 1; index <= roots.Length(); ++index) {
    const TDF_Label root = roots.Value(index);
    TDF_Label referred = root;
    if (XCAFDoc_ShapeTool::IsReference(root)
        && !XCAFDoc_ShapeTool::GetReferredShape(root, referred)) {
      error = "STEP XDE root reference is unresolved";
      return false;
    }
    const gp_Trsf transform = XCAFDoc_ShapeTool::GetLocation(root).Transformation();
    if (!visit(root, referred, -1, transform, 0)) {
      return false;
    }
  }
  if (data.parts.empty() || data.nodes.empty()) {
    error = "STEP XDE source contains no supported exact parts";
    return false;
  }
  return true;
}

bool load_step_xde(const std::string& path, StepXdeData& data, std::string& error) {
  return load_xde<STEPCAFControl_Reader>(path, data, error);
}

bool load_iges_xde(const std::string& path, StepXdeData& data, std::string& error) {
  if (!load_xde<IGESCAFControl_Reader>(path, data, error)) {
    return false;
  }
  if (data.parts.size() != 1 || data.nodes.size() != 1) {
    return true;
  }
  const TDF_Label root = data.parts.front();
  const TopoDS_Shape root_shape = XCAFDoc_ShapeTool::GetShape(root);
  if (root_shape.IsNull() || count_subshapes(root_shape, TopAbs_SOLID) < 2) {
    return true;
  }

  std::vector<TDF_Label> parts;
  std::vector<StepXdeNode> nodes;
  std::string root_name = label_name(root);
  const bool root_name_from_source = !root_name.empty();
  if (!root_name_from_source) {
    root_name = "Imported IGES model";
  }
  nodes.push_back(StepXdeNode{
      0, -1, -1, root_name, root_name_from_source,
      label_color(data.colors, root, TDF_Label()), gp_Trsf()});
  for (TopExp_Explorer solids(root_shape, TopAbs_SOLID); solids.More(); solids.Next()) {
    if (parts.size() >= MAX_STEP_XDE_NODES || nodes.size() >= MAX_STEP_XDE_NODES) {
      error = "IGES XDE solid count exceeds the bounded envelope";
      return false;
    }
    TDF_Label part;
    if (!data.shapes->FindSubShape(root, solids.Current(), part) || part.IsNull()) {
      part = data.shapes->AddSubShape(root, solids.Current());
    }
    if (part.IsNull()) {
      error = "IGES XDE reader could not identify a transferred solid";
      return false;
    }
    std::string name = label_name(part);
    const bool name_from_source = !name.empty();
    if (!name_from_source) {
      name = "Imported IGES part " + std::to_string(parts.size() + 1);
    }
    const std::uint32_t part_index = static_cast<std::uint32_t>(parts.size());
    parts.push_back(part);
    nodes.push_back(StepXdeNode{
        static_cast<std::uint32_t>(nodes.size()), 0,
        static_cast<std::int32_t>(part_index), name, name_from_source,
        label_color(data.colors, part, root), gp_Trsf()});
  }
  data.parts = std::move(parts);
  data.nodes = std::move(nodes);
  return true;
}

std::string matrix_fields(const gp_Trsf& transform) {
  std::ostringstream output;
  output << std::hex << std::setfill('0');
  for (int row = 1; row <= 4; ++row) {
    for (int column = 1; column <= 4; ++column) {
      const double value = row == 4 ? (column == 4 ? 1.0 : 0.0) : transform.Value(row, column);
      std::uint64_t bits = 0;
      static_assert(sizeof(bits) == sizeof(value));
      std::memcpy(&bits, &value, sizeof(bits));
      output << '\t' << std::setw(16) << bits;
    }
  }
  return output.str();
}

struct StepXdeExportPart {
  std::string path;
  std::string name;
  TopoDS_Shape shape;
  TDF_Label label;
};

struct StepXdeExportNode {
  std::int32_t parent_id;
  std::int32_t part_index;
  std::string name;
  bool has_color;
  std::array<unsigned char, 3> color;
  gp_Trsf transform;
};

bool hex_decode(const std::string& encoded, std::string& decoded) {
  if (encoded.size() % 2 != 0) {
    return false;
  }
  const auto nibble = [](char value) -> int {
    if (value >= '0' && value <= '9') return value - '0';
    if (value >= 'a' && value <= 'f') return value - 'a' + 10;
    if (value >= 'A' && value <= 'F') return value - 'A' + 10;
    return -1;
  };
  decoded.clear();
  decoded.reserve(encoded.size() / 2);
  for (std::size_t index = 0; index < encoded.size(); index += 2) {
    const int high = nibble(encoded[index]);
    const int low = nibble(encoded[index + 1]);
    if (high < 0 || low < 0) {
      return false;
    }
    decoded.push_back(static_cast<char>((high << 4) | low));
  }
  return true;
}

std::vector<std::string> split_tabs(const std::string& line) {
  std::vector<std::string> fields;
  std::size_t start = 0;
  while (true) {
    const std::size_t end = line.find('\t', start);
    fields.push_back(line.substr(start, end - start));
    if (end == std::string::npos) break;
    start = end + 1;
  }
  return fields;
}

bool parse_i32(const std::string& value, std::int32_t& parsed) {
  try {
    std::size_t consumed = 0;
    const long long number = std::stoll(value, &consumed, 10);
    if (consumed != value.size()
        || number < std::numeric_limits<std::int32_t>::min()
        || number > std::numeric_limits<std::int32_t>::max()) {
      return false;
    }
    parsed = static_cast<std::int32_t>(number);
    return true;
  } catch (...) {
    return false;
  }
}

bool parse_export_color(
    const std::string& value, bool& has_color, std::array<unsigned char, 3>& color) {
  if (value == "-") {
    has_color = false;
    color = {0, 0, 0};
    return true;
  }
  std::string bytes;
  if (value.size() != 6 || !hex_decode(value, bytes) || bytes.size() != 3) {
    return false;
  }
  has_color = true;
  color = {
      static_cast<unsigned char>(bytes[0]),
      static_cast<unsigned char>(bytes[1]),
      static_cast<unsigned char>(bytes[2])};
  return true;
}

bool parse_export_transform(
    const std::vector<std::string>& fields, std::size_t start, gp_Trsf& transform) {
  if (fields.size() != start + 16) {
    return false;
  }
  std::array<double, 16> matrix{};
  for (std::size_t index = 0; index < matrix.size(); ++index) {
    try {
      std::size_t consumed = 0;
      const std::uint64_t bits = std::stoull(fields[start + index], &consumed, 16);
      if (consumed != fields[start + index].size()) return false;
      std::memcpy(&matrix[index], &bits, sizeof(bits));
      if (!std::isfinite(matrix[index])) return false;
    } catch (...) {
      return false;
    }
  }
  constexpr double epsilon = 1.0e-10;
  if (std::abs(matrix[12]) > epsilon || std::abs(matrix[13]) > epsilon
      || std::abs(matrix[14]) > epsilon || std::abs(matrix[15] - 1.0) > epsilon) {
    return false;
  }
  try {
    transform.SetValues(
        matrix[0], matrix[1], matrix[2], matrix[3],
        matrix[4], matrix[5], matrix[6], matrix[7],
        matrix[8], matrix[9], matrix[10], matrix[11]);
    return transform.Form() != gp_Other;
  } catch (...) {
    return false;
  }
}

bool parse_step_xde_export(
    const std::string& manifest,
    std::vector<StepXdeExportPart>& parts,
    std::vector<StepXdeExportNode>& nodes,
    std::string& error) {
  std::istringstream input(manifest);
  std::string line;
  if (!std::getline(input, line) || line != "KETCHUP_STEP_XDE_EXPORT_V1") {
    error = "STEP XDE export manifest has an unsupported schema";
    return false;
  }
  while (std::getline(input, line)) {
    if (!line.empty() && line.back() == '\r') line.pop_back();
    if (line.empty()) continue;
    const auto fields = split_tabs(line);
    if (fields[0] == "P" && fields.size() == 3) {
      if (parts.size() >= MAX_STEP_XDE_NODES) {
        error = "STEP XDE export part count exceeds the bounded envelope";
        return false;
      }
      StepXdeExportPart part;
      if (!hex_decode(fields[1], part.path) || !hex_decode(fields[2], part.name)
          || part.path.empty() || part.name.empty()) {
        error = "STEP XDE export part is malformed";
        return false;
      }
      parts.push_back(std::move(part));
      continue;
    }
    if (fields[0] == "N" && fields.size() == 21) {
      if (nodes.size() >= MAX_STEP_XDE_NODES) {
        error = "STEP XDE export node count exceeds the bounded envelope";
        return false;
      }
      StepXdeExportNode node;
      if (!parse_i32(fields[1], node.parent_id)
          || !parse_i32(fields[2], node.part_index)
          || !hex_decode(fields[3], node.name)
          || node.name.empty()
          || !parse_export_color(fields[4], node.has_color, node.color)
          || !parse_export_transform(fields, 5, node.transform)) {
        error = "STEP XDE export node is malformed";
        return false;
      }
      const std::int32_t node_id = static_cast<std::int32_t>(nodes.size());
      if (node.parent_id >= node_id || node.parent_id < -1
          || node.part_index < -1
          || node.part_index >= static_cast<std::int32_t>(parts.size())) {
        error = "STEP XDE export hierarchy or part reference is invalid";
        return false;
      }
      if (node.parent_id >= 0
          && nodes[static_cast<std::size_t>(node.parent_id)].part_index >= 0) {
        error = "STEP XDE export part node cannot contain children";
        return false;
      }
      std::size_t depth = 0;
      for (std::int32_t parent = node.parent_id; parent >= 0;
           parent = nodes[static_cast<std::size_t>(parent)].parent_id) {
        if (++depth > MAX_STEP_XDE_DEPTH) {
          error = "STEP XDE export hierarchy exceeds the bounded depth";
          return false;
        }
      }
      nodes.push_back(std::move(node));
      continue;
    }
    error = "STEP XDE export manifest row is malformed";
    return false;
  }
  if (parts.empty() || nodes.empty()) {
    error = "STEP XDE export manifest has no parts or nodes";
    return false;
  }
  for (std::size_t index = 0; index < parts.size(); ++index) {
    if (std::none_of(nodes.begin(), nodes.end(), [index](const StepXdeExportNode& node) {
          return node.part_index == static_cast<std::int32_t>(index);
        })) {
      error = "STEP XDE export contains an unreferenced part";
      return false;
    }
  }
  return true;
}

void set_xde_name(const TDF_Label& label, const std::string& name) {
  TDataStd_Name::Set(label, TCollection_ExtendedString(name.c_str(), true));
}

} // namespace

rust::String xde_manifest_native(rust::Str path, bool iges) noexcept {
  try {
    StepXdeData data;
    std::string error;
    const std::string native_path(path.data(), path.size());
    const bool loaded = iges
        ? load_iges_xde(native_path, data, error)
        : load_step_xde(native_path, data, error);
    if (native_path.empty() || !loaded) {
      return rust::String("ERR\t" + hex_encode(error.empty() ? "invalid STEP XDE path" : error));
    }
    std::ostringstream output;
    output << "KETCHUP_STEP_XDE_V1\n";
    for (std::size_t index = 0; index < data.parts.size(); ++index) {
      const TDF_Label& part = data.parts[index];
      std::string name = label_name(part);
      const bool name_from_source = !name.empty();
      if (!name_from_source) {
        name = "Imported STEP part";
      }
      output << "P\t" << index << '\t' << hex_encode(name) << '\t'
             << (name_from_source ? '1' : '0') << '\t'
             << label_color(data.colors, part, TDF_Label()) << '\n';
    }
    for (const StepXdeNode& node : data.nodes) {
      output << "N\t" << node.id << '\t' << node.parent_id << '\t' << node.part_index
             << '\t' << hex_encode(node.name) << '\t' << (node.name_from_source ? '1' : '0')
             << '\t' << node.color << matrix_fields(node.transform) << '\n';
    }
    return rust::String(output.str());
  } catch (const Standard_Failure& failure) {
    return rust::String("ERR\t" + hex_encode(failure.GetMessageString()));
  } catch (const std::exception& failure) {
    return rust::String("ERR\t" + hex_encode(failure.what()));
  } catch (...) {
    return rust::String("ERR\t" + hex_encode("unknown STEP XDE exception"));
  }
}

rust::String step_xde_manifest_native(rust::Str path) noexcept {
  return xde_manifest_native(path, false);
}

rust::String iges_xde_manifest_native(rust::Str path) noexcept {
  return xde_manifest_native(path, true);
}

template <typename Writer>
rust::String export_xde_assembly_native(
    rust::Str manifest, rust::Str path) noexcept {
  try {
    const std::string encoded(manifest.data(), manifest.size());
    const std::string output_path(path.data(), path.size());
    std::vector<StepXdeExportPart> parts;
    std::vector<StepXdeExportNode> nodes;
    std::string error;
    if (output_path.empty() || !parse_step_xde_export(encoded, parts, nodes, error)) {
      return rust::String(error.empty() ? "invalid STEP XDE export path" : error);
    }

    const occ::handle<TDocStd_Document> document =
        new TDocStd_Document(TCollection_ExtendedString("BinXCAF"));
    const occ::handle<XCAFDoc_ShapeTool> shapes =
        XCAFDoc_DocumentTool::ShapeTool(document->Main());
    const occ::handle<XCAFDoc_ColorTool> colors =
        XCAFDoc_DocumentTool::ColorTool(document->Main());
    if (shapes.IsNull() || colors.IsNull()) {
      return rust::String("STEP XDE document tools are unavailable");
    }

    for (StepXdeExportPart& part : parts) {
      STEPControl_Reader reader;
      if (reader.ReadFile(part.path.c_str()) != IFSelect_RetDone
          || reader.TransferRoots() == 0) {
        return rust::String("STEP XDE writer could not reread an exact part source");
      }
      part.shape = reader.OneShape();
      if (part.shape.IsNull()) {
        return rust::String("STEP XDE writer received a null exact part shape");
      }
      if constexpr (std::is_same_v<Writer, STEPCAFControl_Writer>) {
        part.label = shapes->AddShape(part.shape, false);
        if (part.label.IsNull()) {
          return rust::String("STEP XDE writer could not register an exact part definition");
        }
        set_xde_name(part.label, part.name);
      }
    }

    if constexpr (std::is_same_v<Writer, STEPCAFControl_Writer>) {
      std::vector<TDF_Label> node_definitions(nodes.size());
    for (std::size_t index = 0; index < nodes.size(); ++index) {
      const StepXdeExportNode& node = nodes[index];
      if (node.part_index >= 0) {
        node_definitions[index] = parts[static_cast<std::size_t>(node.part_index)].label;
      } else {
        node_definitions[index] = shapes->NewShape();
        if (node_definitions[index].IsNull()) {
          return rust::String("STEP XDE writer could not create an assembly definition");
        }
        set_xde_name(node_definitions[index], node.name);
      }
    }

    const TDF_Label root = shapes->NewShape();
    if (root.IsNull()) {
      return rust::String("STEP XDE writer could not create the model root");
    }
    set_xde_name(root, "Ketchup model");
    for (std::size_t index = 0; index < nodes.size(); ++index) {
      const StepXdeExportNode& node = nodes[index];
      const TDF_Label parent = node.parent_id < 0
          ? root
          : node_definitions[static_cast<std::size_t>(node.parent_id)];
      const TDF_Label component = shapes->AddComponent(
          parent, node_definitions[index], TopLoc_Location(node.transform));
      if (component.IsNull()) {
        return rust::String("STEP XDE writer could not attach an assembly component");
      }
      set_xde_name(component, node.name);
      if (node.has_color) {
        const Quantity_Color color(
            static_cast<double>(node.color[0]) / 255.0,
            static_cast<double>(node.color[1]) / 255.0,
            static_cast<double>(node.color[2]) / 255.0,
            Quantity_TOC_sRGB);
        colors->SetColor(component, color, XCAFDoc_ColorGen);
        colors->SetColor(component, color, XCAFDoc_ColorSurf);
      }
    }
      shapes->UpdateAssemblies();
    } else {
      for (std::size_t index = 0; index < nodes.size(); ++index) {
        const StepXdeExportNode& node = nodes[index];
        if (node.part_index < 0) {
          continue;
        }
        gp_Trsf world = node.transform;
        for (std::int32_t parent = node.parent_id; parent >= 0;
             parent = nodes[static_cast<std::size_t>(parent)].parent_id) {
          gp_Trsf composed = nodes[static_cast<std::size_t>(parent)].transform;
          composed.Multiply(world);
          world = composed;
        }
        BRepBuilderAPI_Transform placed(
            parts[static_cast<std::size_t>(node.part_index)].shape, world, true);
        if (!placed.IsDone()) {
          return rust::String("IGES XDE writer could not bake an occurrence transform");
        }
        const TDF_Label occurrence = shapes->AddShape(placed.Shape(), false);
        if (occurrence.IsNull()) {
          return rust::String("IGES XDE writer could not register an occurrence solid");
        }
        set_xde_name(occurrence, node.name);
        if (node.has_color) {
          const Quantity_Color color(
              static_cast<double>(node.color[0]) / 255.0,
              static_cast<double>(node.color[1]) / 255.0,
              static_cast<double>(node.color[2]) / 255.0,
              Quantity_TOC_sRGB);
          colors->SetColor(occurrence, color, XCAFDoc_ColorGen);
          colors->SetColor(occurrence, color, XCAFDoc_ColorSurf);
        }
      }
    }

    std::unique_ptr<IgesBrepModeGuard> iges_mode;
    if constexpr (std::is_same_v<Writer, IGESCAFControl_Writer>) {
      iges_mode = std::make_unique<IgesBrepModeGuard>();
      if (!iges_mode->active()) {
        return rust::String("IGES XDE writer could not enable BRep mode");
      }
    }
    Writer writer;
    writer.SetColorMode(true);
    writer.SetNameMode(true);
    bool transferred = false;
    if constexpr (std::is_same_v<Writer, STEPCAFControl_Writer>) {
      transferred = writer.Transfer(document, STEPControl_AsIs);
    } else {
      transferred = writer.Transfer(document);
    }
    if (!transferred) {
      return rust::String("XDE writer could not transfer the assembly document");
    }
    bool written = false;
    if constexpr (std::is_same_v<Writer, STEPCAFControl_Writer>) {
      written = writer.Write(output_path.c_str()) == IFSelect_RetDone;
    } else {
      written = writer.Write(output_path.c_str());
    }
    if (!written) {
      return rust::String("XDE writer could not write the assembly document");
    }
    return rust::String();
  } catch (const Standard_Failure& failure) {
    return rust::String(failure.GetMessageString());
  } catch (const std::exception& failure) {
    return rust::String(failure.what());
  } catch (...) {
    return rust::String("unknown STEP XDE export exception");
  }
}

rust::String export_step_xde_assembly_native(
    rust::Str manifest, rust::Str path) noexcept {
  return export_xde_assembly_native<STEPCAFControl_Writer>(manifest, path);
}

rust::String export_iges_xde_assembly_native(
    rust::Str manifest, rust::Str path) noexcept {
  return export_xde_assembly_native<IGESCAFControl_Writer>(manifest, path);
}

std::unique_ptr<NativeOperationResult> import_xde_part_native(
    rust::Str path, std::uint32_t part_index, bool iges) noexcept {
  return guarded([&] {
    const std::string native_path(path.data(), path.size());
    if (iges) {
      IGESControl_Reader reader;
      if (reader.ReadFile(native_path.c_str()) != IFSelect_RetDone) {
        return error_result(STATUS_INVALID_PARAMETER, "IGES reader could not read the source");
      }
      const int roots = reader.NbRootsForTransfer();
      if (part_index >= static_cast<std::uint32_t>(std::max(roots, 0))
          || !reader.TransferOneRoot(static_cast<int>(part_index) + 1)
          || reader.NbShapes() == 0) {
        return error_result(STATUS_INVALID_PARAMETER, "IGES part index is outside the transferable roots");
      }
      const TopoDS_Shape shape = reader.Shape(reader.NbShapes());
      const std::uint32_t solids = count_subshapes(shape, TopAbs_SOLID);
      return success_result(shape, {}, solids >= 2, false, {}, solids == 0);
    }
    StepXdeData data;
    std::string error;
    const bool loaded = load_step_xde(native_path, data, error);
    if (!loaded) {
      return error_result(STATUS_INVALID_PARAMETER, error);
    }
    if (part_index >= data.parts.size()) {
      return error_result(STATUS_INVALID_PARAMETER, "STEP XDE part index is outside the manifest");
    }
    const TopoDS_Shape shape = XCAFDoc_ShapeTool::GetShape(data.parts[part_index]);
    if (shape.IsNull()) {
      return error_result(STATUS_INVALID_SHAPE, "STEP XDE part has no exact shape");
    }
    const std::uint32_t solids = count_subshapes(shape, TopAbs_SOLID);
    return success_result(shape, {}, solids >= 2, false, {}, solids == 0);
  });
}

std::unique_ptr<NativeOperationResult> import_step_xde_part_native(
    rust::Str path, std::uint32_t part_index) noexcept {
  return import_xde_part_native(path, part_index, false);
}

std::unique_ptr<NativeOperationResult> import_iges_xde_part_native(
    rust::Str path, std::uint32_t part_index) noexcept {
  return import_xde_part_native(path, part_index, true);
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
    const std::uint32_t solids = count_subshapes(shape, TopAbs_SOLID);
    return success_result(shape, {}, solids >= 2, false, {}, solids == 0);
  });
}

std::unique_ptr<NativeOperationResult> import_step_solid_native(
    rust::Str path, std::uint32_t solid_ordinal) noexcept {
  return guarded([&] {
    const std::string native_path(path.data(), path.size());
    STEPControl_Reader reader;
    if (reader.ReadFile(native_path.c_str()) != IFSelect_RetDone) {
      return error_result(STATUS_INVALID_PARAMETER, "STEP reader could not read the fixture");
    }
    if (reader.TransferRoots() == 0) {
      return error_result(STATUS_INVALID_SHAPE, "STEP fixture contains no transferable roots");
    }
    TopExp_Explorer explorer(reader.OneShape(), TopAbs_SOLID);
    for (std::uint32_t ordinal = 0; ordinal < solid_ordinal && explorer.More(); ++ordinal) {
      explorer.Next();
    }
    if (!explorer.More()) {
      return error_result(STATUS_INVALID_PARAMETER, "STEP solid ordinal is outside the transferred assembly");
    }
    return success_result(explorer.Current(), {});
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

std::unique_ptr<NativeOperationResult> import_iges_native(rust::Str path) noexcept {
  return guarded([&] {
    const std::string native_path(path.data(), path.size());
    IGESControl_Reader reader;
    if (reader.ReadFile(native_path.c_str()) != IFSelect_RetDone) {
      return error_result(STATUS_INVALID_PARAMETER, "IGES reader could not read the source");
    }
    if (reader.TransferRoots() == 0) {
      return error_result(STATUS_INVALID_SHAPE, "IGES source contains no transferable roots");
    }
    const TopoDS_Shape shape = reader.OneShape();
    const std::uint32_t solids = count_subshapes(shape, TopAbs_SOLID);
    return success_result(shape, {}, solids >= 2, false, {}, solids == 0);
  });
}

rust::String iges_length_unit_native(rust::Str path) noexcept {
  try {
    const std::string native_path(path.data(), path.size());
    IGESControl_Reader reader;
    if (reader.ReadFile(native_path.c_str()) != IFSelect_RetDone || reader.IGESModel().IsNull()) {
      return rust::String();
    }
    const auto unit_name = reader.IGESModel()->GlobalSection().UnitName();
    if (unit_name.IsNull()) {
      return rust::String();
    }
    std::string unit(unit_name->ToCString());
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
        std::abs(dot_column(0, 0) - 1.0) <= 1.0e-10
        && std::abs(dot_column(1, 1) - 1.0) <= 1.0e-10
        && std::abs(dot_column(2, 2) - 1.0) <= 1.0e-10
        && std::abs(dot_column(0, 1)) <= 1.0e-10
        && std::abs(dot_column(0, 2)) <= 1.0e-10
        && std::abs(dot_column(1, 2)) <= 1.0e-10;
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
    if (normal.SquareMagnitude() <= 1.0e-18) {
      return error_result(STATUS_DEGENERATE_OPERATION, "Plane trim normal is degenerate");
    }
    const gp_Pnt origin(origin_x, origin_y, origin_z);
    const gp_Pnt keep(keep_x, keep_y, keep_z);
    if (std::abs(gp_Vec(origin, keep).Dot(normal.Normalized())) <= 1.0e-7) {
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
    BRepAlgoAPI_Common operation(body.impl().shape, half_space_builder.Solid());
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
    const double tolerance = 1.0e-9 * std::max(source_volume, 1.0);
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
      BRepAlgoAPI_Cut operation(target.impl().shape, tool.impl().shape);
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
      BRepAlgoAPI_Fuse operation(target.impl().shape, tool.impl().shape);
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
      BRepAlgoAPI_Common operation(target.impl().shape, tool.impl().shape);
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
    // Exact geometry bounds include shape tolerances, never render triangulation.
    Bnd_Box left_bounds, right_bounds;
    BRepBndLib::AddOptimal(left.impl().shape, left_bounds, false, true);
    BRepBndLib::AddOptimal(right.impl().shape, right_bounds, false, true);
    double volume = 0.0;
    double contact_area = 0.0;
    if (left_bounds.IsVoid() || right_bounds.IsVoid() || !left_bounds.IsOut(right_bounds)) {
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
    // Bounds reject Boolean work only; preserve exact distance and contact tolerance.
    // The two-shape constructor already performs the distance computation.
    BRepExtrema_DistShapeShape distance(left.impl().shape, right.impl().shape);
    if (!distance.IsDone() || distance.NbSolution() == 0 ||
        !std::isfinite(distance.Value()) || distance.Value() < 0.0) {
      result.diagnostic = "OCCT pair volume or distance query failed";
      return result;
    }
    if (distance.Value() == 0.0) {
      for (TopExp_Explorer left_faces(left.impl().shape, TopAbs_FACE); left_faces.More();
           left_faces.Next()) {
        Bnd_Box left_face_bounds;
        BRepBndLib::AddOptimal(left_faces.Current(), left_face_bounds, false, true);
        for (TopExp_Explorer right_faces(right.impl().shape, TopAbs_FACE); right_faces.More();
             right_faces.Next()) {
          Bnd_Box right_face_bounds;
          BRepBndLib::AddOptimal(right_faces.Current(), right_face_bounds, false, true);
          if (left_face_bounds.IsVoid() || right_face_bounds.IsVoid() ||
              left_face_bounds.IsOut(right_face_bounds)) {
            continue;
          }
          BRepAlgoAPI_Common face_common(left_faces.Current(), right_faces.Current());
          face_common.SetNonDestructive(true);
          face_common.Build();
          if (!face_common.IsDone() || face_common.HasErrors()) {
            result.diagnostic = "OCCT pair face-contact query failed";
            return result;
          }
          if (face_common.Shape().IsNull()) {
            continue;
          }
          GProp_GProps face_properties;
          BRepGProp::SurfaceProperties(face_common.Shape(), face_properties);
          const double area = face_properties.Mass();
          if (!std::isfinite(area) || area < 0.0) {
            result.diagnostic = "OCCT pair face-contact area is invalid";
            return result;
          }
          contact_area += area;
        }
      }
    }
    result.common_volume_mm3 = volume;
    result.common_contact_area_mm2 = contact_area;
    // Zero-volume common results still require the exact distance query.
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

struct NativeMeshResult::Impl {
  std::uint8_t status = STATUS_NULL_RESULT;
  std::string diagnostic = "Native tessellation did not produce a result";
  std::vector<NativeMeshVertex> vertices;
  std::vector<NativeMeshTriangle> triangles;
};

NativeMeshResult::NativeMeshResult(std::unique_ptr<Impl> impl) noexcept
    : impl_(std::move(impl)) {}
NativeMeshResult::~NativeMeshResult() = default;
NativeMeshResult::NativeMeshResult(NativeMeshResult&&) noexcept = default;
NativeMeshResult& NativeMeshResult::operator=(NativeMeshResult&&) noexcept = default;

std::uint8_t NativeMeshResult::mesh_status_code() const noexcept {
  return impl_ == nullptr ? STATUS_NULL_RESULT : impl_->status;
}

rust::String NativeMeshResult::mesh_diagnostic() const {
  return rust::String(impl_ == nullptr ? "Missing native tessellation" : impl_->diagnostic);
}

rust::Vec<NativeMeshVertex> NativeMeshResult::mesh_vertices() const {
  rust::Vec<NativeMeshVertex> output;
  if (impl_ != nullptr) {
    output.reserve(impl_->vertices.size());
    for (const NativeMeshVertex& vertex : impl_->vertices) {
      output.push_back(vertex);
    }
  }
  return output;
}

rust::Vec<NativeMeshTriangle> NativeMeshResult::mesh_triangles() const {
  rust::Vec<NativeMeshTriangle> output;
  if (impl_ != nullptr) {
    output.reserve(impl_->triangles.size());
    for (const NativeMeshTriangle& triangle : impl_->triangles) {
      output.push_back(triangle);
    }
  }
  return output;
}

namespace {

std::unique_ptr<NativeMeshResult> mesh_error(
    std::uint8_t status, std::string diagnostic) noexcept {
  auto impl = std::make_unique<NativeMeshResult::Impl>();
  impl->status = status;
  impl->diagnostic = std::move(diagnostic);
  return std::make_unique<NativeMeshResult>(std::move(impl));
}

} // namespace

std::unique_ptr<NativeMeshResult> tessellate_body_native(
    const NativeOperationResult& body, double deflection,
    double angular_deflection, std::uint32_t max_triangles) noexcept {
  try {
    if (!body.valid() || body.impl().shape.IsNull()) {
      return mesh_error(STATUS_INVALID_PARAMETER, "Exact body is unavailable or invalid");
    }
    if (!(deflection > 0.0) || !(angular_deflection > 0.0) || max_triangles == 0) {
      return mesh_error(STATUS_INVALID_PARAMETER, "Tessellation parameters are out of range");
    }
    TopoDS_Shape shape = body.impl().shape;
    BRepMesh_IncrementalMesh mesher(shape, deflection, Standard_False, angular_deflection, Standard_True);
    mesher.Perform();
    if (!mesher.IsDone()) {
      return mesh_error(STATUS_INVALID_SHAPE, "OCCT tessellation did not complete");
    }
    auto impl = std::make_unique<NativeMeshResult::Impl>();
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(shape, TopAbs_FACE, faces);
    for (Standard_Integer face_index = 1; face_index <= faces.Extent(); ++face_index) {
      const auto face_ordinal = static_cast<std::uint32_t>(face_index - 1);
      const TopoDS_Face face = TopoDS::Face(faces(face_index));
      TopLoc_Location location;
      const Handle(Poly_Triangulation) triangulation = BRep_Tool::Triangulation(face, location);
      if (triangulation.IsNull()) {
        continue;
      }
      const gp_Trsf transform = location.Transformation();
      const bool reversed = face.Orientation() == TopAbs_REVERSED;
      const std::uint32_t base_index = static_cast<std::uint32_t>(impl->vertices.size());
      if (impl->vertices.size() + static_cast<std::size_t>(triangulation->NbNodes()) >
          static_cast<std::size_t>(max_triangles) * 3) {
        return mesh_error(STATUS_INVALID_SHAPE, "Tessellation exceeds the bounded vertex budget");
      }
      for (Standard_Integer node = 1; node <= triangulation->NbNodes(); ++node) {
        gp_Pnt point = triangulation->Node(node);
        point.Transform(transform);
        impl->vertices.push_back(NativeMeshVertex{point.X(), point.Y(), point.Z()});
      }
      for (Standard_Integer index = 1; index <= triangulation->NbTriangles(); ++index) {
        if (impl->triangles.size() >= static_cast<std::size_t>(max_triangles)) {
          return mesh_error(STATUS_INVALID_SHAPE, "Tessellation exceeds the bounded triangle budget");
        }
        Standard_Integer first = 0;
        Standard_Integer second = 0;
        Standard_Integer third = 0;
        triangulation->Triangle(index).Get(first, second, third);
        if (reversed) {
          std::swap(second, third);
        }
        impl->triangles.push_back(NativeMeshTriangle{
            base_index + static_cast<std::uint32_t>(first - 1),
            base_index + static_cast<std::uint32_t>(second - 1),
            base_index + static_cast<std::uint32_t>(third - 1),
            face_ordinal});
      }
    }
    if (impl->triangles.empty()) {
      return mesh_error(STATUS_INVALID_SHAPE, "OCCT tessellation produced no triangles");
    }
    impl->status = STATUS_OK;
    impl->diagnostic.clear();
    return std::make_unique<NativeMeshResult>(std::move(impl));
  } catch (const Standard_Failure& failure) {
    return mesh_error(STATUS_BACKEND_EXCEPTION, standard_failure_message(failure));
  } catch (const std::exception& failure) {
    return mesh_error(STATUS_BACKEND_EXCEPTION, failure.what());
  } catch (...) {
    return mesh_error(STATUS_BACKEND_EXCEPTION, "Unknown native tessellation failure");
  }
}

struct NativeVolumeMeshResult::Impl {
  std::uint8_t status = STATUS_NULL_RESULT;
  std::string diagnostic = "Native volume meshing did not produce a result";
  std::vector<NativeMeshVertex> vertices;
  std::vector<NativeVolumeMeshTetrahedron> tetrahedra;
  std::vector<NativeMeshTriangle> boundary_triangles;
};

NativeVolumeMeshResult::NativeVolumeMeshResult(std::unique_ptr<Impl> impl) noexcept
    : impl_(std::move(impl)) {}
NativeVolumeMeshResult::~NativeVolumeMeshResult() = default;
NativeVolumeMeshResult::NativeVolumeMeshResult(NativeVolumeMeshResult&&) noexcept = default;
NativeVolumeMeshResult& NativeVolumeMeshResult::operator=(NativeVolumeMeshResult&&) noexcept = default;

std::uint8_t NativeVolumeMeshResult::volume_mesh_status_code() const noexcept {
  return impl_ == nullptr ? STATUS_NULL_RESULT : impl_->status;
}

rust::String NativeVolumeMeshResult::volume_mesh_diagnostic() const {
  return rust::String(impl_ == nullptr ? "Missing native volume mesh" : impl_->diagnostic);
}

rust::Vec<NativeMeshVertex> NativeVolumeMeshResult::volume_mesh_vertices() const {
  rust::Vec<NativeMeshVertex> output;
  if (impl_ != nullptr) {
    output.reserve(impl_->vertices.size());
    for (const NativeMeshVertex& vertex : impl_->vertices) {
      output.push_back(vertex);
    }
  }
  return output;
}

rust::Vec<NativeVolumeMeshTetrahedron>
NativeVolumeMeshResult::volume_mesh_tetrahedra() const {
  rust::Vec<NativeVolumeMeshTetrahedron> output;
  if (impl_ != nullptr) {
    output.reserve(impl_->tetrahedra.size());
    for (const NativeVolumeMeshTetrahedron& tetrahedron : impl_->tetrahedra) {
      output.push_back(tetrahedron);
    }
  }
  return output;
}

rust::Vec<NativeMeshTriangle>
NativeVolumeMeshResult::volume_mesh_boundary_triangles() const {
  rust::Vec<NativeMeshTriangle> output;
  if (impl_ != nullptr) {
    output.reserve(impl_->boundary_triangles.size());
    for (const NativeMeshTriangle& triangle : impl_->boundary_triangles) {
      output.push_back(triangle);
    }
  }
  return output;
}

namespace {

std::unique_ptr<NativeVolumeMeshResult> volume_mesh_error(
    std::uint8_t status, std::string diagnostic) noexcept {
  auto impl = std::make_unique<NativeVolumeMeshResult::Impl>();
  impl->status = status;
  impl->diagnostic = std::move(diagnostic);
  return std::make_unique<NativeVolumeMeshResult>(std::move(impl));
}

bool point_is_inside_or_on(const TopoDS_Shape& solid, const gp_Pnt& point,
                           double tolerance) {
  BRepClass3d_SolidClassifier classifier(solid, point, tolerance);
  return classifier.State() == TopAbs_IN || classifier.State() == TopAbs_ON;
}

bool point_is_strictly_inside(const TopoDS_Shape& solid, const gp_Pnt& point,
                              double tolerance) {
  BRepClass3d_SolidClassifier classifier(solid, point, tolerance);
  return classifier.State() == TopAbs_IN;
}

gp_Pnt interpolate_point(const gp_Pnt& first, const gp_Pnt& second, double fraction) {
  return gp_Pnt(first.X() + (second.X() - first.X()) * fraction,
                first.Y() + (second.Y() - first.Y()) * fraction,
                first.Z() + (second.Z() - first.Z()) * fraction);
}

double signed_six_volume(const gp_Pnt& first, const gp_Pnt& second,
                         const gp_Pnt& third, const gp_Pnt& fourth) {
  const gp_Vec ab(first, second);
  const gp_Vec ac(first, third);
  const gp_Vec ad(first, fourth);
  return ab.Dot(ac.Crossed(ad));
}

} // namespace

std::unique_ptr<NativeVolumeMeshResult> volume_mesh_body_native(
    const NativeOperationResult& body, double deflection,
    double angular_deflection, std::uint32_t max_tetrahedra) noexcept {
  try {
    if (!body.valid() || body.impl().shape.IsNull()) {
      return volume_mesh_error(STATUS_INVALID_PARAMETER,
                               "Exact body is unavailable or invalid");
    }
    if (!std::isfinite(deflection) || !std::isfinite(angular_deflection)
        || !(deflection > 0.0) || !(angular_deflection > 0.0)
        || max_tetrahedra < 4 || max_tetrahedra > 65536) {
      return volume_mesh_error(STATUS_INVALID_PARAMETER,
                               "Volume-mesh parameters are out of range");
    }
    if (count_subshapes(body.impl().shape, TopAbs_SOLID) != 1) {
      return volume_mesh_error(STATUS_INVALID_SHAPE,
                               "Volume meshing requires exactly one exact solid");
    }
    TopExp_Explorer solid_explorer(body.impl().shape, TopAbs_SOLID);
    if (!solid_explorer.More()) {
      return volume_mesh_error(STATUS_INVALID_SHAPE,
                               "Volume meshing found no exact solid");
    }
    const TopoDS_Solid solid = TopoDS::Solid(solid_explorer.Current());
    if (!BRepCheck_Analyzer(solid, Standard_True).IsValid()) {
      return volume_mesh_error(STATUS_INVALID_SHAPE,
                               "Volume meshing requires a valid closed solid");
    }

    const double classifier_tolerance = std::max(1.0e-9, deflection * 1.0e-7);
    GProp_GProps volume_properties;
    BRepGProp::VolumeProperties(solid, volume_properties);
    gp_Pnt interior = volume_properties.CentreOfMass();
    bool found_interior = point_is_strictly_inside(solid, interior, classifier_tolerance);
    if (!found_interior) {
      Bnd_Box bounds;
      BRepBndLib::Add(solid, bounds);
      if (bounds.IsVoid() || bounds.IsOpen()) {
        return volume_mesh_error(STATUS_INVALID_SHAPE,
                                 "Exact solid has no finite closed bounds");
      }
      double min_x = 0.0;
      double min_y = 0.0;
      double min_z = 0.0;
      double max_x = 0.0;
      double max_y = 0.0;
      double max_z = 0.0;
      bounds.Get(min_x, min_y, min_z, max_x, max_y, max_z);
      constexpr std::array<double, 7> fractions{
          0.5, 0.25, 0.75, 0.125, 0.375, 0.625, 0.875};
      for (double x_fraction : fractions) {
        for (double y_fraction : fractions) {
          for (double z_fraction : fractions) {
            const gp_Pnt candidate(
                min_x + (max_x - min_x) * x_fraction,
                min_y + (max_y - min_y) * y_fraction,
                min_z + (max_z - min_z) * z_fraction);
            if (point_is_strictly_inside(solid, candidate, classifier_tolerance)) {
              interior = candidate;
              found_interior = true;
              break;
            }
          }
          if (found_interior) {
            break;
          }
        }
        if (found_interior) {
          break;
        }
      }
    }
    if (!found_interior) {
      return volume_mesh_error(STATUS_INVALID_SHAPE,
                               "Could not find a verified point inside the exact solid");
    }

    TopoDS_Shape meshed_shape = body.impl().shape;
    BRepTools::Clean(meshed_shape);
    BRepMesh_IncrementalMesh mesher(
        meshed_shape, deflection, Standard_False, angular_deflection, Standard_True);
    mesher.Perform();
    if (!mesher.IsDone()) {
      return volume_mesh_error(STATUS_INVALID_SHAPE,
                               "OCCT boundary tessellation did not complete");
    }

    auto impl = std::make_unique<NativeVolumeMeshResult::Impl>();
    impl->vertices.push_back(
        NativeMeshVertex{interior.X(), interior.Y(), interior.Z()});
    const double merge_tolerance = std::max(1.0e-9, deflection * 1.0e-7);
    std::map<std::array<std::int64_t, 3>, std::uint32_t> vertex_indices;
    const auto append_boundary_vertex = [&](const gp_Pnt& point) {
      const std::array<std::int64_t, 3> key{
          static_cast<std::int64_t>(std::llround(point.X() / merge_tolerance)),
          static_cast<std::int64_t>(std::llround(point.Y() / merge_tolerance)),
          static_cast<std::int64_t>(std::llround(point.Z() / merge_tolerance))};
      const auto existing = vertex_indices.find(key);
      if (existing != vertex_indices.end()) {
        return existing->second;
      }
      const auto index = static_cast<std::uint32_t>(impl->vertices.size());
      impl->vertices.push_back(NativeMeshVertex{point.X(), point.Y(), point.Z()});
      vertex_indices.emplace(key, index);
      return index;
    };

    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(meshed_shape, TopAbs_FACE, faces);
    const double volume_epsilon =
        std::max(1.0e-15, std::abs(body.impl().summary.volume_mm3) * 1.0e-14);
    for (Standard_Integer face_index = 1; face_index <= faces.Extent(); ++face_index) {
      const auto face_ordinal = static_cast<std::uint32_t>(face_index - 1);
      const TopoDS_Face face = TopoDS::Face(faces(face_index));
      TopLoc_Location location;
      const Handle(Poly_Triangulation) triangulation =
          BRep_Tool::Triangulation(face, location);
      if (triangulation.IsNull() || triangulation->NbTriangles() == 0) {
        return volume_mesh_error(
            STATUS_INVALID_SHAPE,
            "An exact face has no boundary triangulation or provenance");
      }
      const gp_Trsf transform = location.Transformation();
      const bool reversed = face.Orientation() == TopAbs_REVERSED;
      for (Standard_Integer triangle_index = 1;
           triangle_index <= triangulation->NbTriangles(); ++triangle_index) {
        if (impl->tetrahedra.size() >= static_cast<std::size_t>(max_tetrahedra)) {
          return volume_mesh_error(
              STATUS_INVALID_SHAPE,
              "Volume mesh exceeds the bounded tetrahedron budget");
        }
        Standard_Integer first_node = 0;
        Standard_Integer second_node = 0;
        Standard_Integer third_node = 0;
        triangulation->Triangle(triangle_index).Get(
            first_node, second_node, third_node);
        if (reversed) {
          std::swap(second_node, third_node);
        }
        std::array<gp_Pnt, 3> points{
            triangulation->Node(first_node).Transformed(transform),
            triangulation->Node(second_node).Transformed(transform),
            triangulation->Node(third_node).Transformed(transform)};
        double six_volume =
            signed_six_volume(interior, points[0], points[1], points[2]);
        if (!std::isfinite(six_volume) || std::abs(six_volume) <= volume_epsilon) {
          return volume_mesh_error(
              STATUS_INVALID_SHAPE,
              "Boundary cone contains a degenerate tetrahedron");
        }
        if (six_volume < 0.0) {
          std::swap(points[1], points[2]);
          six_volume = -six_volume;
        }
        const gp_Pnt triangle_centroid(
            (points[0].X() + points[1].X() + points[2].X()) / 3.0,
            (points[0].Y() + points[1].Y() + points[2].Y()) / 3.0,
            (points[0].Z() + points[1].Z() + points[2].Z()) / 3.0);
        const std::array<gp_Pnt, 4> ray_targets{
            points[0], points[1], points[2], triangle_centroid};
        for (const gp_Pnt& target : ray_targets) {
          for (double fraction : std::array<double, 4>{0.25, 0.5, 0.75, 0.99}) {
            if (!point_is_inside_or_on(
                    solid, interpolate_point(interior, target, fraction),
                    classifier_tolerance)) {
              return volume_mesh_error(
                  STATUS_INVALID_SHAPE,
                  "Exact solid is not visibility-safe for the bounded cone mesher");
            }
          }
        }
        const std::uint32_t first = append_boundary_vertex(points[0]);
        const std::uint32_t second = append_boundary_vertex(points[1]);
        const std::uint32_t third = append_boundary_vertex(points[2]);
        if (first == second || first == third || second == third) {
          return volume_mesh_error(
              STATUS_INVALID_SHAPE,
              "Boundary tessellation contains a collapsed triangle");
        }
        impl->tetrahedra.push_back(
            NativeVolumeMeshTetrahedron{0, first, second, third});
        impl->boundary_triangles.push_back(
            NativeMeshTriangle{first, second, third, face_ordinal});
      }
    }
    if (impl->tetrahedra.empty()) {
      return volume_mesh_error(STATUS_INVALID_SHAPE,
                               "OCCT boundary produced no tetrahedra");
    }
    impl->status = STATUS_OK;
    impl->diagnostic.clear();
    return std::make_unique<NativeVolumeMeshResult>(std::move(impl));
  } catch (const Standard_Failure& failure) {
    return volume_mesh_error(STATUS_BACKEND_EXCEPTION,
                             standard_failure_message(failure));
  } catch (const std::exception& failure) {
    return volume_mesh_error(STATUS_BACKEND_EXCEPTION, failure.what());
  } catch (...) {
    return volume_mesh_error(STATUS_BACKEND_EXCEPTION,
                             "Unknown native volume-meshing failure");
  }
}

} // namespace ketchup::exact
