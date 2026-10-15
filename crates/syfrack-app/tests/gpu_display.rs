use std::f32::consts::TAU;
use std::sync::Arc;
use syfrack_app_lib::display::DisplayPass;
use syfrack_color::develop;
use syfrack_color::encode::encode;
use syfrack_core::{HdrImage, Look, OutputSpace, Tonemap, Vignette, Zone};
use syfrack_render::GpuContext;

fn pattern(w: u32, h: u32) -> HdrImage {
    let mut img = HdrImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let l = 10f32.powf(-3.0 + 6.0 * x as f32 / (w - 1) as f32);
            let hue = y as f32 / (h - 1) as f32;
            let c = [
                l * (0.5 + 0.5 * (hue * TAU).cos()),
                l * (0.5 + 0.5 * (hue * TAU + TAU / 3.0).cos()),
                l * (0.5 + 0.5 * (hue * TAU + 2.0 * TAU / 3.0).cos()),
            ];
            img.set(x, y, [c[0], c[1], c[2], 1.0]);
        }
    }
    img
}

fn looks() -> Vec<Look> {
    let mut a = Look::neutral();
    a.grain = None;
    a.dither = false;
    let mut b = a.clone();
    b.exposure_ev = 1.5;
    b.temperature = 4000.0;
    b.tint = 0.1;
    b.shadows = Zone {
        offset: [0.0, 0.01, 0.02],
        gain: 0.2,
    };
    b.highlights = Zone {
        offset: [0.02, 0.0, 0.0],
        gain: -0.1,
    };
    b.tonemap = Tonemap::Agx {
        slope: 1.1,
        power: 1.2,
        offset: 0.01,
    };
    b.saturation = 1.2;
    b.vibrance = 0.3;
    b.vignette = Some(Vignette {
        radius: 0.5,
        softness: 0.6,
        intensity: 0.4,
        elliptical: false,
    });
    let mut c = a.clone();
    c.tonemap = Tonemap::Reinhard { white: 4.0 };
    let mut d = a.clone();
    d.exposure_ev = 0.5;
    d.vignette = Some(Vignette {
        radius: 0.4,
        softness: 0.7,
        intensity: 0.5,
        elliptical: true,
    });
    let mut electric = a.clone();
    electric.tonemap = Tonemap::Direct;
    let mut soft = electric.clone();
    soft.neon_softness = 0.6;
    vec![a, b, c, d, electric, soft]
}

/// Runs the display pass on `img` accumulated at `samples` per pixel (the
/// accumulator holds sums, alpha = sample count) and compares against the CPU.
fn check(img: &HdrImage, samples: u32, name: &str) {
    let (w, h) = (img.width, img.height);
    let ctx = Arc::new(GpuContext::new().unwrap());
    let device = &ctx.device;
    let mut pass = DisplayPass::new(device).unwrap();
    let sums: Vec<[f32; 4]> = img
        .data
        .iter()
        .map(|p| {
            let s = samples as f32;
            [p[0] * s, p[1] * s, p[2] * s, s]
        })
        .collect();
    let accum = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("test accum"),
        size: (w * h * 16) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    ctx.queue
        .write_buffer(&accum, 0, bytemuck::cast_slice(&sums));
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("test target"),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DisplayPass::FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let row = (w * 4).next_multiple_of(256);
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("test readback"),
        size: (row * h) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    for (i, look) in looks().iter().enumerate() {
        let gain = 2f32.powf(look.exposure_ev);
        let mut enc = device.create_command_encoder(&Default::default());
        pass.encode_to(
            device,
            &ctx.queue,
            &mut enc,
            &accum,
            (w, h),
            look,
            gain,
            &view,
        );
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        ctx.queue.submit([enc.finish()]);
        let slice = readback.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| tx.send(r).unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        rx.recv().unwrap().unwrap();
        let gpu: Vec<u8> = slice.get_mapped_range().unwrap().to_vec();
        readback.unmap();

        let cpu = encode(&develop(img, look), OutputSpace::Srgb, 8, false);
        let mut over1 = 0;
        let mut max_diff = 0i32;
        for y in 0..h as usize {
            for x in 0..w as usize {
                for ch in 0..3 {
                    let g = gpu[y * row as usize + x * 4 + ch] as i32;
                    let c = cpu.data[(y * w as usize + x) * 3 + ch] as i32;
                    let d = (g - c).abs();
                    max_diff = max_diff.max(d);
                    if d > 1 {
                        over1 += 1;
                    }
                }
            }
        }
        let channels = (w * h * 3) as usize;
        eprintln!(
            "{name} look {i}: max diff {max_diff}/255, {over1} of {channels} channels above 1/255"
        );
        assert!(max_diff <= 2, "{name} look {i}: max diff {max_diff}");
        assert!(
            over1 * 100 <= channels,
            "{name} look {i}: {over1} channels differ by more than 1/255"
        );
    }
}

#[test]
#[ignore = "needs GPU"]
fn gpu_display_matches_cpu_develop() {
    check(&pattern(64, 64), 1, "64x64 1spp");
}

#[test]
#[ignore = "needs GPU"]
fn gpu_display_matches_cpu_develop_wide_averaged() {
    // Non-square frame (elliptical vignette uses the aspect) and a.w = 3.
    check(&pattern(96, 54), 3, "96x54 3spp");
}
