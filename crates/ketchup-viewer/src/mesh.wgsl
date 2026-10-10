struct View {
    view_projection: mat4x4<f32>,
    selection: vec4<f32>,
    section: vec4<f32>,
    options: vec4<f32>,
};
struct Instance {
    world_matrix: mat4x4<f32>,
    color: vec4<f32>,
    identity: vec4<f32>,
};
@group(0) @binding(0) var<uniform> view: View;
@group(1) @binding(0) var<uniform> instance: Instance;
struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
};
@vertex fn vertex_main(@location(0) position: vec3<f32>) -> VertexOut {
    var out: VertexOut;
    let source = vec3<f32>(position.x, -position.z, position.y) * 1000.0;
    let world = instance.world_matrix * vec4<f32>(source, 1.0);
    out.clip = view.view_projection * world;
    out.world = world.xyz;
    return out;
}
fn color() -> vec3<f32> {
    return select(instance.color.rgb, vec3<f32>(1.0, 0.65, 0.08),
        view.selection.x == instance.identity.x);
}
@fragment fn fragment_main(in: VertexOut) -> @location(0) vec4<f32> {
    if view.options.x > 0.0 && dot(in.world, view.section.xyz) > view.section.w { discard; }
    let normal = normalize(cross(dpdx(in.world), dpdy(in.world)));
    let light = 0.35 + 0.65 * abs(dot(normal, normalize(vec3<f32>(0.3, 0.5, 0.8))));
    return vec4<f32>(color() * light, 1.0);
}
@fragment fn fragment_edge(in: VertexOut) -> @location(0) vec4<f32> {
    if view.options.x > 0.0 && dot(in.world, view.section.xyz) > view.section.w { discard; }
    return vec4<f32>(color(), 1.0);
}
