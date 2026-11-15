//! GPU backend over wgpu: Metal (macOS), Vulkan (Linux, Windows), DX12 (Windows) or
//! OpenGL / OpenGL ES (Linux, Windows). WGSL shaders are translated by naga to MSL,
//! SPIR-V, HLSL or GLSL.

mod pathtrace;
mod quick;

use anyhow::{Context, Result, anyhow, bail};
use wgpu::util::DeviceExt;

use super::{Backend, BackendKind, HqFrame, QuickFrame};

/// The wgpu backend: Metal, Vulkan, DX12 or GL, chosen by the [`BackendKind`] it
/// is opened with. Quick mode runs a render pipeline; HQ mode runs the
/// path tracer as a WGSL compute shader.
pub struct GpuBackend {
    device: wgpu::Device,
    queue: wgpu::Queue,
    info: wgpu::AdapterInfo,
}

fn backends_for(kind: BackendKind) -> Result<wgpu::Backends> {
    Ok(match kind {
        BackendKind::Metal => wgpu::Backends::METAL,
        BackendKind::Vulkan => wgpu::Backends::VULKAN,
        BackendKind::Dx12 => wgpu::Backends::DX12,
        BackendKind::Gl => wgpu::Backends::GL,
        BackendKind::Gpu => wgpu::Backends::PRIMARY | wgpu::Backends::GL,
        other => bail!("{other:?} is not a wgpu backend"),
    })
}

/// The order adapters are preferred in: a discrete GPU, then an integrated one, then any
/// other hardware device; among equals, Metal, Vulkan, DX12, then GL.
fn rank(info: &wgpu::AdapterInfo) -> (u8, u8) {
    let device = match info.device_type {
        wgpu::DeviceType::DiscreteGpu => 0,
        wgpu::DeviceType::IntegratedGpu => 1,
        wgpu::DeviceType::VirtualGpu => 2,
        wgpu::DeviceType::Other => 3,
        wgpu::DeviceType::Cpu => 4,
    };
    let api = match info.backend {
        wgpu::Backend::Metal => 0,
        wgpu::Backend::Vulkan => 1,
        wgpu::Backend::Dx12 => 2,
        wgpu::Backend::Gl => 3,
        _ => 4,
    };
    (device, api)
}

fn request_adapter(kind: BackendKind) -> Result<wgpu::Adapter> {
    let backends = backends_for(kind)?;
    // wgpu's environment flags apply (`WGPU_VALIDATION`, `WGPU_DEBUG`,
    // `WGPU_ALLOW_UNDERLYING_NONCOMPLIANT_ADAPTER`, ...); the backends are ours.
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle_from_env();
    desc.backends = backends;
    let instance = wgpu::Instance::new(desc);
    let mut adapters = pollster::block_on(instance.enumerate_adapters(backends));
    adapters.sort_by_key(|a| rank(&a.get_info()));
    let software: Vec<String> = adapters
        .iter()
        .map(|a| a.get_info())
        .filter(|i| i.device_type == wgpu::DeviceType::Cpu)
        .map(|i| format!("{} via {:?}", i.name, i.backend))
        .collect();
    match adapters
        .into_iter()
        .find(|a| a.get_info().device_type != wgpu::DeviceType::Cpu)
    {
        Some(a) => Ok(a),
        None if software.is_empty() => bail!("no {} adapter", super::backend_name(kind)),
        // A software rasterizer (llvmpipe, lavapipe, WARP) is refused: the CPU backend is the software path.
        None => bail!(
            "only a software {} adapter ({})",
            super::backend_name(kind),
            software.join(", ")
        ),
    }
}

/// Describes the adapter a backend kind would use.
pub fn probe(kind: BackendKind) -> Result<String> {
    let a = request_adapter(kind)?;
    let i = a.get_info();
    Ok(format!("{} via {:?}", i.name, i.backend))
}

impl GpuBackend {
    /// Opens a GPU device for `kind` ([`BackendKind::Metal`], [`BackendKind::Vulkan`],
    /// [`BackendKind::Dx12`], [`BackendKind::Gl`], or [`BackendKind::Gpu`] for the best
    /// of them). Fails when no hardware adapter is found; a software adapter is refused.
    pub fn new(kind: BackendKind) -> Result<GpuBackend> {
        let adapter = request_adapter(kind)?;
        let info = adapter.get_info();
        // Take everything the adapter offers: large storage buffers matter for big scenes.
        let limits = adapter.limits();
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("lumenwright"),
            required_features: wgpu::Features::empty(),
            required_limits: limits,
            experimental_features: wgpu::ExperimentalFeatures::default(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
        }))
        .context("creating GPU device")?;
        Ok(GpuBackend {
            device,
            queue,
            info,
        })
    }

    fn storage_buffer(&self, label: &str, data: &[u8]) -> Result<wgpu::Buffer> {
        let max = self.device.limits().max_storage_buffer_binding_size as usize;
        if data.len() > max {
            bail!(
                "{label} buffer is {} MiB, above this GPU's {} MiB storage binding limit; reduce --render-distance",
                data.len() >> 20,
                max >> 20
            );
        }
        // Zero-length bindings are invalid.
        let padded;
        let contents = if data.is_empty() {
            padded = vec![0u8; 16];
            &padded[..]
        } else {
            data
        };
        Ok(self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            }))
    }

    /// Blocking copy of a GPU buffer back to host memory.
    fn read_buffer(&self, src: &wgpu::Buffer, size: u64) -> Result<Vec<u8>> {
        let staging = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.copy_buffer_to_buffer(src, 0, &staging, 0, size);
        self.queue.submit([enc.finish()]);
        self.map_and_read(&staging)
    }

    fn map_and_read(&self, staging: &wgpu::Buffer) -> Result<Vec<u8>> {
        let (tx, rx) = std::sync::mpsc::channel();
        staging.map_async(wgpu::MapMode::Read, .., move |r| {
            let _ = tx.send(r);
        });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| anyhow!("GPU poll failed: {e}"))?;
        rx.recv()?
            .map_err(|e| anyhow!("mapping readback buffer: {e}"))?;
        let data = staging
            .get_mapped_range(..)
            .map_err(|e| anyhow!("{e:?}"))?
            .to_vec();
        staging.unmap();
        Ok(data)
    }
}

impl Backend for GpuBackend {
    fn name(&self) -> String {
        format!("{} ({:?})", self.info.name, self.info.backend).to_lowercase()
    }

    fn render_quick(&mut self, frame: &QuickFrame) -> Result<Vec<u32>> {
        quick::render(self, frame)
    }

    fn render_hq(
        &mut self,
        frame: &HqFrame,
        progress: &(dyn Fn(u32, u32) + Sync),
    ) -> Result<Vec<[f32; 3]>> {
        pathtrace::render(self, frame, progress)
    }
}
