//! Compiled path-tracer and probe pipelines for one graph topology, and a
//! small LRU cache of them keyed by the topology fingerprint.

use crate::graph_wgsl::{Compiled, ParamLayout};
use crate::renderer::Layouts;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

pub const CACHE_SIZE: usize = 8;

pub struct CompiledPipeline {
    pub pipeline: wgpu::ComputePipeline,
    /// Specialization and dispatch must use the same workgroup dimensions.
    pub workgroup: [u32; 2],
    pub probe_pipeline: wgpu::ComputePipeline,
    pub layout: ParamLayout,
    pub texture_layout: Option<ParamLayout>,
    pub fingerprint: u64,
    /// Stable identity of the assembled transport shader, for checkpoints.
    pub shader_fingerprint: u64,
    /// Actual module, including immutable CPU specialization, for resume checks.
    pub(crate) implementation_fingerprint: u64,
    pub wgsl: String,
    pathtrace_module: wgpu::ShaderModule,
    guiding: Mutex<Option<Arc<GuidingPipelines>>>,
    pub(crate) export_source: String,
    pub(crate) wavefront: Mutex<Option<Arc<crate::wavefront::Pipelines>>>,
    export_pipeline: Mutex<Option<Arc<wgpu::ComputePipeline>>>,
}

pub(crate) struct GuidingPipelines {
    pub training: wgpu::ComputePipeline,
    pub sampling: wgpu::ComputePipeline,
}

fn pathtrace_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    module: &wgpu::ShaderModule,
    training: bool,
    sampling: bool,
    workgroup: [u32; 2],
) -> wgpu::ComputePipeline {
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(if training {
            "pathtrace training"
        } else if sampling {
            "pathtrace guided"
        } else {
            "pathtrace BSDF"
        }),
        layout: Some(layout),
        module,
        entry_point: Some("main"),
        compilation_options: wgpu::PipelineCompilationOptions {
            constants: &[
                ("GUIDE_TRAINING", f64::from(training)),
                ("GUIDE_SAMPLING", f64::from(sampling)),
                ("PATH_WORKGROUP_X", f64::from(workgroup[0])),
                ("PATH_WORKGROUP_Y", f64::from(workgroup[1])),
            ],
            ..Default::default()
        },
        cache: None,
    })
}

impl CompiledPipeline {
    /// Builds both pipelines; callable from any thread (wgpu resources are Send + Sync).
    pub fn build(
        device: &wgpu::Device,
        layouts: &Layouts,
        compiled: Compiled,
    ) -> anyhow::Result<Self> {
        Self::build_textured(device, layouts, compiled, None)
    }

    pub fn build_textured(
        device: &wgpu::Device,
        layouts: &Layouts,
        compiled: Compiled,
        texture: Option<Compiled>,
    ) -> anyhow::Result<Self> {
        Self::build_with_nebula(device, layouts, compiled, texture, false)
    }

    pub fn build_with_nebula(
        device: &wgpu::Device,
        layouts: &Layouts,
        compiled: Compiled,
        texture: Option<Compiled>,
        nebula: bool,
    ) -> anyhow::Result<Self> {
        Self::build_with_features(device, layouts, compiled, texture, nebula, false)
    }

    pub fn build_with_features(
        device: &wgpu::Device,
        layouts: &Layouts,
        compiled: Compiled,
        texture: Option<Compiled>,
        nebula: bool,
        optics: bool,
    ) -> anyhow::Result<Self> {
        Self::build_specialized(device, layouts, compiled, texture, nebula, optics, None, 0)
    }

