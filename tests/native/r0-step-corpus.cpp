#include <BRepAlgoAPI_Cut.hxx>
#include <BRepAlgoAPI_Fuse.hxx>
#include <BRepCheck_Analyzer.hxx>
#include <BRepGProp.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <GProp_GProps.hxx>
#include <IFSelect_ReturnStatus.hxx>
#include <Interface_Static.hxx>
#include <STEPControl_StepModelType.hxx>
#include <STEPControl_Writer.hxx>
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
      !write_shape(fuse.Shape(), output_dir / "self-authored-l-bracket.step", 6000.0)) {
    return 5;
  }

  return 0;
}
