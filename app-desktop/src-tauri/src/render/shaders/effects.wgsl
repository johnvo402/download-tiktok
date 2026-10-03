struct Params { rect: vec4<f32>, step: vec4<f32>, effect: vec4<f32> }
@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var<uniform> p: Params;
struct Vertex { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> }
@vertex fn vertex(@builtin(vertex_index) i: u32) -> Vertex {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return Vertex(vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0), uv);
}
@fragment fn copy(v: Vertex) -> @location(0) vec4<f32> {
    return textureSample(src, samp, p.rect.xy + v.uv * p.rect.zw);
}
@fragment fn blur(v: Vertex) -> @location(0) vec4<f32> {
    var sum = vec4<f32>(0.0); var weight = 0.0;
    for(var i = -12; i <= 12; i++) {
        let w = exp(-0.5 * f32(i*i) / (p.effect.x*p.effect.x));
        sum += textureSample(src, samp, clamp(v.uv + p.step.xy * f32(i), vec2<f32>(0.0), vec2<f32>(1.0))) * w;
        weight += w;
    }
    return sum / weight;
}
@fragment fn layer(v: Vertex) -> @location(0) vec4<f32> {
    var color = textureSample(src, samp, v.uv);
    let pos = v.uv * p.rect.zw;
    let edge = min(min(pos.x, p.rect.z-pos.x), min(pos.y, p.rect.w-pos.y));
    let feather = select(1.0, smoothstep(0.0, max(p.effect.y, 0.001), edge), p.effect.y > 0.0);
    let noise = fract(sin(dot(floor(pos), vec2<f32>(12.9898,78.233))) * 43758.5453) - 0.5;
    color = vec4<f32>(clamp(color.rgb + noise*p.effect.z*0.2, vec3<f32>(0.0), vec3<f32>(1.0)), color.a * p.effect.w * feather);
    return color;
}
