#pragma once

#include "rust/cxx.h"

#include <cstdint>
#include <memory>

namespace ketchup::exact {

struct NativeEdgeFaceEvidence;
struct NativeEdgeHistoryEvidence;
struct NativeFaceEdgeEvidence;
struct NativeFaceEvidence;
struct NativeHistoryEvidence;
struct NativeMeshTriangle;
struct NativeMeshVertex;
struct NativeTopologySummary;

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
std::unique_ptr<NativeOperationResult> sweep_rectangle_native(
    rust::Slice<const double> values) noexcept;
std::unique_ptr<NativeOperationResult> loft_spline_native(
    rust::Slice<const double> values) noexcept;
std::unique_ptr<NativeOperationResult> extrude_circle_native(
    double center_x, double center_y, double radius, double height) noexcept;
std::unique_ptr<NativeOperationResult> extrude_mixed_profile_native(
    rust::Slice<const double> segments, double height) noexcept;
std::unique_ptr<NativeOperationResult> revolve_profile_native(
    rust::Slice<const double> points) noexcept;
std::unique_ptr<NativeOperationResult> revolve_general_profile_native(
    rust::Slice<const double> segments,
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
    double thickness) noexcept;
std::unique_ptr<NativeOperationResult> offset_body_face_native(
    const NativeOperationResult& body, std::uint32_t face_ordinal,
    double distance) noexcept;
std::unique_ptr<NativeOperationResult> finish_body_native(
    const NativeOperationResult& body, rust::Slice<const std::uint32_t> edge_ordinals,
    double amount, bool fillet) noexcept;
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
rust::String step_length_unit_native(rust::Str path) noexcept;
std::unique_ptr<NativeOperationResult> transform_body_native(
    const NativeOperationResult& body, rust::Slice<const double> matrix) noexcept;
std::unique_ptr<NativeOperationResult> combine_bodies_native(
    const NativeOperationResult& base, const NativeOperationResult& added) noexcept;
std::unique_ptr<NativeOperationResult> boolean_bodies_native(
    const NativeOperationResult& target, const NativeOperationResult& tool,
    std::uint8_t operation) noexcept;
rust::String export_step_native(
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

} // namespace ketchup::exact
