use syfrack_render::GpuContext;

#[test]
#[ignore = "needs a GPU adapter; run with `cargo test -- --include-ignored`"]
fn probe_runs_a_compute_shader_on_this_gpu() {
    let ctx = GpuContext::new().expect("no usable GPU backend");
    eprintln!("adapter: {} ({:?})", ctx.info.name, ctx.info.backend);
    let out = ctx.run_probe().expect("probe dispatch failed");
    assert_eq!(out.len(), 256);
    for (i, v) in out.iter().enumerate() {
        assert_eq!(*v, (i as u32) * 2 + 1, "element {i}");
    }
}