    pub(crate) fn build_specialized(
        device: &wgpu::Device,
        layouts: &Layouts,
        compiled: Compiled,
        texture: Option<Compiled>,
        nebula: bool,
        optics: bool,
        constants: Option<&crate::shader_source::CpuExportConstants>,
        nature: u8,
    ) -> anyhow::Result<Self> {
        let workgroup = if let Some(constants) = constants {
            constants.1
        } else {
            match std::env::var("SYFRACK_WORKGROUP").as_deref() {
                Ok("8x8") | Err(std::env::VarError::NotPresent) => [8, 8],
                Ok("16x4") => [16, 4],
                Ok("32x2") => [32, 2],
                _ => anyhow::bail!("SYFRACK_WORKGROUP must be 8x8, 16x4 or 32x2"),
            }
        };
        // Nested scopes, popped innermost first: a backend (Internal) or
        // out-of-memory failure must fail the build like a validation error,
        // so it is never cached nor swapped in.
        let oom = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let make = |label: &str, src: String, layout: &wgpu::PipelineLayout| {
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(src.into()),
            });
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                module: &module,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let source = crate::shader_source::pathtrace_with_features(
            &compiled.wgsl,
            texture.as_ref().map(|t| t.wgsl.as_str()),
            nebula,
            optics,
        );
        let mut source = source;
        for (bit, name) in [
            (1, "NATURE_FIELDS_ENABLED"),
            (2, "NATURE_FILM_ENABLED"),
            (4, "KERR_ENABLED"),
            (8, "DISPERSION_ENABLED"),
            (32, "NATURE_VOLUME_ENABLED"),
        ] {
            source = source.replace(
                &format!("const {name}: bool = true;"),
                &format!("const {name}: bool = {};", nature & bit != 0),
            );
        }
        // Projection is known before compilation. Removing the unreachable 3D
        // transport keeps the shared 2D field kernel small on older drivers.
        if nature & 16 != 0 {
            source = source.replace("if ((u.flags & 1u) != 0u)", "if (true)");
        }
        let shader_fingerprint = crate::shader_source::transport_fingerprint(&source);
        // Identity tracks the full transport. Substituted values equal the
        // immutable scene's uniforms, and have a separate, bounded cache.
        let module_source = constants.map_or_else(|| source.clone(), |c| c.apply(&source));
        let pathtrace_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pathtrace module"),
            source: wgpu::ShaderSource::Wgsl(module_source.clone().into()),
        });
        let compile_clock = std::time::Instant::now();
        if std::env::var_os("SYFRACK_TRACE_GPU").is_some() {
            eprintln!("compiling shared transport features={nature}");
        }
        let pipeline = pathtrace_pipeline(
            device,
            &layouts.pathtrace,
            &pathtrace_module,
            false,
            false,
            workgroup,
        );
        let probe_pipeline = make(
            "de_probe",
            crate::shader_source::de_probe(&compiled.wgsl),
            &layouts.probe,
        );
        if std::env::var_os("SYFRACK_TRACE_GPU").is_some() {
            eprintln!("compiled shared transport in {:?}", compile_clock.elapsed());
        }
        let validation = pollster::block_on(validation.pop());
        let internal = pollster::block_on(internal.pop());
        let oom = pollster::block_on(oom.pop());
        if let Some(err) = validation.or(internal).or(oom) {
            let text = err.to_string();
            let node = crate::graph_wgsl::node_of_error(&text, &module_source);
            anyhow::bail!(
                "shader compilation failed{}: {text}",
                node.map(|n| format!(" near node {n}")).unwrap_or_default()
            );
        }
        let fingerprint = crate::graph_wgsl::optics_fingerprint(
            crate::graph_wgsl::nebula_fingerprint(
                crate::graph_wgsl::pipeline_fingerprint(
                    compiled.fingerprint,
                    texture.as_ref().map(|t| t.fingerprint),
                ),
                nebula,
            ),
            optics,
        );
        let fingerprint = crate::graph_wgsl::nature_fingerprint(fingerprint, nature);
        Ok(Self {
            pipeline,
            workgroup,
            probe_pipeline,
            layout: compiled.layout,
            texture_layout: texture.map(|t| t.layout),
            fingerprint,
            shader_fingerprint,
            implementation_fingerprint: crate::shader_source::transport_fingerprint(&module_source),
            wgsl: compiled.wgsl,
            pathtrace_module,
            guiding: Mutex::new(None),
            export_source: module_source,
            wavefront: Mutex::new(None),
            export_pipeline: Mutex::new(None),
        })
    }

    /// Export-only entry point is deliberately outside the transport identity:
    /// it never writes the actual accumulation, depth or saved normal buffers.
    pub(crate) fn export_pipeline(
        &self,
        device: &wgpu::Device,
        layouts: &Layouts,
    ) -> Arc<wgpu::ComputePipeline> {
        let mut cache = self
            .export_pipeline
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if let Some(pipeline) = cache.as_ref() {
            return pipeline.clone();
        }
        let source = format!(
            "{}\n{}",
            self.export_source,
            include_str!("shaders/export_guides.wgsl")
        );
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("material export guides"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let pipeline = Arc::new(
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("material export guides"),
                layout: Some(&layouts.pathtrace),
                module: &module,
                entry_point: Some("export_guides"),
                compilation_options: Default::default(),
                cache: None,
            }),
        );
        *cache = Some(pipeline.clone());
        pipeline
    }

    /// Only guided sessions compile the extra variants. Reuse them for this
    /// topology; plain interactive previews keep the single BSDF pipeline.
    pub(crate) fn guiding_pipelines(
        &self,
        device: &wgpu::Device,
        layouts: &Layouts,
    ) -> anyhow::Result<Arc<GuidingPipelines>> {
        let mut cache = self.guiding.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(pipelines) = cache.as_ref() {
            return Ok(pipelines.clone());
        }
        let oom = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let pipelines = GuidingPipelines {
            training: pathtrace_pipeline(
                device,
                &layouts.pathtrace,
                &self.pathtrace_module,
                true,
                true,
                self.workgroup,
            ),
            sampling: pathtrace_pipeline(
                device,
                &layouts.pathtrace,
                &self.pathtrace_module,
                false,
                true,
                self.workgroup,
            ),
        };
        let validation = pollster::block_on(validation.pop());
        let internal = pollster::block_on(internal.pop());
        let oom = pollster::block_on(oom.pop());
        if let Some(error) = validation.or(internal).or(oom) {
            anyhow::bail!("guiding shader specialization failed: {error}");
        }
        let pipelines = Arc::new(pipelines);
        *cache = Some(pipelines.clone());
        Ok(pipelines)
    }
}

