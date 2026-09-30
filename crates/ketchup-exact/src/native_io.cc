#include "native_common.hxx"

namespace ketchup::exact {

namespace {

constexpr std::size_t MAX_STEP_XDE_NODES = 1024;
constexpr std::size_t MAX_STEP_XDE_DEPTH = 64;

class IgesBrepModeGuard {
public:
  IgesBrepModeGuard() : lock_(mutex()) {
    IGESControl_Controller::Init();
    previous_ = Interface_Static::IVal("write.iges.brep.mode");
    active_ = Interface_Static::SetIVal("write.iges.brep.mode", 1);
  }

  ~IgesBrepModeGuard() {
    if (active_) {
      Interface_Static::SetIVal("write.iges.brep.mode", previous_);
    }
  }

  bool active() const noexcept { return active_; }

private:
  static std::mutex& mutex() {
    static std::mutex value;
    return value;
  }

  std::unique_lock<std::mutex> lock_;
  int previous_ = 0;
  bool active_ = false;
};

struct StepXdeNode {
  std::uint32_t id;
  std::int32_t parent_id;
  std::int32_t part_index;
  std::string name;
  bool name_from_source;
  std::string color;
  gp_Trsf transform;
};

struct StepXdeData {
  occ::handle<TDocStd_Document> document;
  occ::handle<XCAFDoc_ShapeTool> shapes;
  occ::handle<XCAFDoc_ColorTool> colors;
  std::vector<TDF_Label> parts;
  std::vector<StepXdeNode> nodes;
};

std::string hex_encode(const std::string& value) {
  static constexpr char digits[] = "0123456789abcdef";
  std::string encoded;
  encoded.reserve(value.size() * 2);
  for (const unsigned char byte : value) {
    encoded.push_back(digits[byte >> 4]);
    encoded.push_back(digits[byte & 0x0f]);
  }
  return encoded;
}

std::string label_entry(const TDF_Label& label) {
  TCollection_AsciiString entry;
  TDF_Tool::Entry(label, entry);
  return entry.ToCString();
}

std::string label_name(const TDF_Label& label) {
  occ::handle<TDataStd_Name> attribute;
  if (!label.FindAttribute(TDataStd_Name::GetID(), attribute) || attribute.IsNull()) {
    return {};
  }
  const TCollection_ExtendedString& value = attribute->Get();
  std::vector<char> utf8(static_cast<std::size_t>(value.LengthOfCString()) + 1, '\0');
  Standard_PCharacter output = utf8.data();
  const int length = value.ToUTF8CString(output);
  return length > 0 ? std::string(utf8.data(), static_cast<std::size_t>(length)) : std::string();
}

std::string label_color(
    const occ::handle<XCAFDoc_ColorTool>& colors,
    const TDF_Label& primary,
    const TDF_Label& fallback) {
  Quantity_Color color;
  const auto read = [&](const TDF_Label& label) {
    return XCAFDoc_ColorTool::GetColor(label, XCAFDoc_ColorGen, color)
        || XCAFDoc_ColorTool::GetColor(label, XCAFDoc_ColorSurf, color);
  };
  if (!read(primary) && (fallback.IsNull() || !read(fallback))) {
    return "-";
  }
  double red = 0.0;
  double green = 0.0;
  double blue = 0.0;
  color.Values(red, green, blue, Quantity_TOC_sRGB);
  const auto byte = [](double value) {
    return static_cast<unsigned int>(std::lround(std::clamp(value, 0.0, 1.0) * 255.0));
  };
  std::ostringstream encoded;
  encoded << std::hex << std::setfill('0')
          << std::setw(2) << byte(red)
          << std::setw(2) << byte(green)
          << std::setw(2) << byte(blue);
  return encoded.str();
}

template <typename Reader>
bool load_xde(const std::string& path, StepXdeData& data, std::string& error) {
  data.document = new TDocStd_Document(TCollection_ExtendedString("BinXCAF"));
  Reader reader;
  reader.SetColorMode(true);
  reader.SetNameMode(true);
  if (reader.ReadFile(path.c_str()) != IFSelect_RetDone) {
    error = "STEP XDE reader could not read the source";
    return false;
  }
  if (!reader.Transfer(data.document)) {
    error = "STEP XDE source contains no transferable roots";
    return false;
  }
  data.shapes = XCAFDoc_DocumentTool::ShapeTool(data.document->Main());
  data.colors = XCAFDoc_DocumentTool::ColorTool(data.document->Main());
  if (data.shapes.IsNull() || data.colors.IsNull()) {
    error = "STEP XDE document tools are unavailable";
    return false;
  }

  std::map<std::string, std::uint32_t> part_indices;
  std::set<std::string> active_assemblies;
  std::function<bool(const TDF_Label&, const TDF_Label&, std::int32_t, const gp_Trsf&, std::size_t)>
      visit;
  visit = [&](const TDF_Label& instance,
              const TDF_Label& referred,
              std::int32_t parent_id,
              const gp_Trsf& transform,
              std::size_t depth) {
    if (depth > MAX_STEP_XDE_DEPTH || data.nodes.size() >= MAX_STEP_XDE_NODES) {
      error = "STEP XDE hierarchy exceeds the bounded envelope";
      return false;
    }
    const bool assembly = XCAFDoc_ShapeTool::IsAssembly(referred);
    std::int32_t part_index = -1;
    if (!assembly) {
      const std::string entry = label_entry(referred);
      const auto [iterator, inserted] = part_indices.emplace(entry, part_indices.size());
      if (inserted) {
        if (data.parts.size() >= MAX_STEP_XDE_NODES) {
          error = "STEP XDE part count exceeds the bounded envelope";
          return false;
        }
        data.parts.push_back(referred);
      }
      part_index = static_cast<std::int32_t>(iterator->second);
    }
    std::string name = label_name(instance);
    if (name.empty()) {
      name = label_name(referred);
    }
    const bool name_from_source = !name.empty();
    if (!name_from_source) {
      name = assembly ? "Imported STEP assembly" : "Imported STEP part";
    }
    const std::uint32_t node_id = static_cast<std::uint32_t>(data.nodes.size());
    data.nodes.push_back(StepXdeNode{
        node_id,
        parent_id,
        part_index,
        std::move(name),
        name_from_source,
        label_color(data.colors, instance, referred),
        transform,
    });
    if (!assembly) {
      return true;
    }

    const std::string assembly_entry = label_entry(referred);
    if (!active_assemblies.insert(assembly_entry).second) {
      error = "STEP XDE hierarchy contains an assembly cycle";
      return false;
    }
    NCollection_Sequence<TDF_Label> components;
    if (!XCAFDoc_ShapeTool::GetComponents(referred, components, false)) {
      active_assemblies.erase(assembly_entry);
      error = "STEP XDE assembly has no readable components";
      return false;
    }
    for (int index = 1; index <= components.Length(); ++index) {
      const TDF_Label component = components.Value(index);
      TDF_Label target;
      if (!XCAFDoc_ShapeTool::GetReferredShape(component, target) || target.IsNull()) {
        active_assemblies.erase(assembly_entry);
        error = "STEP XDE component has no referred definition";
        return false;
      }
      const gp_Trsf local = XCAFDoc_ShapeTool::GetLocation(component).Transformation();
      if (!visit(component, target, static_cast<std::int32_t>(node_id), local, depth + 1)) {
        active_assemblies.erase(assembly_entry);
        return false;
      }
    }
    active_assemblies.erase(assembly_entry);
    return true;
  };

  NCollection_Sequence<TDF_Label> roots;
  data.shapes->GetFreeShapes(roots);
  if (roots.IsEmpty()) {
    error = "STEP XDE source contains no free shapes";
    return false;
  }
  for (int index = 1; index <= roots.Length(); ++index) {
    const TDF_Label root = roots.Value(index);
    TDF_Label referred = root;
    if (XCAFDoc_ShapeTool::IsReference(root)
        && !XCAFDoc_ShapeTool::GetReferredShape(root, referred)) {
      error = "STEP XDE root reference is unresolved";
      return false;
    }
    const gp_Trsf transform = XCAFDoc_ShapeTool::GetLocation(root).Transformation();
    if (!visit(root, referred, -1, transform, 0)) {
      return false;
    }
  }
  if (data.parts.empty() || data.nodes.empty()) {
    error = "STEP XDE source contains no supported exact parts";
    return false;
  }
  return true;
}

bool load_step_xde(const std::string& path, StepXdeData& data, std::string& error) {
  return load_xde<STEPCAFControl_Reader>(path, data, error);
}

bool load_iges_xde(const std::string& path, StepXdeData& data, std::string& error) {
  if (!load_xde<IGESCAFControl_Reader>(path, data, error)) {
    return false;
  }
  if (data.parts.size() != 1 || data.nodes.size() != 1) {
    return true;
  }
  const TDF_Label root = data.parts.front();
  const TopoDS_Shape root_shape = XCAFDoc_ShapeTool::GetShape(root);
  if (root_shape.IsNull() || count_subshapes(root_shape, TopAbs_SOLID) < 2) {
    return true;
  }

  std::vector<TDF_Label> parts;
  std::vector<StepXdeNode> nodes;
  std::string root_name = label_name(root);
  const bool root_name_from_source = !root_name.empty();
  if (!root_name_from_source) {
    root_name = "Imported IGES model";
  }
  nodes.push_back(StepXdeNode{
      0, -1, -1, root_name, root_name_from_source,
      label_color(data.colors, root, TDF_Label()), gp_Trsf()});
  for (TopExp_Explorer solids(root_shape, TopAbs_SOLID); solids.More(); solids.Next()) {
    if (parts.size() >= MAX_STEP_XDE_NODES || nodes.size() >= MAX_STEP_XDE_NODES) {
      error = "IGES XDE solid count exceeds the bounded envelope";
      return false;
    }
    TDF_Label part;
    if (!data.shapes->FindSubShape(root, solids.Current(), part) || part.IsNull()) {
      part = data.shapes->AddSubShape(root, solids.Current());
    }
    if (part.IsNull()) {
      error = "IGES XDE reader could not identify a transferred solid";
      return false;
    }
    std::string name = label_name(part);
    const bool name_from_source = !name.empty();
    if (!name_from_source) {
      name = "Imported IGES part " + std::to_string(parts.size() + 1);
    }
    const std::uint32_t part_index = static_cast<std::uint32_t>(parts.size());
    parts.push_back(part);
    nodes.push_back(StepXdeNode{
        static_cast<std::uint32_t>(nodes.size()), 0,
        static_cast<std::int32_t>(part_index), name, name_from_source,
        label_color(data.colors, part, root), gp_Trsf()});
  }
  data.parts = std::move(parts);
  data.nodes = std::move(nodes);
  return true;
}

std::string matrix_fields(const gp_Trsf& transform) {
  std::ostringstream output;
  output << std::hex << std::setfill('0');
  for (int row = 1; row <= 4; ++row) {
    for (int column = 1; column <= 4; ++column) {
      const double value = row == 4 ? (column == 4 ? 1.0 : 0.0) : transform.Value(row, column);
      std::uint64_t bits = 0;
      static_assert(sizeof(bits) == sizeof(value));
      std::memcpy(&bits, &value, sizeof(bits));
      output << '\t' << std::setw(16) << bits;
    }
  }
  return output.str();
}

struct StepXdeExportPart {
  std::string path;
  std::string name;
  TopoDS_Shape shape;
  TDF_Label label;
};

struct StepXdeExportNode {
  std::int32_t parent_id;
  std::int32_t part_index;
  std::string name;
  bool has_color;
  std::array<unsigned char, 3> color;
  gp_Trsf transform;
};

bool hex_decode(const std::string& encoded, std::string& decoded) {
  if (encoded.size() % 2 != 0) {
    return false;
  }
  const auto nibble = [](char value) -> int {
    if (value >= '0' && value <= '9') return value - '0';
    if (value >= 'a' && value <= 'f') return value - 'a' + 10;
    if (value >= 'A' && value <= 'F') return value - 'A' + 10;
    return -1;
  };
  decoded.clear();
  decoded.reserve(encoded.size() / 2);
  for (std::size_t index = 0; index < encoded.size(); index += 2) {
    const int high = nibble(encoded[index]);
    const int low = nibble(encoded[index + 1]);
    if (high < 0 || low < 0) {
      return false;
    }
    decoded.push_back(static_cast<char>((high << 4) | low));
  }
  return true;
}

std::vector<std::string> split_tabs(const std::string& line) {
  std::vector<std::string> fields;
  std::size_t start = 0;
  while (true) {
    const std::size_t end = line.find('\t', start);
    fields.push_back(line.substr(start, end - start));
    if (end == std::string::npos) break;
    start = end + 1;
  }
  return fields;
}

bool parse_i32(const std::string& value, std::int32_t& parsed) {
  try {
    std::size_t consumed = 0;
    const long long number = std::stoll(value, &consumed, 10);
    if (consumed != value.size()
        || number < std::numeric_limits<std::int32_t>::min()
        || number > std::numeric_limits<std::int32_t>::max()) {
      return false;
    }
    parsed = static_cast<std::int32_t>(number);
    return true;
  } catch (...) {
    return false;
  }
}

bool parse_export_color(
    const std::string& value, bool& has_color, std::array<unsigned char, 3>& color) {
  if (value == "-") {
    has_color = false;
    color = {0, 0, 0};
    return true;
  }
  std::string bytes;
  if (value.size() != 6 || !hex_decode(value, bytes) || bytes.size() != 3) {
    return false;
  }
  has_color = true;
  color = {
      static_cast<unsigned char>(bytes[0]),
      static_cast<unsigned char>(bytes[1]),
      static_cast<unsigned char>(bytes[2])};
  return true;
}

bool parse_export_transform(
    const std::vector<std::string>& fields, std::size_t start, gp_Trsf& transform) {
  if (fields.size() != start + 16) {
    return false;
  }
  std::array<double, 16> matrix{};
  for (std::size_t index = 0; index < matrix.size(); ++index) {
    try {
      std::size_t consumed = 0;
      const std::uint64_t bits = std::stoull(fields[start + index], &consumed, 16);
      if (consumed != fields[start + index].size()) return false;
      std::memcpy(&matrix[index], &bits, sizeof(bits));
      if (!std::isfinite(matrix[index])) return false;
    } catch (...) {
      return false;
    }
  }
  const double epsilon = tolerances().rounding;
  if (std::abs(matrix[12]) > epsilon || std::abs(matrix[13]) > epsilon
      || std::abs(matrix[14]) > epsilon || std::abs(matrix[15] - 1.0) > epsilon) {
    return false;
  }
  try {
    transform.SetValues(
        matrix[0], matrix[1], matrix[2], matrix[3],
        matrix[4], matrix[5], matrix[6], matrix[7],
        matrix[8], matrix[9], matrix[10], matrix[11]);
    return transform.Form() != gp_Other;
  } catch (...) {
    return false;
  }
}

bool parse_step_xde_export(
    const std::string& manifest,
    std::vector<StepXdeExportPart>& parts,
    std::vector<StepXdeExportNode>& nodes,
    std::string& error) {
  std::istringstream input(manifest);
  std::string line;
  if (!std::getline(input, line) || line != "KETCHUP_STEP_XDE_EXPORT_V1") {
    error = "STEP XDE export manifest has an unsupported schema";
    return false;
  }
  while (std::getline(input, line)) {
    if (!line.empty() && line.back() == '\r') line.pop_back();
    if (line.empty()) continue;
    const auto fields = split_tabs(line);
    if (fields[0] == "P" && fields.size() == 3) {
      if (parts.size() >= MAX_STEP_XDE_NODES) {
        error = "STEP XDE export part count exceeds the bounded envelope";
        return false;
      }
      StepXdeExportPart part;
      if (!hex_decode(fields[1], part.path) || !hex_decode(fields[2], part.name)
          || part.path.empty() || part.name.empty()) {
        error = "STEP XDE export part is malformed";
        return false;
      }
      parts.push_back(std::move(part));
      continue;
    }
    if (fields[0] == "N" && fields.size() == 21) {
      if (nodes.size() >= MAX_STEP_XDE_NODES) {
        error = "STEP XDE export node count exceeds the bounded envelope";
        return false;
      }
      StepXdeExportNode node;
      if (!parse_i32(fields[1], node.parent_id)
          || !parse_i32(fields[2], node.part_index)
          || !hex_decode(fields[3], node.name)
          || node.name.empty()
          || !parse_export_color(fields[4], node.has_color, node.color)
          || !parse_export_transform(fields, 5, node.transform)) {
        error = "STEP XDE export node is malformed";
        return false;
      }
      const std::int32_t node_id = static_cast<std::int32_t>(nodes.size());
      if (node.parent_id >= node_id || node.parent_id < -1
          || node.part_index < -1
          || node.part_index >= static_cast<std::int32_t>(parts.size())) {
        error = "STEP XDE export hierarchy or part reference is invalid";
        return false;
      }
      if (node.parent_id >= 0
          && nodes[static_cast<std::size_t>(node.parent_id)].part_index >= 0) {
        error = "STEP XDE export part node cannot contain children";
        return false;
      }
      std::size_t depth = 0;
      for (std::int32_t parent = node.parent_id; parent >= 0;
           parent = nodes[static_cast<std::size_t>(parent)].parent_id) {
        if (++depth > MAX_STEP_XDE_DEPTH) {
          error = "STEP XDE export hierarchy exceeds the bounded depth";
          return false;
        }
      }
      nodes.push_back(std::move(node));
      continue;
    }
    error = "STEP XDE export manifest row is malformed";
    return false;
  }
  if (parts.empty() || nodes.empty()) {
    error = "STEP XDE export manifest has no parts or nodes";
    return false;
  }
  for (std::size_t index = 0; index < parts.size(); ++index) {
    if (std::none_of(nodes.begin(), nodes.end(), [index](const StepXdeExportNode& node) {
          return node.part_index == static_cast<std::int32_t>(index);
        })) {
      error = "STEP XDE export contains an unreferenced part";
      return false;
    }
  }
  return true;
}

void set_xde_name(const TDF_Label& label, const std::string& name) {
  TDataStd_Name::Set(label, TCollection_ExtendedString(name.c_str(), true));
}

} // namespace

rust::String xde_manifest_native(rust::Str path, bool iges) noexcept {
  try {
    StepXdeData data;
    std::string error;
    const std::string native_path(path.data(), path.size());
    const bool loaded = iges
        ? load_iges_xde(native_path, data, error)
        : load_step_xde(native_path, data, error);
    if (native_path.empty() || !loaded) {
      return rust::String("ERR\t" + hex_encode(error.empty() ? "invalid STEP XDE path" : error));
    }
    std::ostringstream output;
    output << "KETCHUP_STEP_XDE_V1\n";
    for (std::size_t index = 0; index < data.parts.size(); ++index) {
      const TDF_Label& part = data.parts[index];
      std::string name = label_name(part);
      const bool name_from_source = !name.empty();
      if (!name_from_source) {
        name = "Imported STEP part";
      }
      output << "P\t" << index << '\t' << hex_encode(name) << '\t'
             << (name_from_source ? '1' : '0') << '\t'
             << label_color(data.colors, part, TDF_Label()) << '\n';
    }
    for (const StepXdeNode& node : data.nodes) {
      output << "N\t" << node.id << '\t' << node.parent_id << '\t' << node.part_index
             << '\t' << hex_encode(node.name) << '\t' << (node.name_from_source ? '1' : '0')
             << '\t' << node.color << matrix_fields(node.transform) << '\n';
    }
    return rust::String(output.str());
  } catch (const Standard_Failure& failure) {
    return rust::String("ERR\t" + hex_encode(failure.GetMessageString()));
  } catch (const std::exception& failure) {
    return rust::String("ERR\t" + hex_encode(failure.what()));
  } catch (...) {
    return rust::String("ERR\t" + hex_encode("unknown STEP XDE exception"));
  }
}

rust::String step_xde_manifest_native(rust::Str path) noexcept {
  return xde_manifest_native(path, false);
}

rust::String iges_xde_manifest_native(rust::Str path) noexcept {
  return xde_manifest_native(path, true);
}

template <typename Writer>
rust::String export_xde_assembly_native(
    rust::Str manifest, rust::Str path) noexcept {
  try {
    const std::string encoded(manifest.data(), manifest.size());
    const std::string output_path(path.data(), path.size());
    std::vector<StepXdeExportPart> parts;
    std::vector<StepXdeExportNode> nodes;
    std::string error;
    if (output_path.empty() || !parse_step_xde_export(encoded, parts, nodes, error)) {
      return rust::String(error.empty() ? "invalid STEP XDE export path" : error);
    }

    const occ::handle<TDocStd_Document> document =
        new TDocStd_Document(TCollection_ExtendedString("BinXCAF"));
    const occ::handle<XCAFDoc_ShapeTool> shapes =
        XCAFDoc_DocumentTool::ShapeTool(document->Main());
    const occ::handle<XCAFDoc_ColorTool> colors =
        XCAFDoc_DocumentTool::ColorTool(document->Main());
    if (shapes.IsNull() || colors.IsNull()) {
      return rust::String("STEP XDE document tools are unavailable");
    }

    for (StepXdeExportPart& part : parts) {
      STEPControl_Reader reader;
      if (reader.ReadFile(part.path.c_str()) != IFSelect_RetDone
          || reader.TransferRoots() == 0) {
        return rust::String("STEP XDE writer could not reread an exact part source");
      }
      part.shape = reader.OneShape();
      if (part.shape.IsNull()) {
        return rust::String("STEP XDE writer received a null exact part shape");
      }
      if constexpr (std::is_same_v<Writer, STEPCAFControl_Writer>) {
        part.label = shapes->AddShape(part.shape, false);
        if (part.label.IsNull()) {
          return rust::String("STEP XDE writer could not register an exact part definition");
        }
        set_xde_name(part.label, part.name);
      }
    }

    if constexpr (std::is_same_v<Writer, STEPCAFControl_Writer>) {
      std::vector<TDF_Label> node_definitions(nodes.size());
    for (std::size_t index = 0; index < nodes.size(); ++index) {
      const StepXdeExportNode& node = nodes[index];
      if (node.part_index >= 0) {
        node_definitions[index] = parts[static_cast<std::size_t>(node.part_index)].label;
      } else {
        node_definitions[index] = shapes->NewShape();
        if (node_definitions[index].IsNull()) {
          return rust::String("STEP XDE writer could not create an assembly definition");
        }
        set_xde_name(node_definitions[index], node.name);
      }
    }

    const TDF_Label root = shapes->NewShape();
    if (root.IsNull()) {
      return rust::String("STEP XDE writer could not create the model root");
    }
    set_xde_name(root, "Ketchup model");
    for (std::size_t index = 0; index < nodes.size(); ++index) {
      const StepXdeExportNode& node = nodes[index];
      const TDF_Label parent = node.parent_id < 0
          ? root
          : node_definitions[static_cast<std::size_t>(node.parent_id)];
      const TDF_Label component = shapes->AddComponent(
          parent, node_definitions[index], TopLoc_Location(node.transform));
      if (component.IsNull()) {
        return rust::String("STEP XDE writer could not attach an assembly component");
      }
      set_xde_name(component, node.name);
      if (node.has_color) {
        const Quantity_Color color(
            static_cast<double>(node.color[0]) / 255.0,
            static_cast<double>(node.color[1]) / 255.0,
            static_cast<double>(node.color[2]) / 255.0,
            Quantity_TOC_sRGB);
        colors->SetColor(component, color, XCAFDoc_ColorGen);
        colors->SetColor(component, color, XCAFDoc_ColorSurf);
      }
    }
      shapes->UpdateAssemblies();
    } else {
      for (std::size_t index = 0; index < nodes.size(); ++index) {
        const StepXdeExportNode& node = nodes[index];
        if (node.part_index < 0) {
          continue;
        }
        gp_Trsf world = node.transform;
        for (std::int32_t parent = node.parent_id; parent >= 0;
             parent = nodes[static_cast<std::size_t>(parent)].parent_id) {
          gp_Trsf composed = nodes[static_cast<std::size_t>(parent)].transform;
          composed.Multiply(world);
          world = composed;
        }
        BRepBuilderAPI_Transform placed(
            parts[static_cast<std::size_t>(node.part_index)].shape, world, true);
        if (!placed.IsDone()) {
          return rust::String("IGES XDE writer could not bake an occurrence transform");
        }
        const TDF_Label occurrence = shapes->AddShape(placed.Shape(), false);
        if (occurrence.IsNull()) {
          return rust::String("IGES XDE writer could not register an occurrence solid");
        }
        set_xde_name(occurrence, node.name);
        if (node.has_color) {
          const Quantity_Color color(
              static_cast<double>(node.color[0]) / 255.0,
              static_cast<double>(node.color[1]) / 255.0,
              static_cast<double>(node.color[2]) / 255.0,
              Quantity_TOC_sRGB);
          colors->SetColor(occurrence, color, XCAFDoc_ColorGen);
          colors->SetColor(occurrence, color, XCAFDoc_ColorSurf);
        }
      }
    }

    std::unique_ptr<IgesBrepModeGuard> iges_mode;
    if constexpr (std::is_same_v<Writer, IGESCAFControl_Writer>) {
      iges_mode = std::make_unique<IgesBrepModeGuard>();
      if (!iges_mode->active()) {
        return rust::String("IGES XDE writer could not enable BRep mode");
      }
    }
    Writer writer;
    writer.SetColorMode(true);
    writer.SetNameMode(true);
    bool transferred = false;
    if constexpr (std::is_same_v<Writer, STEPCAFControl_Writer>) {
      transferred = writer.Transfer(document, STEPControl_AsIs);
    } else {
      transferred = writer.Transfer(document);
    }
    if (!transferred) {
      return rust::String("XDE writer could not transfer the assembly document");
    }
    bool written = false;
    if constexpr (std::is_same_v<Writer, STEPCAFControl_Writer>) {
      written = writer.Write(output_path.c_str()) == IFSelect_RetDone;
    } else {
      written = writer.Write(output_path.c_str());
    }
    if (!written) {
      return rust::String("XDE writer could not write the assembly document");
    }
    return rust::String();
  } catch (const Standard_Failure& failure) {
    return rust::String(failure.GetMessageString());
  } catch (const std::exception& failure) {
    return rust::String(failure.what());
  } catch (...) {
    return rust::String("unknown STEP XDE export exception");
  }
}

rust::String export_step_xde_assembly_native(
    rust::Str manifest, rust::Str path) noexcept {
  return export_xde_assembly_native<STEPCAFControl_Writer>(manifest, path);
}

rust::String export_iges_xde_assembly_native(
    rust::Str manifest, rust::Str path) noexcept {
  return export_xde_assembly_native<IGESCAFControl_Writer>(manifest, path);
}

std::unique_ptr<NativeOperationResult> import_xde_part_native(
    rust::Str path, std::uint32_t part_index, bool iges) noexcept {
  return guarded([&] {
    const std::string native_path(path.data(), path.size());
    if (iges) {
      IGESControl_Reader reader;
      if (reader.ReadFile(native_path.c_str()) != IFSelect_RetDone) {
        return error_result(STATUS_INVALID_PARAMETER, "IGES reader could not read the source");
      }
      const int roots = reader.NbRootsForTransfer();
      if (part_index >= static_cast<std::uint32_t>(std::max(roots, 0))
          || !reader.TransferOneRoot(static_cast<int>(part_index) + 1)
          || reader.NbShapes() == 0) {
        return error_result(STATUS_INVALID_PARAMETER, "IGES part index is outside the transferable roots");
      }
      const TopoDS_Shape shape = reader.Shape(reader.NbShapes());
      const std::uint32_t solids = count_subshapes(shape, TopAbs_SOLID);
      return success_result(shape, {}, solids >= 2, false, {}, solids == 0);
    }
    StepXdeData data;
    std::string error;
    const bool loaded = load_step_xde(native_path, data, error);
    if (!loaded) {
      return error_result(STATUS_INVALID_PARAMETER, error);
    }
    if (part_index >= data.parts.size()) {
      return error_result(STATUS_INVALID_PARAMETER, "STEP XDE part index is outside the manifest");
    }
    const TopoDS_Shape shape = XCAFDoc_ShapeTool::GetShape(data.parts[part_index]);
    if (shape.IsNull()) {
      return error_result(STATUS_INVALID_SHAPE, "STEP XDE part has no exact shape");
    }
    const std::uint32_t solids = count_subshapes(shape, TopAbs_SOLID);
    return success_result(shape, {}, solids >= 2, false, {}, solids == 0);
  });
}

std::unique_ptr<NativeOperationResult> import_step_xde_part_native(
    rust::Str path, std::uint32_t part_index) noexcept {
  return import_xde_part_native(path, part_index, false);
}

std::unique_ptr<NativeOperationResult> import_iges_xde_part_native(
    rust::Str path, std::uint32_t part_index) noexcept {
  return import_xde_part_native(path, part_index, true);
}

std::unique_ptr<NativeOperationResult> import_step_native(rust::Str path) noexcept {
  return guarded([&] {
    const std::string native_path(path.data(), path.size());
    STEPControl_Reader reader;
    if (reader.ReadFile(native_path.c_str()) != IFSelect_RetDone) {
      return error_result(STATUS_INVALID_PARAMETER, "STEP reader could not read the fixture");
    }
    if (reader.TransferRoots() == 0) {
      return error_result(STATUS_INVALID_SHAPE, "STEP fixture contains no transferable roots");
    }
    const TopoDS_Shape shape = reader.OneShape();
    const std::uint32_t solids = count_subshapes(shape, TopAbs_SOLID);
    return success_result(shape, {}, solids >= 2, false, {}, solids == 0);
  });
}

std::unique_ptr<NativeOperationResult> import_step_solid_native(
    rust::Str path, std::uint32_t solid_ordinal) noexcept {
  return guarded([&] {
    const std::string native_path(path.data(), path.size());
    STEPControl_Reader reader;
    if (reader.ReadFile(native_path.c_str()) != IFSelect_RetDone) {
      return error_result(STATUS_INVALID_PARAMETER, "STEP reader could not read the fixture");
    }
    if (reader.TransferRoots() == 0) {
      return error_result(STATUS_INVALID_SHAPE, "STEP fixture contains no transferable roots");
    }
    TopExp_Explorer explorer(reader.OneShape(), TopAbs_SOLID);
    for (std::uint32_t ordinal = 0; ordinal < solid_ordinal && explorer.More(); ++ordinal) {
      explorer.Next();
    }
    if (!explorer.More()) {
      return error_result(STATUS_INVALID_PARAMETER, "STEP solid ordinal is outside the transferred assembly");
    }
    return success_result(explorer.Current(), {});
  });
}

rust::String step_length_unit_native(rust::Str path) noexcept {
  try {
    const std::string native_path(path.data(), path.size());
    STEPControl_Reader reader;
    if (reader.ReadFile(native_path.c_str()) != IFSelect_RetDone) {
      return rust::String();
    }
    NCollection_Sequence<TCollection_AsciiString> length_units;
    NCollection_Sequence<TCollection_AsciiString> angle_units;
    NCollection_Sequence<TCollection_AsciiString> solid_angle_units;
    reader.FileUnits(length_units, angle_units, solid_angle_units);
    if (length_units.Length() != 1) {
      return rust::String();
    }
    std::string unit(length_units.Value(1).ToCString());
    std::transform(unit.begin(), unit.end(), unit.begin(), [](unsigned char character) {
      return static_cast<char>(std::tolower(character));
    });
    return rust::String(unit);
  } catch (...) {
    return rust::String();
  }
}

std::unique_ptr<NativeOperationResult> import_iges_native(rust::Str path) noexcept {
  return guarded([&] {
    const std::string native_path(path.data(), path.size());
    IGESControl_Reader reader;
    if (reader.ReadFile(native_path.c_str()) != IFSelect_RetDone) {
      return error_result(STATUS_INVALID_PARAMETER, "IGES reader could not read the source");
    }
    if (reader.TransferRoots() == 0) {
      return error_result(STATUS_INVALID_SHAPE, "IGES source contains no transferable roots");
    }
    const TopoDS_Shape shape = reader.OneShape();
    const std::uint32_t solids = count_subshapes(shape, TopAbs_SOLID);
    return success_result(shape, {}, solids >= 2, false, {}, solids == 0);
  });
}

rust::String iges_length_unit_native(rust::Str path) noexcept {
  try {
    const std::string native_path(path.data(), path.size());
    IGESControl_Reader reader;
    if (reader.ReadFile(native_path.c_str()) != IFSelect_RetDone || reader.IGESModel().IsNull()) {
      return rust::String();
    }
    const auto unit_name = reader.IGESModel()->GlobalSection().UnitName();
    if (unit_name.IsNull()) {
      return rust::String();
    }
    std::string unit(unit_name->ToCString());
    std::transform(unit.begin(), unit.end(), unit.begin(), [](unsigned char character) {
      return static_cast<char>(std::tolower(character));
    });
    return rust::String(unit);
  } catch (...) {
    return rust::String();
  }
}

} // namespace ketchup::exact
