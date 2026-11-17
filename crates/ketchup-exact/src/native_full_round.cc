#include "native_common.hxx"

#include <BRepBuilderAPI_MakePolygon.hxx>
#include <BRepPrimAPI_MakeSphere.hxx>

#include <algorithm>
#include <cmath>

namespace ketchup::exact {

// Rounding every edge of a convex polyhedron by `radius` is its opening by a
// ball: move every face plane inward by the radius, then grow what is left by
// the radius. When the rounds consume whole faces (the radius is half the
// body's thickness) OCCT's rolling-ball fillet has no face left to run on, yet
// what is left after the move is a polygon, a segment or a point, and growing
// it by a ball is a fuse of exact primitives: a slab over the polygon, a
// cylinder along each edge and a sphere at each corner. Returns a null shape
// for any other body, so the caller keeps OCCT's refusal.
TopoDS_Shape full_round_of_convex_polyhedron(const TopoDS_Shape& body, double radius) {
  constexpr std::size_t MAX_PLANES = 64;
  constexpr double HALF_PI = 3.14159265358979323846 / 2.0;
  const double tolerance = tolerances().linear_mm;
  std::vector<gp_XYZ> normals;
  std::vector<double> offsets;
  for (TopExp_Explorer explorer(body, TopAbs_FACE); explorer.More(); explorer.Next()) {
    const TopoDS_Face face = TopoDS::Face(explorer.Current());
    gp_Dir normal;
    TopExp_Explorer vertex(face, TopAbs_VERTEX);
    if (normals.size() == MAX_PLANES || !oriented_planar_face_normal(face, normal)
        || !vertex.More()) {
      return {};
    }
    const gp_XYZ point = BRep_Tool::Pnt(TopoDS::Vertex(vertex.Current())).XYZ();
    normals.push_back(normal.XYZ());
    offsets.push_back(normal.XYZ().Dot(point) - radius);
  }
  for (TopExp_Explorer explorer(body, TopAbs_VERTEX); explorer.More(); explorer.Next()) {
    const gp_XYZ point = BRep_Tool::Pnt(TopoDS::Vertex(explorer.Current())).XYZ();
    for (std::size_t plane = 0; plane < normals.size(); ++plane) {
      if (normals[plane].Dot(point) > offsets[plane] + radius + tolerance) {
        return {};  // not convex
      }
    }
  }
  // The corners of what is left: points where three moved planes meet that
  // lie inside all of them.
  std::vector<gp_XYZ> corners;
  for (std::size_t i = 0; i < normals.size(); ++i) {
    for (std::size_t j = i + 1; j < normals.size(); ++j) {
      for (std::size_t k = j + 1; k < normals.size(); ++k) {
        const gp_XYZ jk = normals[j].Crossed(normals[k]);
        const double determinant = normals[i].Dot(jk);
        if (std::abs(determinant) <= tolerances().angular_rad) continue;
        const gp_XYZ corner =
            (jk * offsets[i] + normals[k].Crossed(normals[i]) * offsets[j]
             + normals[i].Crossed(normals[j]) * offsets[k]) / determinant;
        bool inside = true;
        for (std::size_t plane = 0; plane < normals.size() && inside; ++plane) {
          inside = normals[plane].Dot(corner) <= offsets[plane] + tolerance;
        }
        const bool known = std::any_of(corners.begin(), corners.end(), [&](const gp_XYZ& other) {
          return (other - corner).Modulus() <= tolerance;
        });
        if (inside && !known) corners.push_back(corner);
      }
    }
  }
  if (corners.empty()) {
    return {};  // the radius is more than half the body's thickness
  }
  // A body left whole by the move is one OCCT's fillet makes; it failed for
  // another reason, so this case does not apply.
  const gp_XYZ origin = corners.front();
  gp_XYZ along;
  for (const gp_XYZ& corner : corners) {
    if ((corner - origin).Modulus() > along.Modulus()) along = corner - origin;
  }
  gp_XYZ across;
  if (along.Modulus() > tolerance) {
    const gp_XYZ direction = along.Normalized();
    for (const gp_XYZ& corner : corners) {
      const gp_XYZ offset = corner - origin;
      const gp_XYZ off_line = offset - direction * offset.Dot(direction);
      if (off_line.Modulus() > across.Modulus()) across = off_line;
    }
  }
  const bool flat = across.Modulus() > tolerance;
  const gp_XYZ flat_normal = flat ? along.Crossed(across).Normalized() : gp_XYZ();
  if (flat) {
    for (const gp_XYZ& corner : corners) {
      if (std::abs((corner - origin).Dot(flat_normal)) > tolerance) return {};
    }
  }

  // The pieces meet face to face, never tangent along a seam, so the fuse
  // below only merges coincident faces.
  NCollection_List<TopoDS_Shape> parts;
  if (flat) {
    // Order the polygon's corners around its centre.
    gp_XYZ centre;
    for (const gp_XYZ& corner : corners) centre += corner;
    centre /= static_cast<double>(corners.size());
    const gp_XYZ u = along.Normalized();
    const gp_XYZ v = flat_normal.Crossed(u);
    std::sort(corners.begin(), corners.end(), [&](const gp_XYZ& a, const gp_XYZ& b) {
      return std::atan2((a - centre).Dot(v), (a - centre).Dot(u))
          < std::atan2((b - centre).Dot(v), (b - centre).Dot(u));
    });
    BRepBuilderAPI_MakePolygon polygon;
    for (const gp_XYZ& corner : corners) {
      polygon.Add(gp_Pnt(corner - flat_normal * radius));
    }
    polygon.Close();
    if (!polygon.IsDone()) return {};
    BRepBuilderAPI_MakeFace base(polygon.Wire(), true);
    if (!base.IsDone()) return {};
    BRepPrimAPI_MakePrism slab(base.Face(), gp_Vec(flat_normal * (2.0 * radius)));
    if (!slab.IsDone()) return {};
    parts.Append(slab.Shape());
    // Along each side, half a cylinder on the slab's side face; at each
    // corner, the half disc between two sides turned about the polygon's
    // normal through the sides' outward directions.
    const std::size_t count = corners.size();
    std::vector<gp_XYZ> outward(count);
    std::vector<TopoDS_Face> half_discs(count);
    for (std::size_t index = 0; index < count; ++index) {
      const gp_XYZ& start = corners[index];
      const gp_XYZ& end = corners[(index + 1) % count];
      const gp_XYZ direction = (end - start).Normalized();
      outward[index] = direction.Crossed(flat_normal);
      if (outward[index].Dot(start - centre) < 0.0) outward[index].Reverse();
      const gp_Ax2 frame{gp_Pnt(start), gp_Dir(direction), gp_Dir(outward[index])};
      const occ::handle<Geom_TrimmedCurve> arc = GC_MakeArcOfCircle(
          gp_Pnt(start - flat_normal * radius), gp_Pnt(start + outward[index] * radius),
          gp_Pnt(start + flat_normal * radius));
      BRepBuilderAPI_MakeWire wire(
          BRepBuilderAPI_MakeEdge(arc).Edge(),
          BRepBuilderAPI_MakeEdge(
              gp_Pnt(start + flat_normal * radius), gp_Pnt(start - flat_normal * radius)).Edge());
      if (!wire.IsDone()) return {};
      BRepBuilderAPI_MakeFace disc(gp_Pln(gp_Ax3(frame)), wire.Wire(), true);
      if (!disc.IsDone()) return {};
      half_discs[index] = disc.Face();
      BRepPrimAPI_MakePrism side(disc.Face(), gp_Vec(end - start));
      if (!side.IsDone()) return {};
      parts.Append(side.Shape());
    }
    for (std::size_t index = 0; index < count; ++index) {
      const std::size_t previous = (index + count - 1) % count;
      // The previous side's half disc, moved to this corner.
      gp_Trsf to_corner;
      to_corner.SetTranslation(gp_Vec(corners[index] - corners[previous]));
      const TopoDS_Shape disc = BRepBuilderAPI_Transform(half_discs[previous], to_corner, true).Shape();
      const double angle = std::atan2(
          outward[previous].Crossed(outward[index]).Dot(flat_normal),
          outward[previous].Dot(outward[index]));
      const gp_Dir axis(angle < 0.0 ? flat_normal.Reversed() : flat_normal);
      BRepPrimAPI_MakeRevol wedge(disc, gp_Ax1(gp_Pnt(corners[index]), axis), std::abs(angle));
      if (!wedge.IsDone()) return {};
      parts.Append(wedge.Shape());
    }
  } else if (along.Modulus() > tolerance) {
    // A segment: a cylinder capped by two half balls.
    const gp_XYZ direction = along.Normalized();
    const auto by_position = [&](const gp_XYZ& a, const gp_XYZ& b) {
      return a.Dot(direction) < b.Dot(direction);
    };
    const gp_XYZ start = *std::min_element(corners.begin(), corners.end(), by_position);
    const gp_XYZ end = *std::max_element(corners.begin(), corners.end(), by_position);
    parts.Append(BRepPrimAPI_MakeCylinder(
        gp_Ax2(gp_Pnt(start), gp_Dir(direction)), radius, (end - start).Modulus()).Shape());
    parts.Append(BRepPrimAPI_MakeSphere(
        gp_Ax2(gp_Pnt(end), gp_Dir(direction)), radius, 0.0, HALF_PI).Shape());
    parts.Append(BRepPrimAPI_MakeSphere(
        gp_Ax2(gp_Pnt(start), gp_Dir(direction.Reversed())), radius, 0.0, HALF_PI).Shape());
  } else {
    parts.Append(BRepPrimAPI_MakeSphere(gp_Pnt(origin), radius).Shape());
  }
  TopoDS_Shape rounded = parts.First();
  if (parts.Size() > 1) {
    NCollection_List<TopoDS_Shape> arguments;
    arguments.Append(parts.First());
    parts.RemoveFirst();
    BRepAlgoAPI_Fuse fuse;
    fuse.SetArguments(arguments);
    fuse.SetTools(parts);
    fuse.Build();
    if (!fuse.IsDone() || fuse.HasErrors()) return {};
    fuse.SimplifyResult(true, true);
    rounded = fuse.Shape();
  }
  TopExp_Explorer solid(rounded, TopAbs_SOLID);
  if (!solid.More() || count_subshapes(rounded, TopAbs_SOLID) != 1
      || !BRepCheck_Analyzer(solid.Current()).IsValid()) {
    return {};
  }
  return solid.Current();
}

} // namespace ketchup::exact
