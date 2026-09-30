use super::*;

impl ExactBackend {
    /// Tessellate an exact body into a display mesh.
    ///
    /// The mesh is derived, never canonical: it exists so an exact body that
    /// carries no analytic render geometry — an imported STEP part — is still
    /// visible and pickable. Deflections are supplied by the caller so the
    /// same body always yields the same triangles.
    pub fn tessellate_body(
        &self,
        body: &ExactBody,
        deflection: f64,
        angular_deflection: f64,
        max_triangles: u32,
    ) -> Result<ExactTessellation, GeometryError> {
        let input = format!(
            "tessellate_body:{}:{}:{}:{max_triangles}",
            body.result_fingerprint,
            deflection.to_bits(),
            angular_deflection.to_bits()
        );
        let input_digest = stable_digest(&input);
        let native = body.native.as_ref().ok_or_else(|| GeometryError {
            code: GeometryErrorCode::NullResult,
            diagnostic: "Exact body lost its owned native shape".to_owned(),
            operation: "tessellate_body",
            input_digest: input_digest.clone(),
            backend_fingerprint: BACKEND_FINGERPRINT,
        })?;
        let mesh =
            ffi::tessellate_body_native(native, deflection, angular_deflection, max_triangles);
        let mesh = mesh.as_ref().ok_or_else(|| GeometryError {
            code: GeometryErrorCode::NullResult,
            diagnostic: "Native facade returned no tessellation object".to_owned(),
            operation: "tessellate_body",
            input_digest: input_digest.clone(),
            backend_fingerprint: BACKEND_FINGERPRINT,
        })?;
        if mesh.mesh_status_code() != 0 {
            return Err(GeometryError {
                code: native_status(mesh.mesh_status_code()),
                diagnostic: mesh.mesh_diagnostic(),
                operation: "tessellate_body",
                input_digest,
                backend_fingerprint: BACKEND_FINGERPRINT,
            });
        }
        let vertices_mm = mesh
            .mesh_vertices()
            .into_iter()
            .map(|vertex| [vertex.x_mm, vertex.y_mm, vertex.z_mm])
            .collect::<Vec<_>>();
        let triangles = mesh
            .mesh_triangles()
            .into_iter()
            .map(|triangle| ExactMeshTriangle {
                vertex_indices: [triangle.first, triangle.second, triangle.third],
                face_ordinal: triangle.face_ordinal,
            })
            .collect::<Vec<_>>();
        let vertex_count = vertices_mm.len();
        if triangles.is_empty()
            || vertices_mm
                .iter()
                .flatten()
                .any(|coordinate| !coordinate.is_finite())
            || triangles.iter().any(|triangle| {
                triangle
                    .vertex_indices
                    .iter()
                    .any(|index| *index as usize >= vertex_count)
            })
        {
            return Err(GeometryError {
                code: GeometryErrorCode::InvalidShape,
                diagnostic: "Native tessellation is not a well-formed indexed mesh".to_owned(),
                operation: "tessellate_body",
                input_digest,
                backend_fingerprint: BACKEND_FINGERPRINT,
            });
        }
        Ok(ExactTessellation {
            vertices_mm,
            triangles,
        })
    }

