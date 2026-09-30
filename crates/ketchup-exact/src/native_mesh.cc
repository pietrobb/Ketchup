#include "native_common.hxx"

namespace ketchup::exact {

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

    const double classifier_tolerance = std::max(tolerances().rounding, deflection * tolerances().linear_mm);
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
    const double merge_tolerance = std::max(tolerances().rounding, deflection * tolerances().linear_mm);
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
        tolerances().negligible * std::max(1.0, std::abs(body.impl().summary.volume_mm3));
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

// Program-named topology (prototype). Every face of a named result carries its
// program name as source_element_id. Names never come from OCCT ordinals or
// fingerprints: base solids name faces by the profile segment that swept them,
// body operations carry the caller's per-face labels through OCCT history, and
// faces a finish creates are tagged with the selected edge index.

} // namespace ketchup::exact
