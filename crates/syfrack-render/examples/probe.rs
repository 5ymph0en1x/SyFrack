fn main() -> anyhow::Result<()> {
    let ctx = syfrack_render::GpuContext::new()?;
    println!("adapter : {}", ctx.info.name);
    println!("backend : {:?}", ctx.info.backend);
    println!("driver  : {} {}", ctx.info.driver, ctx.info.driver_info);
    let out = ctx.run_probe()?;
    println!("probe   : {:?} ... {:?}", &out[..4], &out[252..]);
    Ok(())
}
