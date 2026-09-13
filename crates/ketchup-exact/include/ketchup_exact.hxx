#pragma once

#include "rust/cxx.h"

#include <cstdint>
#include <memory>

namespace ketchup::exact {

struct NativeEdgeEvidence;
struct NativeEdgeFaceEvidence;
struct NativeEdgeHistoryEvidence;
struct NativeFaceEdgeEvidence;
struct NativeFaceEvidence;
struct NativeHistoryEvidence;
struct NativeMeshTriangle;
struct NativeMeshVertex;
struct NativeVolumeMeshTetrahedron;
struct NativeTopologySummary;
struct NativePairQuery;
NativePairQuery query_body_pair_native(
    const class NativeOperationResult& left,
    const class NativeOperationResult& right) noexcept;

class NativeOperationResult final {
public:
  struct Impl;

  explicit NativeOperationResult(std::unique_ptr<Impl> impl) noexcept;
  ~NativeOperationResult();
  NativeOperationResult(NativeOperationResult&&) noexcept;
  NativeOperationResult& operator=(NativeOperationResult&&) noexcept;
  NativeOperationResult(const NativeOperationResult&) = delete;
  NativeOperationResult& operator=(const NativeOperationResult&) = delete;

  std::uint8_t status_code() const noexcept;
  rust::String diagnostic() const;
  bool valid() const noexcept;
  NativeTopologySummary topology_summary() const noexcept;
  rust::Vec<NativeFaceEvidence> face_evidence() const;
  rust::Vec<NativeEdgeEvidence> edge_evidence() const;
  rust::Vec<NativeFaceEdgeEvidence> face_edge_evidence() const;
  rust::Vec<NativeEdgeFaceEvidence> edge_face_evidence() const;
  rust::Vec<NativeHistoryEvidence> history_evidence() const;
  rust::Vec<NativeEdgeHistoryEvidence> edge_history_evidence() const;

