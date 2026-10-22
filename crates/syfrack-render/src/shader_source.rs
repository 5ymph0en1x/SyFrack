//! WGSL has no include; each module is the prelude, the generated distance
//! estimator (`graph_wgsl::compile`) and an entry-point body, concatenated.

pub const COMMON: &str = include_str!("shaders/common.wgsl");
pub const PATHTRACE_BODY: &str = include_str!("shaders/pathtrace.wgsl");
pub const PLANAR: &str = include_str!("shaders/planar.wgsl");
pub const PATTERN: &str = include_str!("shaders/pattern.wgsl");
pub const CREATION: &str = include_str!("shaders/creation.wgsl");
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
pub const GEODESIC: &str = include_str!("shaders/geodesic.wgsl");
pub const PHENOMENA: &str = include_str!("shaders/phenomena.wgsl");
pub const DE_PROBE_BODY: &str = include_str!("shaders/de_probe.wgsl");

/// Export scenes are immutable. On a software device, substitute only exact
/// zero-valued, read-only scene fields and the dimension. This lets its native
/// compiler discard absent transports without changing the integrator, numeric
/// parameters of enabled features, camera coordinates or sample scheduling.
#[derive(Clone, Copy)]
pub(crate) struct CpuExportConstants(pub u16, pub [u32; 2]);

const ZERO_FIELDS: [(&str, &str, &str); 11] = [
    ("u.atmosphere.a.x", "f32", "0.0"),
    ("u.atmosphere.ground", "vec4f", "vec4f(0.0)"),
    ("u.atmosphere.dunes", "vec4f", "vec4f(0.0)"),
    ("u.atmosphere.portal", "vec4f", "vec4f(0.0)"),
    ("u.seascape", "Seascape", "Seascape()"),
    ("u.seascape.ocean_wave", "vec4f", "vec4f(0.0)"),
    ("u.surface", "SurfaceVariation", "SurfaceVariation()"),
    ("u.texture", "FractalTexture", "FractalTexture()"),
    ("u.finish_count", "vec4u", "vec4u(0u)"),
    ("u.finishes", "array<Finish,8>", "array<Finish,8>()"),
    ("u.coloring", "Coloring", "Coloring()"),
];

impl CpuExportConstants {
    pub fn from_scene(scene: &syfrack_core::Scene) -> Self {
        let u = crate::Uniforms::from_scene(scene, 1, 1, 1, 1);
        let zero = |bytes: &[u8]| bytes.iter().all(|b| *b == 0);
        let fields = [
            u.atmosphere.a[0].to_bits() == 0,
            zero(bytemuck::bytes_of(&u.atmosphere.ground)),
            zero(bytemuck::bytes_of(&u.atmosphere.dunes)),
            zero(bytemuck::bytes_of(&u.atmosphere.portal)),
            zero(bytemuck::bytes_of(&u.seascape)),
            zero(bytemuck::bytes_of(&u.seascape.ocean_wave)),
            zero(bytemuck::bytes_of(&u.surface)),
            zero(bytemuck::bytes_of(&u.texture)),
            zero(bytemuck::bytes_of(&u.finish_count)),
            zero(bytemuck::bytes_of(&u.finishes)),
            zero(bytemuck::bytes_of(&u.coloring)),
        ];
        let mut mask = u16::from(scene.dimension == syfrack_core::Dimension::TwoD);
        for (i, fixed) in fields.into_iter().enumerate() {
            mask |= u16::from(fixed) << (i + 1);
        }
        Self(mask, [4, 1])
    }

