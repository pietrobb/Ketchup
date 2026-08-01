#include <BRepCheck_Analyzer.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <TopoDS_Shape.hxx>

int main()
{
  const TopoDS_Shape aShape = BRepPrimAPI_MakeBox(10.0, 20.0, 30.0).Shape();
  return BRepCheck_Analyzer(aShape).IsValid() ? 0 : 1;
}
