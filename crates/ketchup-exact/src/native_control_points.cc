// The control net of a single-face surface, and moving one of its control
// points: the face keeps its trimming curves in the surface's parameters and
// its edges and corners follow the reshaped surface.

#include "native_common.hxx"

#include <BRepBuilderAPI_NurbsConvert.hxx>
#include <BRepTools_Modification.hxx>
#include <BRepTools_Modifier.hxx>
#include <Geom2d_Curve.hxx>
#include <Geom_BSplineSurface.hxx>

#include <utility>

namespace ketchup::exact {

namespace {

// A single-face surface as one B-spline face, or null and the reason.
struct NurbsFace {
  TopoDS_Face face;
  occ::handle<Geom_BSplineSurface> surface;
  std::string failure;
};

NurbsFace nurbs_face(const NativeOperationResult& body) {
  NurbsFace result;
  const TopoDS_Face face = single_surface_face(body);
  if (face.IsNull()) {
    result.failure = "Control points belong to a single-face surface";
    return result;
  }
  BRepBuilderAPI_NurbsConvert conversion(face, true);
  if (!conversion.IsDone()) {
    result.failure = "The surface has no B-spline form";
    return result;
  }
  TopExp_Explorer faces(conversion.Shape(), TopAbs_FACE);
  if (!faces.More()) {
    result.failure = "The surface has no B-spline form";
    return result;
  }
  const TopoDS_Face converted = TopoDS::Face(faces.Current());
  TopLoc_Location location;
  const occ::handle<Geom_BSplineSurface> surface =
      occ::handle<Geom_BSplineSurface>::DownCast(BRep_Tool::Surface(converted, location));
  if (surface.IsNull() || !location.IsIdentity()) {
    result.failure = "The surface has no B-spline form";
    return result;
  }
  // A seam edge would need both of its curves on the surface moved together.
  for (TopExp_Explorer edges(converted, TopAbs_EDGE); edges.More(); edges.Next()) {
    const TopoDS_Edge& edge = TopoDS::Edge(edges.Current());
    if (BRep_Tool::IsClosed(edge, converted) || BRep_Tool::Degenerated(edge)) {
      result.failure = "Control points move on surfaces without seam or collapsed edges";
      return result;
    }
  }
  result.face = converted;
  result.surface = surface;
  return result;
}

// Puts one face on a new surface with the same parameters: every edge keeps
// its curve in those parameters, its 3D curve is dropped (and rebuilt from
// them), and every corner moves to where the new surface puts it.
class ReshapedSurface : public BRepTools_Modification {
 public:
  ReshapedSurface(const TopoDS_Face& face, const occ::handle<Geom_BSplineSurface>& surface)
      : face_(face), surface_(surface) {
    for (TopExp_Explorer edges(face, TopAbs_EDGE); edges.More(); edges.Next()) {
      const TopoDS_Edge& edge = TopoDS::Edge(edges.Current());
      double first = 0.0;
      double last = 0.0;
      const occ::handle<Geom2d_Curve> curve = BRep_Tool::CurveOnSurface(edge, face, first, last);
      if (curve.IsNull()) {
        continue;
      }
      for (TopoDS_Iterator vertices(edge); vertices.More(); vertices.Next()) {
        const TopoDS_Vertex& vertex = TopoDS::Vertex(vertices.Value());
        const gp_Pnt2d at = curve->Value(BRep_Tool::Parameter(vertex, edge, face));
        corners_.emplace_back(vertex, surface->Value(at.X(), at.Y()));
      }
    }
  }

  bool NewSurface(const TopoDS_Face& face, occ::handle<Geom_Surface>& surface,
                  TopLoc_Location& location, double& tolerance, bool& reverse_wires,
                  bool& reverse_face) override {
    if (!face.IsSame(face_)) {
      return false;
    }
    surface = surface_;
    location = TopLoc_Location();
    tolerance = BRep_Tool::Tolerance(face);
    reverse_wires = false;
    reverse_face = false;
    return true;
  }

  bool NewCurve(const TopoDS_Edge& edge, occ::handle<Geom_Curve>& curve,
                TopLoc_Location& location, double& tolerance) override {
    curve.Nullify();
    location = TopLoc_Location();
    tolerance = BRep_Tool::Tolerance(edge);
    return true;
  }

  bool NewPoint(const TopoDS_Vertex& vertex, gp_Pnt& point, double& tolerance) override {
    for (const auto& [corner, moved] : corners_) {
      if (corner.IsSame(vertex)) {
        point = moved;
        tolerance = BRep_Tool::Tolerance(vertex);
        return true;
      }
    }
    return false;
  }

  bool NewCurve2d(const TopoDS_Edge& edge, const TopoDS_Face& face, const TopoDS_Edge&,
                  const TopoDS_Face&, occ::handle<Geom2d_Curve>& curve,
                  double& tolerance) override {
    double first = 0.0;
    double last = 0.0;
    curve = BRep_Tool::CurveOnSurface(edge, face, first, last);
    tolerance = BRep_Tool::Tolerance(edge);
    return !curve.IsNull();
  }

