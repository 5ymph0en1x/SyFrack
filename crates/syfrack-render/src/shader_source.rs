//! WGSL has no include; each module is the prelude, the generated distance
//! estimator (`graph_wgsl::compile`) and an entry-point body, concatenated.

pub const COMMON: &str = include_str!("shaders/common.wgsl");
pub const PATHTRACE_BODY: &str = include_str!("shaders/pathtrace.wgsl");
pub const PLANAR: &str = include_str!("shaders/planar.wgsl");
pub const TEXTURE: &str = include_str!("shaders/texture.wgsl");
pub const FINISH_DETAIL: &str = include_str!("shaders/finish_detail.wgsl");
pub const SEASCAPE: &str = include_str!("shaders/seascape.wgsl");
pub const OCEAN: &str = include_str!("shaders/ocean.wgsl");
pub const LANDSCAPE: &str = include_str!("shaders/landscape.wgsl");
pub const PORTAL: &str = include_str!("shaders/portal.wgsl");
pub const LIGHTS: &str = include_str!("shaders/lights.wgsl");
pub const ENVIRONMENT: &str = include_str!("shaders/environment.wgsl");
pub const GUIDING: &str = include_str!("shaders/guiding.wgsl");
pub const NEBULA: &str = include_str!("shaders/nebula.wgsl");
pub const OPTICS: &str = include_str!("shaders/optics.wgsl");
pub const DE_PROBE_BODY: &str = include_str!("shaders/de_probe.wgsl");

/// FNV-1a with a fixed seed, rather than a platform-dependent default hasher.
/// Node names are emitted as comments; renaming them must not invalidate a
/// checkpoint made with a cached pipeline of the same graph topology.
pub(crate) fn transport_fingerprint(source: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for line in source.lines() {
        let code = line.split("//").next().unwrap().trim();
        if code.is_empty() {
            continue;
        }
        for byte in code.bytes().chain(std::iter::once(b'\n')) {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
        }
    }
    hash
}

/// The path tracer around the generated `de_sample` function `de`.
pub fn pathtrace(de: &str) -> String {
    pathtrace_with_texture(de, None)
}

pub fn pathtrace_with_texture(de: &str, texture: Option<&str>) -> String {
    pathtrace_with_nebula(de, texture, false)
}

pub fn pathtrace_with_nebula(de: &str, texture: Option<&str>, nebula: bool) -> String {
    pathtrace_with_features(de, texture, nebula, false)
}

pub fn pathtrace_with_features(
    de: &str,
    texture: Option<&str>,
    nebula: bool,
    optics: bool,
) -> String {
    let texture = texture.unwrap_or(
        "fn texture_de_sample(p: vec3f) -> DeSample { return DeSample(0.0, vec4f(1.0), 0u, 0u, 0u); }",
    );
    format!(
        "{COMMON}
{de}
{texture}
{PLANAR}
{TEXTURE}
{FINISH_DETAIL}
{ENVIRONMENT}
{LIGHTS}
{SEASCAPE}
{OCEAN}
{LANDSCAPE}
{PORTAL}
{GUIDING}
const NEBULA_ENABLED: bool = {nebula};
const OPTICS_ENABLED: bool = {optics};
{NEBULA}
{OPTICS}
{PATHTRACE_BODY}"
    )
}

/// The distance-field probe used by the CPU/GPU consistency harness.
pub fn de_probe(de: &str) -> String {
    format!(
        "{COMMON}
{de}
{DE_PROBE_BODY}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const DE: &str =
        "fn de_sample(p: vec3f) -> DeSample { return DeSample(length(p), vec4f(1.0), 0u, 0u, 0u); } fn de_sample_exact(p: vec3f) -> DeSample { return de_sample(p); }";

    #[test]
    fn checkpoint_identity_ignores_node_names_but_tracks_transport_changes() {
        let a = pathtrace(&format!("// node 1 sphere\n{DE}"));
        let b = pathtrace(&format!("// node 1 renamed\n{DE}"));
        assert_eq!(transport_fingerprint(&a), transport_fingerprint(&b));
        assert_ne!(
            transport_fingerprint(&a),
            transport_fingerprint(&a.replace(
                "fn fog_extent() -> f32 { return ray_extent(); }",
                "fn fog_extent() -> f32 { return ray_extent()*0.8; }"
            ))
        );
    }

    #[test]
    fn sources_are_assembled_from_common_plus_de_plus_body() {
        let pt = pathtrace(DE);
        assert!(pt.contains("struct Uniforms"));
        assert!(pt.contains("fn de_sample("));
        assert!(pt.contains("@compute"));
        assert_eq!(pt.matches("struct Uniforms").count(), 1);
        assert_eq!(pt.matches("fn de_sample(").count(), 1);
    }

    #[test]
    fn de_probe_source_has_the_prelude_the_estimator_and_entry() {
        let src = de_probe(DE);
        assert!(src.contains("struct Uniforms"));
        assert!(src.contains("var<uniform> graph_params"));
        assert!(src.contains("fn de_sample("));
        assert!(src.contains("@compute"));
        assert!(src.contains("var<storage, read> points"));
        assert!(src.contains("results_traps"));
        assert_eq!(src.matches("struct Uniforms").count(), 1);
    }

    #[test]
    fn the_prelude_no_longer_holds_hand_written_estimators() {
        // Spelled in pieces so a grep for the removed names finds nothing.
        for gone in [
            concat!("fn de_", "mandelbulb("),
            concat!("fn de_", "mandel", "box("),
            concat!("fn de_", "menger("),
            concat!("fn de_", "sphere("),
        ] {
            assert!(!COMMON.contains(gone), "{gone} still in common.wgsl");
        }
        assert!(!COMMON.contains("fn de_sample("));
    }

    #[test]
    fn both_assembled_shaders_parse() {
        for src in [
            pathtrace(DE),
            pathtrace_with_nebula(DE, None, true),
            pathtrace_with_features(DE, None, true, true),
            de_probe(DE),
        ] {
            let module = wgpu::naga::front::wgsl::parse_str(&src).expect("wgsl parses");
            wgpu::naga::valid::Validator::new(
                wgpu::naga::valid::ValidationFlags::all(),
                wgpu::naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .expect("WGSL types and bindings validate");
        }
    }

    #[test]
    fn rust_and_wgsl_constants_agree() {
        use crate::uniforms::*;
        let pairs = [
            ("KIND_SUN", KIND_SUN),
            ("KIND_AREA", KIND_AREA),
            ("KIND_ENV", KIND_ENV),
            ("COLOR_NONE", COLOR_NONE),
            ("COLOR_SLOT_ITER", COLOR_SLOT_ITER),
            ("COLOR_SLOT_NORMAL", COLOR_SLOT_NORMAL),
        ];
        for (name, value) in pairs {
            assert!(
                COMMON.contains(&format!("const {name}: u32 = {value}u;")),
                "{name} = {value} missing from common.wgsl"
            );
        }
        assert!(!COMMON.contains(concat!("FRACTAL", "_")));
        let packed = crate::graph_wgsl::PARAM_COUNT / 4;
        assert!(COMMON.contains(&format!("graph_params: array<vec4f, {packed}>")));
        assert!(TEXTURE.contains(&format!("texture_params: array<vec4f, {packed}>")));
    }
}
