# frozen_string_literal: true

require 'sketchup.rb'
require 'json'
require 'fileutils'

module Ketchup
  module SketchupBridge
    SCHEMA = 'ketchup.sketchup-scene.v1'
    EXTENSION = '.kscene'
    MAX_DEPTH = 32
    MAX_SOURCE_BYTES = 32 * 1024 * 1024
    MAX_DEFINITIONS = 128
    MAX_INSTANCES = 512
    MAX_TOTAL_VERTICES = 200_000
    MAX_TOTAL_TRIANGLES = 400_000
    MAX_VERTICES_PER_DEFINITION = 100_000
    MAX_TRIANGLES_PER_DEFINITION = 200_000

    module_function

    def export_active_model
      model = Sketchup.active_model
      path = UI.savepanel('Export Ketchup Scene', nil, default_name(model))
      return unless path

      path += EXTENSION unless File.extname(path).downcase == EXTENSION
      package = build_package(model)
      validate_package_limits!(package)
      json = JSON.generate(package)
      raise 'Export exceeds the 32 MiB bridge limit.' if json.bytesize + 1 > MAX_SOURCE_BYTES
      temporary = "#{path}.tmp-#{Process.pid}"
      File.open(temporary, 'wb') do |file|
        file.write(json)
        file.write("\n")
        file.flush
        file.fsync
      end
      FileUtils.mv(temporary, path, force: true)
      UI.messagebox("Ketchup scene exported to:\n#{path}")
    rescue StandardError => error
      File.delete(temporary) if defined?(temporary) && temporary && File.exist?(temporary)
      UI.messagebox("Ketchup scene export failed:\n#{error.message}")
    end

    def default_name(model)
      stem = if model.path && !model.path.empty?
               File.basename(model.path, File.extname(model.path))
             else
               'Untitled'
             end
      "#{stem}#{EXTENSION}"
    end

    def build_package(model)
      state = {
        definitions: {},
        instances: [],
        visited: {},
        material_assignments: 0,
        textures: 0,
        unsupported_entities: 0,
        total_vertices: 0,
        total_triangles: 0
      }
      walk_entities(
        model.entities,
        'root',
        model.title.to_s.empty? ? 'SketchUp model' : model.title.to_s,
        model.title.to_s.empty? ? 'SketchUp model' : model.title.to_s,
        Geom::Transformation.new,
        true,
        state,
        [],
        0
      )
      raise 'The active model contains no face geometry to export.' if state[:instances].empty?

      definitions = state[:definitions].values.sort_by { |definition| definition['id'] }
      instances = state[:instances].sort_by do |instance|
        [instance['definition'], instance['name'], instance['transform'], instance['visible'] ? 1 : 0]
      end
      {
        'schema' => SCHEMA,
        'units' => 'inch',
        'definitions' => definitions,
        'instances' => instances,
        'metadata' => {
          'material_assignments' => state[:material_assignments],
          'textures' => state[:textures],
          'tags' => model.layers.count { |layer| layer.name != 'Layer0' && layer.name != 'Untagged' },
          'scenes' => model.pages.length,
          'unsupported_entities' => state[:unsupported_entities]
        }
      }
    end

    def validate_package_limits!(package)
      definitions = package['definitions']
      instances = package['instances']
      raise "Export exceeds #{MAX_DEFINITIONS} definitions." if definitions.length > MAX_DEFINITIONS
      raise "Export exceeds #{MAX_INSTANCES} instances." if instances.length > MAX_INSTANCES

      total_vertices = 0
      total_triangles = 0
      definitions.each do |definition|
        vertices = definition['vertices'].length
        triangles = definition['triangles'].length
        if vertices > MAX_VERTICES_PER_DEFINITION
          raise "Definition #{definition['name']} exceeds #{MAX_VERTICES_PER_DEFINITION} vertices."
        end
        if triangles > MAX_TRIANGLES_PER_DEFINITION
          raise "Definition #{definition['name']} exceeds #{MAX_TRIANGLES_PER_DEFINITION} triangles."
        end
        total_vertices += vertices
        total_triangles += triangles
      end
      raise "Export exceeds #{MAX_TOTAL_VERTICES} total vertices." if total_vertices > MAX_TOTAL_VERTICES
      raise "Export exceeds #{MAX_TOTAL_TRIANGLES} total triangles." if total_triangles > MAX_TOTAL_TRIANGLES
    end

    def walk_entities(entities, key, definition_name, instance_name, world_transform, visible, state, path, depth)
      raise "Component nesting exceeds #{MAX_DEPTH} levels." if depth > MAX_DEPTH
      raise 'Recursive component definition is not supported.' if path.include?(key)

      ensure_container_meshes(entities, key, definition_name, state).each_with_index do |definition_id, index|
        suffix = state[:definitions_for_container][key].length > 1 ? " · solid #{index + 1}" : ''
        raise "Export exceeds #{MAX_INSTANCES} instances." if state[:instances].length >= MAX_INSTANCES

        state[:instances] << {
          'definition' => definition_id,
          'name' => safe_name("#{instance_name}#{suffix}", definition_id),
          'transform' => row_major_transform(world_transform),
          'visible' => visible
        }
      end

      sorted_entities(entities).each do |entity|
        case entity
        when Sketchup::Group
          child_key = "group:#{persistent_identity(entity)}"
          child_name = entity.name.to_s.empty? ? 'Group' : entity.name.to_s
          walk_entities(
            entity.entities,
            child_key,
            child_name,
            child_name,
            world_transform * entity.transformation,
            visible && drawing_element_visible?(entity),
            state,
            path + [key],
            depth + 1
          )
        when Sketchup::ComponentInstance
          definition = entity.definition
          child_key = "component:#{definition.guid}"
          definition_name = definition.name.to_s.empty? ? 'Component' : definition.name.to_s
          instance_name = entity.name.to_s.empty? ? definition_name : entity.name.to_s
          walk_entities(
            definition.entities,
            child_key,
            definition_name,
            instance_name,
            world_transform * entity.transformation,
            visible && drawing_element_visible?(entity),
            state,
            path + [key],
            depth + 1
          )
        end
      end
    end

    def ensure_container_meshes(entities, key, name, state)
      state[:definitions_for_container] ||= {}
      return state[:definitions_for_container][key] if state[:definitions_for_container].key?(key)

      faces = sorted_entities(entities).select do |entity|
        entity.is_a?(Sketchup::Face) && drawing_element_visible?(entity)
      end
      count_metadata_once(entities, key, state)
      if faces.empty?
        state[:definitions_for_container][key] = []
        return []
      end

      vertices, triangles = triangulate_faces(faces, state)
      shells = split_connected_shells(vertices, triangles)
      definition_ids = shells.each_with_index.map do |shell, index|
        definition_id = "#{key}:solid:#{index + 1}"
        shell_vertices, shell_triangles = compact_shell(vertices, shell)
        orient_consistently!(shell_triangles)
        orient_positive!(shell_vertices, shell_triangles)
        if state[:definitions].length >= MAX_DEFINITIONS
          raise "Export exceeds #{MAX_DEFINITIONS} definitions."
        end
        if shell_vertices.length > MAX_VERTICES_PER_DEFINITION
          raise "Definition #{name} exceeds #{MAX_VERTICES_PER_DEFINITION} vertices."
        end
        if shell_triangles.length > MAX_TRIANGLES_PER_DEFINITION
          raise "Definition #{name} exceeds #{MAX_TRIANGLES_PER_DEFINITION} triangles."
        end
        state[:total_vertices] += shell_vertices.length
        state[:total_triangles] += shell_triangles.length
        if state[:total_vertices] > MAX_TOTAL_VERTICES
          raise "Export exceeds #{MAX_TOTAL_VERTICES} total vertices."
        end
        if state[:total_triangles] > MAX_TOTAL_TRIANGLES
          raise "Export exceeds #{MAX_TOTAL_TRIANGLES} total triangles."
        end
        state[:definitions][definition_id] = {
          'id' => definition_id,
          'name' => safe_name(shells.length > 1 ? "#{name} · solid #{index + 1}" : name, definition_id),
          'vertices' => shell_vertices.map { |point| point.map { |value| normalized_number(value) } },
          'triangles' => shell_triangles
        }
        definition_id
      end
      state[:definitions_for_container][key] = definition_ids
    end

    def triangulate_faces(faces, state)
      vertices = []
      vertex_indices = {}
      triangles = []
      faces.each do |face|
        state[:material_assignments] += 1 if face.material
        state[:material_assignments] += 1 if face.back_material
        state[:textures] += 1 if face.material && face.material.texture
        state[:textures] += 1 if face.back_material && face.back_material.texture
        mesh = face.mesh(7)
        points = mesh.points
        mesh.polygons.each do |polygon|
          polygon_indices = polygon.map do |signed_index|
            point = points[signed_index.abs - 1]
            key = [normalized_number(point.x), normalized_number(point.y), normalized_number(point.z)]
            vertex_indices[key] ||= begin
              vertices << key
              vertices.length - 1
            end
          end
          next if polygon_indices.length < 3

          (1...(polygon_indices.length - 1)).each do |offset|
            triangles << [polygon_indices[0], polygon_indices[offset], polygon_indices[offset + 1]]
          end
        end
      end
      [vertices, triangles]
    end

    def split_connected_shells(_vertices, triangles)
      edge_to_triangles = Hash.new { |hash, edge| hash[edge] = [] }
      triangles.each_with_index do |triangle, triangle_index|
        [[triangle[0], triangle[1]], [triangle[1], triangle[2]], [triangle[2], triangle[0]]].each do |edge|
          edge_to_triangles[edge.sort] << triangle_index
        end
      end
      adjacency = Array.new(triangles.length) { [] }
      edge_to_triangles.each_value do |owners|
        owners.combination(2) do |left, right|
          adjacency[left] << right
          adjacency[right] << left
        end
      end
      visited = Array.new(triangles.length, false)
      shells = []
      triangles.each_index do |start|
        next if visited[start]

        queue = [start]
        visited[start] = true
        shell = []
        until queue.empty?
          current = queue.shift
          shell << triangles[current]
          adjacency[current].sort.each do |neighbor|
            next if visited[neighbor]

            visited[neighbor] = true
            queue << neighbor
          end
        end
        shells << shell
      end
      shells.sort_by { |shell| shell.flatten.min || 0 }
    end

    def compact_shell(vertices, triangles)
      used = triangles.flatten.uniq.sort
      remap = {}
      compact_vertices = used.each_with_index.map do |old_index, new_index|
        remap[old_index] = new_index
        vertices[old_index]
      end
      compact_triangles = triangles.map { |triangle| triangle.map { |index| remap[index] } }
      [compact_vertices, compact_triangles]
    end

    def orient_consistently!(triangles)
      edge_owners = Hash.new { |hash, edge| hash[edge] = [] }
      triangles.each_with_index do |triangle, triangle_index|
        [[triangle[0], triangle[1]], [triangle[1], triangle[2]], [triangle[2], triangle[0]]].each do |from, to|
          edge_owners[[from, to].sort] << [triangle_index, from < to ? 1 : -1]
        end
      end
      unless edge_owners.values.all? { |owners| owners.length == 2 }
        raise 'Face geometry is not a closed two-manifold solid.'
      end

      adjacency = Array.new(triangles.length) { [] }
      edge_owners.each_value do |owners|
        left, right = owners
        same_direction = left[1] == right[1]
        adjacency[left[0]] << [right[0], same_direction]
        adjacency[right[0]] << [left[0], same_direction]
      end
      flips = Array.new(triangles.length)
      triangles.each_index do |start|
        next unless flips[start].nil?

        flips[start] = false
        queue = [start]
        until queue.empty?
          current = queue.shift
          adjacency[current].sort_by(&:first).each do |neighbor, same_direction|
            required = flips[current] ^ same_direction
            if flips[neighbor].nil?
              flips[neighbor] = required
              queue << neighbor
            elsif flips[neighbor] != required
              raise 'Face winding is contradictory and cannot define a solid.'
            end
          end
        end
      end
      triangles.each_with_index do |triangle, index|
        triangle[1], triangle[2] = triangle[2], triangle[1] if flips[index]
      end
    end

    def orient_positive!(vertices, triangles)
      volume_times_six = triangles.sum do |triangle|
        a = vertices[triangle[0]]
        b = vertices[triangle[1]]
        c = vertices[triangle[2]]
        a[0] * (b[1] * c[2] - b[2] * c[1]) +
          a[1] * (b[2] * c[0] - b[0] * c[2]) +
          a[2] * (b[0] * c[1] - b[1] * c[0])
      end
      triangles.each { |triangle| triangle[1], triangle[2] = triangle[2], triangle[1] } if volume_times_six.negative?
    end

    def count_metadata_once(entities, key, state)
      return if state[:visited][key]

      state[:visited][key] = true
      sorted_entities(entities).each do |entity|
        if entity.is_a?(Sketchup::Face)
          state[:unsupported_entities] += 1 unless drawing_element_visible?(entity)
          next
        end
        next if entity.is_a?(Sketchup::ComponentInstance)
        next if entity.is_a?(Sketchup::Group)
        next if entity.is_a?(Sketchup::Edge) && !entity.faces.empty?

        state[:unsupported_entities] += 1
      end
    end

    def drawing_element_visible?(entity)
      return false unless entity.visible?
      return true unless entity.respond_to?(:layer)

      layer = entity.layer
      return true unless layer
      return false unless layer.visible?

      folder = layer.respond_to?(:folder) ? layer.folder : nil
      while folder
        return false unless folder.visible?

        folder = folder.respond_to?(:folder) ? folder.folder : nil
      end
      true
    end

    def row_major_transform(transformation)
      value = transformation.to_a
      [
        value[0], value[4], value[8], value[12],
        value[1], value[5], value[9], value[13],
        value[2], value[6], value[10], value[14],
        value[3], value[7], value[11], value[15]
      ].map { |number| normalized_number(number) }
    end

    def sorted_entities(entities)
      entities.to_a.sort_by { |entity| [persistent_identity(entity), entity.typename] }
    end

    def persistent_identity(entity)
      entity.respond_to?(:persistent_id) ? entity.persistent_id.to_i : entity.entityID.to_i
    end

    def safe_name(value, fallback)
      clean = value.to_s.encode(
        Encoding::UTF_8,
        invalid: :replace,
        undef: :replace,
        replace: '?'
      ).gsub(/[[:cntrl:]]/, ' ').strip
      clean = fallback if clean.empty?
      truncated = +''
      clean.each_char do |character|
        break if truncated.bytesize + character.bytesize > 1024

        truncated << character
      end
      truncated
    end

    def normalized_number(value)
      number = value.to_f
      number.zero? ? 0.0 : number
    end

    unless file_loaded?(__FILE__)
      UI.menu('File').add_item('Export Ketchup Scene…') { export_active_model }
      file_loaded(__FILE__)
    end
  end
end
