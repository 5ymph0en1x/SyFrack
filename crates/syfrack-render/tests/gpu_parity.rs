use std::sync::Arc;
use syfrack_render::{GpuContext, Renderer};

/// Deliberately a strict release gate, not a tolerance test or a default CI
/// prerequisite. Native WGSL and the polynomial candidates are not qualified
/// for cross-device bit-exact rendering until this gate passes in full.
#[test]
#[ignore = "strict arithmetic release gate; needs GPU and 100000 points per bundled graph"]
fn parity_lockstep_test() {
    let renderer = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let report = syfrack_render::parity::audit(&renderer, 100_000, 11, &mut |g| {
        eprintln!(
            "{}: distance differences {}/{}",
            g.graph, g.distance_mismatches, g.points
        );
    })
    .unwrap();
    for r in &report.arithmetic {
        eprintln!("{}: {} bit differences", r.operation, r.mismatches);
    }
    report.require_bit_exact().unwrap();
}