  const Impl& impl() const noexcept;

private:
  std::unique_ptr<Impl> impl_;
};

std::unique_ptr<NativeOperationResult> make_box_native(
    double origin_x, double origin_y, double origin_z,
    double size_x, double size_y, double size_z) noexcept;
std::unique_ptr<NativeOperationResult> extrude_rectangle_native(
    double width, double depth, double height) noexcept;
std::unique_ptr<NativeOperationResult> offset_rectangle_native(
    double min_x, double min_y, double max_x, double max_y,
    double distance) noexcept;
std::unique_ptr<NativeOperationResult> offset_planar_profile_native(
    rust::Slice<const double> segments, double distance) noexcept;
std::unique_ptr<NativeOperationResult> planar_surface_profile_native(
    rust::Slice<const double> segments) noexcept;
std::unique_ptr<NativeOperationResult> trim_surface_native(
    const NativeOperationResult& target,
    const NativeOperationResult& cutter) noexcept;
std::unique_ptr<NativeOperationResult> extend_planar_surface_native(
    const NativeOperationResult& target, double distance) noexcept;
std::unique_ptr<NativeOperationResult> combine_surfaces_native(
    const NativeOperationResult& base,
    const NativeOperationResult& added) noexcept;
std::unique_ptr<NativeOperationResult> knit_surface_compound_native(
    const NativeOperationResult& surfaces,
    double tolerance, bool make_solid) noexcept;
std::unique_ptr<NativeOperationResult> thicken_surface_native(
    const NativeOperationResult& surface,
    double thickness, std::uint8_t direction) noexcept;
std::unique_ptr<NativeOperationResult> offset_planar_region_native(
    rust::Slice<const double> segments,
    rust::Slice<const std::uint32_t> loop_segment_counts,
    double distance) noexcept;
std::unique_ptr<NativeOperationResult> offset_planar_circle_native(
    double center_x, double center_y, double radius, double distance) noexcept;
std::unique_ptr<NativeOperationResult> sweep_rectangle_native(
    rust::Slice<const double> values) noexcept;
std::unique_ptr<NativeOperationResult> sweep_planar_profile_native(
    rust::Slice<const double> profile_segments,
    rust::Slice<const double> path_segments) noexcept;
std::unique_ptr<NativeOperationResult> loft_framed_profiles_native(
    rust::Slice<const double> values,
    rust::Slice<const double> guide_segments,
    std::uint8_t continuity,
    bool make_solid) noexcept;
std::unique_ptr<NativeOperationResult> loft_spline_native(
    rust::Slice<const double> values) noexcept;
std::unique_ptr<NativeOperationResult> loft_planar_profiles_native(
    rust::Slice<const double> segments,
    rust::Slice<const std::uint32_t> section_segment_counts,
    rust::Slice<const double> elevations) noexcept;
std::unique_ptr<NativeOperationResult> extrude_circle_native(
    double center_x, double center_y, double radius, double height) noexcept;
std::unique_ptr<NativeOperationResult> sweep_axial_tool_native(
    rust::Slice<const double> values) noexcept;
std::unique_ptr<NativeOperationResult> extrude_mixed_profile_native(
    rust::Slice<const double> segments, double height) noexcept;
std::unique_ptr<NativeOperationResult> extrude_planar_region_native(
    rust::Slice<const double> segments,
    rust::Slice<const std::uint32_t> loop_segment_counts,
    double height) noexcept;
std::unique_ptr<NativeOperationResult> revolve_profile_native(
    rust::Slice<const double> points) noexcept;
std::unique_ptr<NativeOperationResult> revolve_general_profile_native(
    rust::Slice<const double> segments,
    double axis_start_x, double axis_start_y,
    double axis_end_x, double axis_end_y,
    double angle_degrees) noexcept;
std::unique_ptr<NativeOperationResult> revolve_planar_region_native(
    rust::Slice<const double> segments,
    rust::Slice<const std::uint32_t> loop_segment_counts,
    double axis_start_x, double axis_start_y,
    double axis_end_x, double axis_end_y,
    double angle_degrees) noexcept;
std::unique_ptr<NativeOperationResult> shell_box_native(
    double width, double depth, double height, double thickness) noexcept;
std::unique_ptr<NativeOperationResult> finish_shell_box_native(
    double width, double depth, double height, double thickness,
    double amount, bool fillet) noexcept;
std::unique_ptr<NativeOperationResult> shell_revolve_profile_native(
    rust::Slice<const double> points, double thickness) noexcept;
std::unique_ptr<NativeOperationResult> finish_shell_revolve_profile_native(
    rust::Slice<const double> points, double thickness, double amount,
    bool fillet) noexcept;
std::unique_ptr<NativeOperationResult> shell_body_native(
    const NativeOperationResult& body, rust::Slice<const std::uint32_t> face_ordinals,
    double thickness, std::uint8_t direction) noexcept;
std::unique_ptr<NativeOperationResult> offset_body_face_native(
    const NativeOperationResult& body, std::uint32_t face_ordinal,
    double distance) noexcept;
std::unique_ptr<NativeOperationResult> finish_body_native(
    const NativeOperationResult& body, rust::Slice<const std::uint32_t> edge_ordinals,
    rust::Slice<const std::uint32_t> face_ordinals, double amount, bool fillet,
    rust::Slice<const double> fillet_radius_stations, std::uint8_t chamfer_mode,
    double chamfer_secondary) noexcept;
std::unique_ptr<NativeOperationResult> cut_box_native(
    const NativeOperationResult& base,
    double origin_x, double origin_y, double origin_z,
    double size_x, double size_y, double size_z) noexcept;
std::unique_ptr<NativeOperationResult> cut_mixed_profile_native(
    const NativeOperationResult& base, rust::Slice<const double> segments,
    double origin_z, double height) noexcept;
std::unique_ptr<NativeOperationResult> fuse_mixed_profile_native(
    const NativeOperationResult& base, rust::Slice<const double> segments,
    double origin_z, double height) noexcept;
std::unique_ptr<NativeOperationResult> common_mixed_profile_native(
    const NativeOperationResult& base, rust::Slice<const double> segments,
    double origin_z, double height) noexcept;
std::unique_ptr<NativeOperationResult> split_mixed_profile_native(
    const NativeOperationResult& base, rust::Slice<const double> segments,
    double origin_z, double height) noexcept;
std::unique_ptr<NativeOperationResult> cut_cylinder_native(
    const NativeOperationResult& base,
    double center_x, double center_y, double origin_z,
    double radius, double height) noexcept;
std::unique_ptr<NativeOperationResult> fuse_cylinder_native(
    const NativeOperationResult& base,
    double center_x, double center_y, double origin_z,
    double radius, double height) noexcept;
std::unique_ptr<NativeOperationResult> common_cylinder_native(
    const NativeOperationResult& base,
    double center_x, double center_y, double origin_z,
    double radius, double height) noexcept;
std::unique_ptr<NativeOperationResult> split_cylinder_native(
    const NativeOperationResult& base,
    double center_x, double center_y, double origin_z,
    double radius, double height) noexcept;
std::unique_ptr<NativeOperationResult> fuse_box_native(
    const NativeOperationResult& base,
    double origin_x, double origin_y, double origin_z,
    double size_x, double size_y, double size_z) noexcept;
std::unique_ptr<NativeOperationResult> common_box_native(
    const NativeOperationResult& base,
    double origin_x, double origin_y, double origin_z,
    double size_x, double size_y, double size_z) noexcept;
std::unique_ptr<NativeOperationResult> split_box_native(
    const NativeOperationResult& base,
    double origin_x, double origin_y, double origin_z,
    double size_x, double size_y, double size_z) noexcept;
std::unique_ptr<NativeOperationResult> exception_probe_native() noexcept;
std::unique_ptr<NativeOperationResult> import_step_native(rust::Str path) noexcept;
std::unique_ptr<NativeOperationResult> import_step_solid_native(
    rust::Str path, std::uint32_t solid_ordinal) noexcept;
std::unique_ptr<NativeOperationResult> import_step_xde_part_native(
    rust::Str path, std::uint32_t part_index) noexcept;
rust::String step_xde_manifest_native(rust::Str path) noexcept;
rust::String export_step_xde_assembly_native(
    rust::Str manifest, rust::Str path) noexcept;
rust::String step_length_unit_native(rust::Str path) noexcept;
std::unique_ptr<NativeOperationResult> import_iges_native(rust::Str path) noexcept;
std::unique_ptr<NativeOperationResult> import_iges_xde_part_native(
    rust::Str path, std::uint32_t part_index) noexcept;
rust::String iges_xde_manifest_native(rust::Str path) noexcept;
rust::String export_iges_xde_assembly_native(
    rust::Str manifest, rust::Str path) noexcept;
rust::String iges_length_unit_native(rust::Str path) noexcept;
std::unique_ptr<NativeOperationResult> transform_body_native(
    const NativeOperationResult& body, rust::Slice<const double> matrix) noexcept;
std::unique_ptr<NativeOperationResult> combine_bodies_native(
    const NativeOperationResult& base, const NativeOperationResult& added) noexcept;
std::unique_ptr<NativeOperationResult> trim_body_by_plane_native(
    const NativeOperationResult& body,
    double origin_x, double origin_y, double origin_z,
    double normal_x, double normal_y, double normal_z,
    double keep_x, double keep_y, double keep_z) noexcept;
std::unique_ptr<NativeOperationResult> boolean_bodies_native(
    const NativeOperationResult& target, const NativeOperationResult& tool,
    std::uint8_t operation) noexcept;
rust::String export_step_native(
    const NativeOperationResult& body, rust::Str path) noexcept;
rust::String export_iges_native(
    const NativeOperationResult& body, rust::Str path) noexcept;

class NativeMeshResult final {
public:
  struct Impl;

