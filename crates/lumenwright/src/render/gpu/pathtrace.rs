//! HQ mode on the GPU: the WGSL port of the path-tracing kernel as a compute shader.

use std::time::{Duration, Instant};

use anyhow::Result;
use wgpu::util::DeviceExt;

use super::GpuBackend;
use crate::render::HqFrame;

/// Target wall time per submission; long dispatches can trip OS GPU watchdogs.
const TARGET_DISPATCH: Duration = Duration::from_millis(150);

pub fn render(
    gpu: &GpuBackend,
    f: &HqFrame,
    progress: &(dyn Fn(u32, u32) + Sync),
) -> Result<Vec<[f32; 3]>> {
    let device = &gpu.device;
    let s = f.scene;
    let (w, h) = (f.params.frame[0], f.params.frame[1]);
    let spp = f.spp.max(1);

    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("pathtrace.wgsl"),
        source: wgpu::ShaderSource::Wgsl(include_str!("pathtrace.wgsl").into()),
    });
    let ubuf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("params"),
        size: std::mem::size_of::<lumenwright_kernel::Params>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let grid = gpu.storage_buffer("grid", bytemuck::cast_slice(&s.grid))?;
    let models = gpu.storage_buffer("models", bytemuck::cast_slice(&s.models))?;
    let quads = gpu.storage_buffer("quads", bytemuck::cast_slice(&s.quads))?;
    let atlas = gpu.storage_buffer("atlas", bytemuck::cast_slice(&s.atlas))?;
    let emitters = gpu.storage_buffer("emitters", bytemuck::cast_slice(&s.emitters))?;
    let emitter_grid = gpu.storage_buffer("emitter grid", bytemuck::cast_slice(&s.emitter_grid))?;
    let sky = gpu.storage_buffer("sky cache", bytemuck::cast_slice(&f.sky))?;
    let accum_size = (w as u64) * (h as u64) * 16;
    let accum = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("accum"),
        contents: &vec![0u8; accum_size as usize],
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
    });

    let storage = |binding: u32, read_only: bool| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("pt bgl"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            storage(1, true),
            storage(2, true),
            storage(3, true),
            storage(4, true),
            storage(5, false),
            storage(6, true),
            storage(7, true),
            storage(8, true),
        ],
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("pt bg"),
        layout: &bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: ubuf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: grid.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: models.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: quads.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: atlas.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: accum.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: emitters.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 7,
                resource: emitter_grid.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 8,
                resource: sky.as_entire_binding(),
            },
        ],
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("pt layout"),
        bind_group_layouts: &[Some(&bgl)],
        immediate_size: 0,
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("pathtrace"),
        layout: Some(&layout),
        module: &module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });

    // Progressive accumulation; the batch size adapts to keep each submission short.
    let mut done = 0u32;
    let mut batch = 1u32;
    let t0 = Instant::now();
    while done < spp {
        let n = batch.min(spp - done);
        let mut p = f.params;
        p.frame[2] = done;
        p.misc_u[1] = n;
        gpu.queue.write_buffer(&ubuf, 0, bytemuck::bytes_of(&p));
        let mut enc = device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("pt"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        }
        let start = Instant::now();
        gpu.queue.submit([enc.finish()]);
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| anyhow::anyhow!("GPU poll failed: {e}"))?;
        let took = start.elapsed();
        done += n;
        progress(done, spp);
        if took < TARGET_DISPATCH / 2 {
            batch = (batch * 2).min(64);
        } else if took > TARGET_DISPATCH * 2 && batch > 1 {
            batch /= 2;
        }
    }
    log::info!(
        "gpu path tracing: {spp} spp at {w}x{h} in {:.2?}",
        t0.elapsed()
    );

    let bytes = gpu.read_buffer(&accum, accum_size)?;
    let acc: &[[f32; 4]] = bytemuck::cast_slice(&bytes);
    Ok(acc
        .iter()
        .map(|a| {
            let n = a[3].max(1.0);
            [a[0] / n, a[1] / n, a[2] / n]
        })
        .collect())
}
