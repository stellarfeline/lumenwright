//! GPU backend over wgpu: native Metal (macOS), Vulkan (Linux/Windows/NVIDIA/AMD)
//! or DX12 (Windows). WGSL shaders are translated by naga to MSL / SPIR-V / HLSL.

mod pathtrace;
mod quick;

use anyhow::{Context, Result, anyhow, bail};
use wgpu::util::DeviceExt;

use super::{Backend, BackendKind, HqFrame, QuickFrame};

/// The wgpu backend: Metal, Vulkan or DX12, chosen by the [`BackendKind`] it
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
        BackendKind::Gpu | BackendKind::Auto => wgpu::Backends::PRIMARY,
        other => bail!("{other:?} is not a wgpu backend"),
    })
}

fn request_adapter(kind: BackendKind) -> Result<wgpu::Adapter> {
    let backends = backends_for(kind)?;
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
    desc.backends = backends;
    let instance = wgpu::Instance::new(desc);
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        force_fallback_adapter: false,
        compatible_surface: None,
        apply_limit_buckets: false,
    }))
    .map_err(|e| anyhow!("no {kind:?} adapter: {e}"))?;
    if adapter.get_info().device_type == wgpu::DeviceType::Cpu {
        bail!("only a software {kind:?} adapter is available");
    }
    Ok(adapter)
}

/// Describes the adapter a backend kind would use.
pub fn probe(kind: BackendKind) -> Result<String> {
    let a = request_adapter(kind)?;
    let i = a.get_info();
    Ok(format!("{} via {:?}", i.name, i.backend))
}

impl GpuBackend {
    /// Opens a GPU device for `kind` ([`BackendKind::Metal`], [`BackendKind::Vulkan`],
    /// [`BackendKind::Dx12`], or [`BackendKind::Gpu`] / [`BackendKind::Auto`] for any
    /// primary API). Fails when no hardware adapter is found; a software adapter
    /// is refused.
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
