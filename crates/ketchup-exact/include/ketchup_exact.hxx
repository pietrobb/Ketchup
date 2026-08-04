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
std::unique_ptr<NativeOperationResult> cut_box_native(
    const NativeOperationResult& base,
    double origin_x, double origin_y, double origin_z,
    double size_x, double size_y, double size_z) noexcept;
std::unique_ptr<NativeOperationResult> exception_probe_native() noexcept;
std::unique_ptr<NativeOperationResult> import_step_native(rust::Str path) noexcept;

} // namespace ketchup::exact