/// A compilation requested with `Renderer::request`, polled with `Renderer::poll`.
pub struct CompileTicket {
    pub fingerprint: u64,
    pub(crate) rx: std::sync::mpsc::Receiver<anyhow::Result<Arc<CompiledPipeline>>>,
    /// The final status once delivered, so repeated polls agree.
    pub(crate) done: std::cell::RefCell<Option<CompileStatus>>,
}

/// The state of a requested compilation.
#[derive(Clone)]
pub enum CompileStatus {
    Pending,
    Ready(Arc<CompiledPipeline>),
    Failed(String),
}

/// A cache entry keyed by its topology fingerprint.
pub trait Fingerprinted {
    fn fingerprint(&self) -> u64;
}

impl Fingerprinted for CompiledPipeline {
    fn fingerprint(&self) -> u64 {
        self.fingerprint
    }
}

/// Least-recently-used cache of `CACHE_SIZE` entries keyed by fingerprint.
pub struct LruCache<T> {
    entries: Mutex<VecDeque<Arc<T>>>,
}

impl<T> Default for LruCache<T> {
    fn default() -> Self {
        Self {
            entries: Mutex::new(VecDeque::new()),
        }
    }
}

/// The renderer's cache of compiled pipelines.
pub type PipelineCache = LruCache<CompiledPipeline>;

impl<T: Fingerprinted> LruCache<T> {
    /// The entry for `fingerprint`, which becomes the most recently used.
    pub fn get(&self, fingerprint: u64) -> Option<Arc<T>> {
        let mut e = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        let pos = e.iter().position(|p| p.fingerprint() == fingerprint)?;
        let hit = e.remove(pos)?;
        e.push_back(hit.clone());
        Some(hit)
    }

    /// Inserts (or replaces) `p` as the most recently used entry, evicting
    /// the least recently used beyond `CACHE_SIZE`.
    pub fn insert(&self, p: Arc<T>) {
        let mut e = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        e.retain(|x| x.fingerprint() != p.fingerprint());
        e.push_back(p);
        while e.len() > CACHE_SIZE {
            e.pop_front();
        }
    }

    pub fn len(&self) -> usize {
        self.entries.lock().unwrap_or_else(|p| p.into_inner()).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    struct Fake(u64);
    impl Fingerprinted for Fake {
        fn fingerprint(&self) -> u64 {
            self.0
        }
    }

    #[test]
    fn the_least_recently_used_entry_is_evicted() {
        let cache: LruCache<Fake> = LruCache::default();
        for fp in 0..CACHE_SIZE as u64 {
            cache.insert(Arc::new(Fake(fp)));
        }
        assert_eq!(cache.len(), CACHE_SIZE);
        // A hit moves 0 to the back: 1 is now the oldest.
        let hit = cache.get(0).unwrap();
        assert_eq!(hit.0, 0);
        cache.insert(Arc::new(Fake(100)));
        assert_eq!(cache.len(), CACHE_SIZE);
        assert!(cache.get(1).is_none(), "the oldest entry was evicted");
        assert!(cache.get(0).is_some(), "the hit entry survived");
        assert!(cache.get(100).is_some());
        // Re-inserting a fingerprint replaces it without growing.
        cache.insert(Arc::new(Fake(100)));
        assert_eq!(cache.len(), CACHE_SIZE);
    }
}
