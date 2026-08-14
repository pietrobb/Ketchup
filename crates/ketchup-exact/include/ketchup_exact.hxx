#pragma once

#include "rust/cxx.h"

#include <cstdint>
#include <memory>

namespace ketchup::exact {

struct NativeEdgeFaceEvidence;
struct NativeFaceEdgeEvidence;
struct NativeFaceEvidence;
struct NativeHistoryEvidence;
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
std::unique_ptr<NativeOperationResult> cut_box_native(
    const NativeOperationResult& base,
    double origin_x, double origin_y, double origin_z,
    double size_x, double size_y, double size_z) noexcept;
std::unique_ptr<NativeOperationResult> cut_cylinder_native(
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
std::unique_ptr<NativeOperationResult> transform_body_native(
    const NativeOperationResult& body, rust::Slice<const double> matrix) noexcept;
std::unique_ptr<NativeOperationResult> combine_bodies_native(
    const NativeOperationResult& base, const NativeOperationResult& added) noexcept;
rust::String export_step_native(
    const NativeOperationResult& body, rust::Str path) noexcept;

} // namespace ketchup::exact