  bool NewParameter(const TopoDS_Vertex&, const TopoDS_Edge&, double&, double&) override {
    return false;
  }

  GeomAbs_Shape Continuity(const TopoDS_Edge&, const TopoDS_Face&, const TopoDS_Face&,
                           const TopoDS_Edge&, const TopoDS_Face&, const TopoDS_Face&) override {
    return GeomAbs_C0;
  }

  DEFINE_STANDARD_RTTI_INLINE(ReshapedSurface, BRepTools_Modification)

 private:
  TopoDS_Face face_;
  occ::handle<Geom_BSplineSurface> surface_;
  std::vector<std::pair<TopoDS_Vertex, gp_Pnt>> corners_;
};

}  // namespace

NativeControlNet surface_control_net_native(const NativeOperationResult& body) noexcept {
  NativeControlNet result{};
  result.status = STATUS_INVALID_SHAPE;
  try {
    const NurbsFace nurbs = nurbs_face(body);
    if (nurbs.surface.IsNull()) {
      result.status = STATUS_INVALID_PARAMETER;
      result.diagnostic = nurbs.failure;
      return result;
    }
    const occ::handle<Geom_BSplineSurface>& surface = nurbs.surface;
    result.u_count = static_cast<std::uint32_t>(surface->NbUPoles());
    result.v_count = static_cast<std::uint32_t>(surface->NbVPoles());
    for (int u = 1; u <= surface->NbUPoles(); ++u) {
      for (int v = 1; v <= surface->NbVPoles(); ++v) {
        const gp_Pnt pole = surface->Pole(u, v);
        for (const double value : {pole.X(), pole.Y(), pole.Z(), surface->Weight(u, v)}) {
          result.values.push_back(value);
        }
      }
    }
    result.status = STATUS_OK;
  } catch (const Standard_Failure& error) {
    result.status = STATUS_BACKEND_EXCEPTION;
    result.diagnostic = standard_failure_message(error);
  } catch (const std::exception& error) {
    result.status = STATUS_BACKEND_EXCEPTION;
    result.diagnostic = error.what();
  }
  return result;
}

std::unique_ptr<NativeOperationResult> move_surface_control_point_native(
    const NativeOperationResult& body, std::uint32_t u_index, std::uint32_t v_index,
    NativePoint offset, double weight) noexcept {
  return guarded([&] {
    const NurbsFace nurbs = nurbs_face(body);
    if (nurbs.surface.IsNull()) {
      return error_result(STATUS_INVALID_PARAMETER, nurbs.failure);
    }
    const occ::handle<Geom_BSplineSurface>& original = nurbs.surface;
    if (u_index >= static_cast<std::uint32_t>(original->NbUPoles())
        || v_index >= static_cast<std::uint32_t>(original->NbVPoles())) {
      std::ostringstream diagnostic;
      diagnostic << "Control point (" << u_index << ", " << v_index
                 << ") is outside the surface's " << original->NbUPoles() << " x "
                 << original->NbVPoles() << " control net";
      return error_result(STATUS_INVALID_PARAMETER, diagnostic.str());
    }
    const int u = static_cast<int>(u_index) + 1;
    const int v = static_cast<int>(v_index) + 1;
    const occ::handle<Geom_BSplineSurface> surface =
        occ::handle<Geom_BSplineSurface>::DownCast(original->Copy());
    const gp_Pnt pole = original->Pole(u, v).Translated(gp_Vec(offset.x, offset.y, offset.z));
    // A weight of zero keeps the current one.
    surface->SetPole(u, v, pole, weight > 0.0 ? weight : original->Weight(u, v));

    const occ::handle<ReshapedSurface> reshape = new ReshapedSurface(nurbs.face, surface);
    BRepTools_Modifier modifier(nurbs.face, reshape);
    if (!modifier.IsDone()) {
      return error_result(STATUS_INVALID_SHAPE, "The face does not follow its moved surface");
    }
    TopoDS_Shape result = modifier.ModifiedShape(nurbs.face);
    if (result.IsNull() || !BRepLib::BuildCurves3d(result, tolerances().linear_mm)) {
      return error_result(STATUS_INVALID_SHAPE, "The edges of the moved surface do not rebuild");
    }
    BRepLib::UpdateTolerances(result);
    if (!BRepCheck_Analyzer(result).IsValid() || count_subshapes(result, TopAbs_FACE) != 1) {
      return error_result(STATUS_INVALID_SHAPE, "The moved surface is not one valid bounded face");
    }
    const ShapeIndex result_index(result);
    std::vector<HistoryRecord> history;
    history.push_back(history_record(
        "surface_control_point.face", "modified", "surface.face", result_index, result));
    return success_result(result, std::move(history), false, false, {}, true);
  });
}

}  // namespace ketchup::exact