    pub fn apply(&self, source: &str) -> String {
        let mut out = replace_field(
            source,
            "(u.flags & 1u)",
            if self.0 & 1 != 0 { "1u" } else { "0u" },
        );
        let mut helpers = String::new();
        for (i, (field, ty, value)) in ZERO_FIELDS.iter().enumerate() {
            if self.0 & (1 << (i + 1)) != 0 {
                let name = format!("cpu_export_zero_{i}");
                out = replace_field(&out, field, &format!("{name}()"));
                // Function evaluation preserves WGSL's runtime expression
                // semantics in unreachable defaults (e.g. disabled parallax's
                // 0/0). The native compiler can inline and eliminate them.
                helpers.push_str(&format!("\nfn {name}() -> {ty} {{ return {value}; }}\n"));
            }
        }
        let uniform = "@group(0) @binding(0) var<uniform> u: Uniforms;";
        let at = out.find(uniform).expect("transport uniform declaration") + uniform.len();
        out.insert_str(at, &helpers);
        out
    }
}

fn replace_field(source: &str, field: &str, value: &str) -> String {
    let identifier = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut out = String::with_capacity(source.len());
    let mut previous = 0;
    for (start, _) in source.match_indices(field) {
        let end = start + field.len();
        let line = source[..start].rfind('\n').map_or(0, |i| i + 1);
        // In particular, scalar .x must not replace a vector .xyz expression.
        if source[line..start].contains("//")
            || (start > 0
                && (identifier(source.as_bytes()[start - 1])
                    || source.as_bytes()[start - 1] == b'.'))
            || source.as_bytes().get(end).is_some_and(|c| identifier(*c))
        {
            continue;
        }
        out.push_str(&source[previous..start]);
        out.push_str(value);
        previous = end;
    }
    out.push_str(&source[previous..]);
    out
}

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
    let transport = PATHTRACE_BODY;
    format!(
        "{COMMON}
{PHENOMENA}
{CREATION}
{GEODESIC}
{de}
fn scene_sample(p:vec3f)->DeSample {{ return nature_surface(p,de_sample(p)); }}
fn scene_sample_exact(p:vec3f)->DeSample {{ return nature_surface(p,de_sample_exact(p)); }}
{texture}
{PLANAR}
{PATTERN}
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
{transport}"
    )
}

/// The distance-field probe used by the CPU/GPU consistency harness.
pub fn de_probe(de: &str) -> String {
    format!(
        "{COMMON}
{PHENOMENA}
{PLANAR}
{PATTERN}
{CREATION}
fn texture_de_sample(p:vec3f)->DeSample {{ return DeSample(0.0,vec4f(1.0),0u,0u,0u); }}
{GEODESIC}
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
    fn cpu_frozen_scenes_validate_and_only_freeze_absent_features() {
        let clear = syfrack_core::Scene::example();
        let mut fog = clear.clone();
        fog.atmosphere.fog_density = 0.1;
        let mut thicker = fog.clone();
        thicker.atmosphere.fog_density = 0.2;
        let clear_constants = CpuExportConstants::from_scene(&clear);
        let fog_constants = CpuExportConstants::from_scene(&fog);
        assert_ne!(clear_constants.0, fog_constants.0);
        assert_eq!(fog_constants.0, CpuExportConstants::from_scene(&thicker).0);
        for scene in [clear, fog, syfrack_core::Scene::example_2d()] {
            let constants = CpuExportConstants::from_scene(&scene);
            let src = constants.apply(&pathtrace(DE));
            let module = wgpu::naga::front::wgsl::parse_str(&src)
                .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&src)));
            wgpu::naga::valid::Validator::new(
                wgpu::naga::valid::ValidationFlags::all(),
                wgpu::naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .expect("frozen CPU shader validates");
            assert!(!src.contains("(u.flags & 1u)"));
        }
        assert_eq!(
            replace_field(
                "u.atmosphere.a.xyz + u.atmosphere.a.x + myu.atmosphere.a.x",
                "u.atmosphere.a.x",
                "0.0"
            ),
            "u.atmosphere.a.xyz + 0.0 + myu.atmosphere.a.x"
        );
    }

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
        assert!(COMMON.contains("var graph_params_overflow: texture_2d<f32>"));
        assert!(COMMON.contains(&format!(
            "graph_params: array<vec4f, {}>",
            crate::params::UNIFORM_SCALARS / 4
        )));
        assert!(TEXTURE.contains("var texture_params_overflow: texture_2d<f32>"));
    }
}