  explicit NativeMeshResult(std::unique_ptr<Impl> impl) noexcept;
  ~NativeMeshResult();
  NativeMeshResult(NativeMeshResult&&) noexcept;
  NativeMeshResult& operator=(NativeMeshResult&&) noexcept;
  NativeMeshResult(const NativeMeshResult&) = delete;
  NativeMeshResult& operator=(const NativeMeshResult&) = delete;

  std::uint8_t mesh_status_code() const noexcept;
  rust::String mesh_diagnostic() const;
  rust::Vec<NativeMeshVertex> mesh_vertices() const;
  rust::Vec<NativeMeshTriangle> mesh_triangles() const;

private:
  std::unique_ptr<Impl> impl_;
};

std::unique_ptr<NativeMeshResult> tessellate_body_native(
    const NativeOperationResult& body, double deflection,
    double angular_deflection, std::uint32_t max_triangles) noexcept;

class NativeVolumeMeshResult final {
public:
  struct Impl;

  explicit NativeVolumeMeshResult(std::unique_ptr<Impl> impl) noexcept;
  ~NativeVolumeMeshResult();
  NativeVolumeMeshResult(NativeVolumeMeshResult&&) noexcept;
  NativeVolumeMeshResult& operator=(NativeVolumeMeshResult&&) noexcept;
  NativeVolumeMeshResult(const NativeVolumeMeshResult&) = delete;
  NativeVolumeMeshResult& operator=(const NativeVolumeMeshResult&) = delete;

  std::uint8_t volume_mesh_status_code() const noexcept;
  rust::String volume_mesh_diagnostic() const;
  rust::Vec<NativeMeshVertex> volume_mesh_vertices() const;
  rust::Vec<NativeVolumeMeshTetrahedron> volume_mesh_tetrahedra() const;
  rust::Vec<NativeMeshTriangle> volume_mesh_boundary_triangles() const;

private:
  std::unique_ptr<Impl> impl_;
};

std::unique_ptr<NativeVolumeMeshResult> volume_mesh_body_native(
    const NativeOperationResult& body, double deflection,
    double angular_deflection, std::uint32_t max_tetrahedra) noexcept;

} // namespace ketchup::exact