    /// Build a bounded tetrahedral mesh from one exact solid.
    ///
    /// The native mesher cones an OCCT face tessellation to a verified interior
    /// point. It therefore accepts only solids whose complete tessellated
    /// boundary is visible from that point and fails closed for unsupported
    /// non-star-shaped domains. The returned mesh is additionally checked for
    /// manifold connectivity, positive Jacobians, quality and volume error.
    pub fn volume_mesh_body(
        &self,
        body: &ExactBody,
        options: ExactVolumeMeshOptions,
    ) -> Result<ExactVolumeMesh, GeometryError> {
        let input = format!(
            "volume_mesh_body:{}:{:016x}:{:016x}:{}:{:016x}:{:016x}",
            body.result_fingerprint,
            options.surface_deflection_mm.to_bits(),
            options.angular_deflection_rad.to_bits(),
            options.max_tetrahedra,
            options.max_relative_volume_error.to_bits(),
            options.min_tetrahedron_quality.to_bits(),
        );
        let input_digest = stable_digest(&input);
        let invalid_options = !options.surface_deflection_mm.is_finite()
            || options.surface_deflection_mm <= 0.0
            || options.surface_deflection_mm > MAX_LENGTH_MM
            || !options.angular_deflection_rad.is_finite()
            || options.angular_deflection_rad <= 0.0
            || options.angular_deflection_rad > std::f64::consts::PI
            || !(4..=65_536).contains(&options.max_tetrahedra)
            || !options.max_relative_volume_error.is_finite()
            || !(NEGLIGIBLE..=0.25).contains(&options.max_relative_volume_error)
            || !options.min_tetrahedron_quality.is_finite()
            || !(0.0..=1.0).contains(&options.min_tetrahedron_quality);
        if invalid_options {
            return Err(parameter_error(
                GeometryErrorCode::InvalidParameter,
                "volume_mesh_body",
                &input,
                "Volume-mesh options are non-finite or outside bounded ranges".to_owned(),
            ));
        }
        if body.topology.solid_count != 1 || body.topology.volume_mm3 <= 0.0 {
            return Err(parameter_error(
                GeometryErrorCode::InvalidShape,
                "volume_mesh_body",
                &input,
                "Volume meshing requires exactly one closed exact solid".to_owned(),
            ));
        }
        let native = body.native.as_ref().ok_or_else(|| GeometryError {
            code: GeometryErrorCode::NullResult,
            diagnostic: "Exact body lost its owned native shape".to_owned(),
            operation: "volume_mesh_body",
            input_digest: input_digest.clone(),
            backend_fingerprint: BACKEND_FINGERPRINT,
        })?;
        let mesh = ffi::volume_mesh_body_native(
            native,
            options.surface_deflection_mm,
            options.angular_deflection_rad,
            options.max_tetrahedra,
        );
        let mesh = mesh.as_ref().ok_or_else(|| GeometryError {
            code: GeometryErrorCode::NullResult,
            diagnostic: "Native facade returned no volume-mesh object".to_owned(),
            operation: "volume_mesh_body",
            input_digest: input_digest.clone(),
            backend_fingerprint: BACKEND_FINGERPRINT,
        })?;
        if mesh.volume_mesh_status_code() != 0 {
            return Err(GeometryError {
                code: native_status(mesh.volume_mesh_status_code()),
                diagnostic: mesh.volume_mesh_diagnostic(),
                operation: "volume_mesh_body",
                input_digest,
                backend_fingerprint: BACKEND_FINGERPRINT,
            });
        }

        let vertices_mm = mesh
            .volume_mesh_vertices()
            .into_iter()
            .map(|vertex| [vertex.x_mm, vertex.y_mm, vertex.z_mm])
            .collect::<Vec<_>>();
        let tetrahedra = mesh
            .volume_mesh_tetrahedra()
            .into_iter()
            .map(|tetrahedron| ExactVolumeMeshTetrahedron {
                vertex_indices: [
                    tetrahedron.first,
                    tetrahedron.second,
                    tetrahedron.third,
                    tetrahedron.fourth,
                ],
            })
            .collect::<Vec<_>>();
        let boundary_triangles = mesh
            .volume_mesh_boundary_triangles()
            .into_iter()
            .map(|triangle| ExactMeshTriangle {
                vertex_indices: [triangle.first, triangle.second, triangle.third],
                face_ordinal: triangle.face_ordinal,
            })
            .collect::<Vec<_>>();
        let vertex_count = vertices_mm.len();
        if vertices_mm.len() < 4
            || tetrahedra.is_empty()
            || tetrahedra.len() > options.max_tetrahedra as usize
            || boundary_triangles.is_empty()
            || vertices_mm
                .iter()
                .flatten()
                .any(|coordinate| !coordinate.is_finite())
            || tetrahedra.iter().any(|tetrahedron| {
                tetrahedron
                    .vertex_indices
                    .iter()
                    .any(|index| *index as usize >= vertex_count)
                    || {
                        let mut unique = tetrahedron.vertex_indices;
                        unique.sort_unstable();
                        unique.windows(2).any(|pair| pair[0] == pair[1])
                    }
            })
            || boundary_triangles.iter().any(|triangle| {
                triangle.face_ordinal >= body.topology.face_count
                    || triangle
                        .vertex_indices
                        .iter()
                        .any(|index| *index as usize >= vertex_count)
            })
        {
            return Err(parameter_error(
                GeometryErrorCode::InvalidShape,
                "volume_mesh_body",
                &input,
                "Native volume mesh is not a bounded well-formed indexed mesh".to_owned(),
            ));
        }

        let mut face_uses = std::collections::BTreeMap::<[u32; 3], u32>::new();
        let mut tetrahedral_volume_mm3 = 0.0;
        let mut minimum_signed_volume_mm3 = f64::INFINITY;
        let mut minimum_quality = f64::INFINITY;
        let mut maximum_edge_ratio = 0.0_f64;
        for tetrahedron in &tetrahedra {
            let indices = tetrahedron.vertex_indices;
            for face in [
                [indices[0], indices[1], indices[2]],
                [indices[0], indices[1], indices[3]],
                [indices[0], indices[2], indices[3]],
                [indices[1], indices[2], indices[3]],
            ] {
                let mut key = face;
                key.sort_unstable();
                *face_uses.entry(key).or_default() += 1;
            }
            let points = indices.map(|index| vertices_mm[index as usize]);
            let ab = subtract3(points[1], points[0]);
            let ac = subtract3(points[2], points[0]);
            let ad = subtract3(points[3], points[0]);
            let signed_volume_mm3 = dot3(ab, cross3(ac, ad)) / 6.0;
            if !signed_volume_mm3.is_finite() || signed_volume_mm3 <= NEGLIGIBLE {
                return Err(parameter_error(
                    GeometryErrorCode::InvalidShape,
                    "volume_mesh_body",
                    &input,
                    "Volume mesh contains an inverted or degenerate tetrahedron".to_owned(),
                ));
            }
            let edge_squared = [
                squared_distance3(points[0], points[1]),
                squared_distance3(points[0], points[2]),
                squared_distance3(points[0], points[3]),
                squared_distance3(points[1], points[2]),
                squared_distance3(points[1], points[3]),
                squared_distance3(points[2], points[3]),
            ];
            let minimum_edge_squared = edge_squared.iter().copied().fold(f64::INFINITY, f64::min);
            let maximum_edge_squared = edge_squared.iter().copied().fold(0.0_f64, f64::max);
            let quality =
                12.0 * (3.0 * signed_volume_mm3).powf(2.0 / 3.0) / edge_squared.iter().sum::<f64>();
            if !quality.is_finite()
                || quality < options.min_tetrahedron_quality
                || minimum_edge_squared <= 0.0
            {
                return Err(parameter_error(
                    GeometryErrorCode::InvalidShape,
                    "volume_mesh_body",
                    &input,
                    "Volume mesh violates the requested tetrahedron quality bound".to_owned(),
                ));
            }
            tetrahedral_volume_mm3 += signed_volume_mm3;
            minimum_signed_volume_mm3 = minimum_signed_volume_mm3.min(signed_volume_mm3);
            minimum_quality = minimum_quality.min(quality);
            maximum_edge_ratio =
                maximum_edge_ratio.max((maximum_edge_squared / minimum_edge_squared).sqrt());
        }

        let expected_boundary = face_uses
            .iter()
            .filter_map(|(face, count)| (*count == 1).then_some(*face))
            .collect::<std::collections::BTreeSet<_>>();
        if face_uses.values().any(|count| *count > 2) {
            return Err(parameter_error(
                GeometryErrorCode::InvalidShape,
                "volume_mesh_body",
                &input,
                "Volume mesh contains non-manifold tetrahedral faces".to_owned(),
            ));
        }
        let actual_boundary = boundary_triangles
            .iter()
            .map(|triangle| {
                let mut face = triangle.vertex_indices;
                face.sort_unstable();
                face
            })
            .collect::<std::collections::BTreeSet<_>>();
        if actual_boundary.len() != boundary_triangles.len() || actual_boundary != expected_boundary
        {
            return Err(parameter_error(
                GeometryErrorCode::InvalidShape,
                "volume_mesh_body",
                &input,
                "Boundary provenance does not match the tetrahedral boundary".to_owned(),
            ));
        }
        let mut boundary_edges = std::collections::BTreeMap::<[u32; 2], u32>::new();
        for triangle in &boundary_triangles {
            let indices = triangle.vertex_indices;
            for mut edge in [
                [indices[0], indices[1]],
                [indices[1], indices[2]],
                [indices[2], indices[0]],
            ] {
                edge.sort_unstable();
                *boundary_edges.entry(edge).or_default() += 1;
            }
        }
        if boundary_edges.values().any(|count| *count != 2) {
            return Err(parameter_error(
                GeometryErrorCode::InvalidShape,
                "volume_mesh_body",
                &input,
                "Tetrahedral boundary is open or non-manifold".to_owned(),
            ));
        }

        let exact_volume_mm3 = body.topology.volume_mm3;
        let relative_volume_error =
            (tetrahedral_volume_mm3 - exact_volume_mm3).abs() / exact_volume_mm3;
        if !relative_volume_error.is_finite()
            || relative_volume_error > options.max_relative_volume_error
        {
            return Err(parameter_error(
                GeometryErrorCode::InvalidShape,
                "volume_mesh_body",
                &input,
                format!(
                    "Tetrahedral volume relative error {relative_volume_error:.6e} exceeds {:.6e}",
                    options.max_relative_volume_error
                ),
            ));
        }

        let mut fingerprint_input = format!(
            "{EXACT_VOLUME_MESH_SCHEMA}:{}:{input_digest}",
            body.result_fingerprint
        );
        for vertex in &vertices_mm {
            for coordinate in vertex {
                fingerprint_input.push_str(&format!(":{:016x}", coordinate.to_bits()));
            }
        }
        for tetrahedron in &tetrahedra {
            fingerprint_input.push_str(&format!(":{:?}", tetrahedron.vertex_indices));
        }
        for triangle in &boundary_triangles {
            fingerprint_input.push_str(&format!(
                ":{:?}:{}",
                triangle.vertex_indices, triangle.face_ordinal
            ));
        }
        let mesh_fingerprint = stable_digest(&fingerprint_input);
        Ok(ExactVolumeMesh {
            schema: EXACT_VOLUME_MESH_SCHEMA,
            source_result_fingerprint: body.result_fingerprint.clone(),
            request_digest: input_digest,
            mesh_fingerprint,
            vertices_mm,
            tetrahedra,
            boundary_triangles,
            exact_volume_mm3,
            tetrahedral_volume_mm3,
            relative_volume_error,
            minimum_signed_volume_mm3,
            minimum_quality,
            maximum_edge_ratio,
        })
    }
}
