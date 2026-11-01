#include <BRepAlgoAPI_Cut.hxx>
#include <BRepAlgoAPI_Fuse.hxx>
#include <BRepBuilderAPI_NurbsConvert.hxx>
#include <BRepCheck_Analyzer.hxx>
#include <BRepTools.hxx>
#include <BRep_Builder.hxx>
#include <BRep_Tool.hxx>
#include <Geom2d_Curve.hxx>
#include <Geom_BSplineSurface.hxx>
#include <BRepGProp.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <GProp_GProps.hxx>
#include <IFSelect_ReturnStatus.hxx>
#include <Interface_Static.hxx>
#include <STEPControl_StepModelType.hxx>
#include <STEPControl_Writer.hxx>
#include <TopExp_Explorer.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Shape.hxx>
#include <gp_Pnt.hxx>

#include <cmath>
#include <filesystem>
#include <iostream>
#include <string>

namespace {

bool write_shape(const TopoDS_Shape& shape, const std::filesystem::path& path, double expected_volume)
{
  if (shape.IsNull() || !BRepCheck_Analyzer(shape).IsValid()) {
    std::cerr << "Invalid generated shape: " << path.string() << '\n';
    return false;
  }

  GProp_GProps properties;
  BRepGProp::VolumeProperties(shape, properties);
  if (std::abs(properties.Mass() - expected_volume) > 1.0e-7) {
    std::cerr << "Unexpected volume for " << path.string() << ": " << properties.Mass() << '\n';
    return false;
  }

  STEPControl_Writer writer;
  if (writer.Transfer(shape, STEPControl_AsIs) != IFSelect_RetDone) {
    std::cerr << "STEP transfer failed: " << path.string() << '\n';
    return false;
  }
  if (writer.Write(path.string().c_str()) != IFSelect_RetDone) {
    std::cerr << "STEP write failed: " << path.string() << '\n';
    return false;
  }
  return true;
}

// A 100 x 60 x 40 box converted to B-spline faces, whose top surface is then
// narrowed by one millionth of its parameter range while its pcurves stay, so
// they reach just past the surface domain, as in some third-party exports.
bool write_bspline_pcurve_overshoot(const std::filesystem::path& path)
{
  TopoDS_Shape box = BRepBuilderAPI_NurbsConvert(BRepPrimAPI_MakeBox(100.0, 60.0, 40.0).Shape(), Standard_True).Shape();
  TopoDS_Face top;
  double highest = -1.0;
  for (TopExp_Explorer it(box, TopAbs_FACE); it.More(); it.Next()) {
    const TopoDS_Face face = TopoDS::Face(it.Current());
    double u0, u1, v0, v1;
    BRepTools::UVBounds(face, u0, u1, v0, v1);
    const gp_Pnt middle = BRep_Tool::Surface(face)->Value((u0 + u1) / 2.0, (v0 + v1) / 2.0);
    if (middle.Z() > highest) {
      highest = middle.Z();
      top = face;
    }
  }
  TopLoc_Location location;
  const auto surface = occ::down_cast<Geom_BSplineSurface>(BRep_Tool::Surface(top, location));
  if (top.IsNull() || surface.IsNull()) {
    std::cerr << "No B-spline top face for " << path.string() << '\n';
    return false;
  }
  double u0, u1, v0, v1;
  surface->Bounds(u0, u1, v0, v1);
  const auto narrowed = occ::down_cast<Geom_BSplineSurface>(surface->Copy());
  const double du = (u1 - u0) * 1.0e-6;
  const double dv = (v1 - v0) * 1.0e-6;
  narrowed->Segment(u0 + du, u1 - du, v0 + dv, v1 - dv);
  BRep_Builder builder;
  for (TopExp_Explorer it(top, TopAbs_EDGE); it.More(); it.Next()) {
    const TopoDS_Edge edge = TopoDS::Edge(it.Current());
    double first, last;
    const auto pcurve = BRep_Tool::CurveOnSurface(edge, top, first, last);
    builder.UpdateEdge(edge, pcurve, narrowed, location, 1.0e-3);
  }
  builder.UpdateFace(top, narrowed, location, 1.0e-3);

  double face_u0, face_u1, face_v0, face_v1;
  BRepTools::UVBounds(top, face_u0, face_u1, face_v0, face_v1);
  if (!(face_u0 < u0 + du && face_u1 > u1 - du)) {
    std::cerr << "Top face pcurves stay inside the domain for " << path.string() << '\n';
    return false;
  }
  GProp_GProps properties;
  BRepGProp::VolumeProperties(box, properties);
  if (std::abs(properties.Mass() - 240000.0) > 1.0e-3 * 240000.0) {
    std::cerr << "Unexpected volume for " << path.string() << ": " << properties.Mass() << '\n';
    return false;
  }
  STEPControl_Writer writer;
  return writer.Transfer(box, STEPControl_AsIs) == IFSelect_RetDone
      && writer.Write(path.string().c_str()) == IFSelect_RetDone;
}

} // namespace

int main(int argc, char** argv)
{
  if (argc != 2) {
    std::cerr << "Usage: r0-step-corpus <output-directory>\n";
    return 2;
  }

  const std::filesystem::path output_dir(argv[1]);
  std::filesystem::create_directories(output_dir);
  Interface_Static::SetCVal("write.step.schema", "AP214IS");

  const TopoDS_Shape box = BRepPrimAPI_MakeBox(10.0, 20.0, 30.0).Shape();

  BRepAlgoAPI_Cut cut(
      BRepPrimAPI_MakeBox(40.0, 30.0, 10.0).Shape(),
      BRepPrimAPI_MakeBox(gp_Pnt(10.0, 10.0, -5.0), 20.0, 10.0, 20.0).Shape());
  cut.Build();
  if (!cut.IsDone()) {
    std::cerr << "Through-cut fixture construction failed\n";
    return 3;
  }

  BRepAlgoAPI_Fuse fuse(
      BRepPrimAPI_MakeBox(40.0, 10.0, 10.0).Shape(),
      BRepPrimAPI_MakeBox(10.0, 30.0, 10.0).Shape());
  fuse.Build();
  if (!fuse.IsDone()) {
    std::cerr << "L-bracket fixture construction failed\n";
    return 4;
  }

  if (!write_shape(box, output_dir / "self-authored-box.step", 6000.0) ||
      !write_shape(cut.Shape(), output_dir / "self-authored-through-cut.step", 10000.0) ||
      !write_shape(fuse.Shape(), output_dir / "self-authored-l-bracket.step", 6000.0) ||
      !write_bspline_pcurve_overshoot(output_dir / "self-authored-bspline-pcurve-overshoot.step")) {
    return 5;
  }

  return 0;
}
