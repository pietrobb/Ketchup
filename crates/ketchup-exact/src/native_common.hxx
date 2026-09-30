#pragma once

// Declarations shared by the native translation units: result construction,
// evidence, history records, status codes and the exception guard.

#include "ketchup_exact.hxx"
#include "ketchup-exact/src/lib.rs.h"

#include <BRepAlgoAPI_Common.hxx>
#include <BRepAlgoAPI_Cut.hxx>
#include <BRepAlgoAPI_Fuse.hxx>
#include <BOPAlgo_Splitter.hxx>
#include <BRepAdaptor_Curve.hxx>
#include <BRepAdaptor_Surface.hxx>
#include <Precision.hxx>
#include <BRepBndLib.hxx>
#include <BRepBuilderAPI_FindPlane.hxx>
#include <GeomLib_IsPlanarSurface.hxx>
#include <Geom_RectangularTrimmedSurface.hxx>
#include <BRepBuilderAPI_MakeEdge.hxx>
#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepBuilderAPI_MakeVertex.hxx>
#include <BRepBuilderAPI_MakeSolid.hxx>
#include <BRepBuilderAPI_MakeWire.hxx>
#include <BRepBuilderAPI_Sewing.hxx>
#include <BRepBuilderAPI_GTransform.hxx>
#include <BRepBuilderAPI_Transform.hxx>
#include <BRepCheck_Analyzer.hxx>
#include <BRepClass_FaceClassifier.hxx>
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

inline constexpr std::uint8_t STATUS_OK = 0;
inline constexpr std::uint8_t STATUS_INVALID_PARAMETER = 1;
inline constexpr std::uint8_t STATUS_NON_FINITE_PARAMETER = 2;
inline constexpr std::uint8_t STATUS_NO_GEOMETRIC_CHANGE = 3;
inline constexpr std::uint8_t STATUS_DEGENERATE_OPERATION = 4;
inline constexpr std::uint8_t STATUS_INVALID_SHAPE = 5;
inline constexpr std::uint8_t STATUS_BACKEND_EXCEPTION = 6;
inline constexpr std::uint8_t STATUS_NULL_RESULT = 7;

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

const NativeTolerances& tolerances() noexcept;

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

std::uint32_t count_subshapes(const TopoDS_Shape& shape, TopAbs_ShapeEnum kind);

TopoDS_Face face_at_ordinal(const TopoDS_Shape& shape, std::uint32_t target);

TopoDS_Edge edge_at_ordinal(const TopoDS_Shape& shape, std::uint32_t target);

std::pair<std::uint32_t, bool> face_ordinal(
    const TopoDS_Shape& result, const TopoDS_Shape& candidate);

HistoryRecord history_record(
    std::string role,
    std::string relation,
    std::string source,
    const TopoDS_Shape& result,
    const TopoDS_Shape& output);

EdgeHistoryRecord edge_history_record(
    std::string role,
    std::string relation,
    std::string source,
    const TopoDS_Shape& result,
    const TopoDS_Shape& output);

std::string standard_failure_message(const Standard_Failure& failure);

std::unique_ptr<NativeOperationResult> error_result(
    std::uint8_t status, std::string diagnostic) noexcept;

bool oriented_planar_face_normal(const TopoDS_Face& face, gp_Dir& normal);

NativeFaceEvidence inspect_face(const TopoDS_Face& face, std::uint32_t ordinal);

NativeEdgeEvidence inspect_edge(const TopoDS_Edge& edge, std::uint32_t ordinal);

std::unique_ptr<NativeOperationResult> success_result(
    TopoDS_Shape shape,
    std::vector<HistoryRecord> history,
    bool allow_multi_solid = false,
    bool allow_planar_face = false,
    std::vector<EdgeHistoryRecord> edge_history = {},
    bool allow_surface = false);

TopoDS_Edge cubic_bezier_edge(
    rust::Slice<const double> segments, std::size_t offset, double z);

std::unique_ptr<NativeOperationResult> sweep_spatial_profile_native_impl(
    rust::Slice<const double> profile_segments,
    rust::Slice<const double> path_segments) noexcept;

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
} // namespace ketchup::exact
