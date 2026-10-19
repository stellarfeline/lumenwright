//! Quick mode on the GPU: a real raster pipeline (vertex + fragment shaders).

use anyhow::Result;
use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use super::GpuBackend;
use crate::render::QuickFrame;
use crate::render::sky::QuickSky;
use crate::scene::mesh::Vertex;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    view_proj: [[f32; 4]; 4],
    sky: QuickSky,
    atlas_size: [u32; 4],
}

pub fn render(gpu: &GpuBackend, f: &QuickFrame) -> Result<Vec<u32>> {
    let t0 = std::time::Instant::now();
    let device = &gpu.device;
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("quick.wgsl"),
        source: wgpu::ShaderSource::Wgsl(include_str!("quick.wgsl").into()),
    });
    let uniforms = Uniforms {
        view_proj: f.view_proj.to_cols_array_2d(),
        sky: f.sky,
        atlas_size: [f.atlas_size[0], f.atlas_size[1], 0, 0],
    };
    let ubuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("quick uniforms"),
        contents: bytemuck::bytes_of(&uniforms),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let atlas = gpu.storage_buffer("atlas", bytemuck::cast_slice(f.atlas_pixels))?;

    let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("quick bgl"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("quick bg"),
        layout: &bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: ubuf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: atlas.as_entire_binding(),
            },
        ],
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("quick layout"),
        bind_group_layouts: &[Some(&bgl)],
        immediate_size: 0,
    });

    let format = wgpu::TextureFormat::Rgba8Unorm;
    let depth_format = wgpu::TextureFormat::Depth32Float;
    let vertex_layout = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<Vertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Unorm8x4],
    };
    let make_pipeline = |fs: &str,
                         blend: Option<wgpu::BlendState>,
                         depth_write: bool,
                         depth_compare: wgpu::CompareFunction| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(fs),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(vertex_layout.clone())],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: depth_format,
                depth_write_enabled: Some(depth_write),
                depth_compare: Some(depth_compare),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some(fs),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        })
    };
    let opaque = make_pipeline("fs_opaque", None, true, wgpu::CompareFunction::Less);
    let translucent = make_pipeline(
        "fs_translucent",
        Some(wgpu::BlendState::ALPHA_BLENDING),
        false,
        wgpu::CompareFunction::Less,
    );
    // See-through text displays: blended over everything, no depth test.
    let see_through = make_pipeline(
        "fs_translucent",
        Some(wgpu::BlendState::ALPHA_BLENDING),
        false,
        wgpu::CompareFunction::Always,
    );
    let sky = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("sky"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs_sky"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: Default::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: depth_format,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::Always),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs_sky"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    });

    let size = wgpu::Extent3d {
        width: f.width,
        height: f.height,
        depth_or_array_layers: 1,
    };
    let color = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("color"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let depth = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("depth"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: depth_format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let color_view = color.create_view(&Default::default());
    let depth_view = depth.create_view(&Default::default());

    let mesh = f.mesh;
    let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("vertices"),
        contents: if mesh.vertices.is_empty() {
            &[0u8; 24]
        } else {
            bytemuck::cast_slice(&mesh.vertices)
        },
        usage: wgpu::BufferUsages::VERTEX,
    });
    let mk_index = |idx: &[u32]| {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("indices"),
            contents: if idx.is_empty() {
                &[0u8; 12]
            } else {
                bytemuck::cast_slice(idx)
            },
            usage: wgpu::BufferUsages::INDEX,
        })
    };
    let ibuf_o = mk_index(&mesh.opaque);
    let ibuf_t = mk_index(&mesh.translucent);
    let ibuf_s = mk_index(&mesh.see_through);

    let mut enc = device.create_command_encoder(&Default::default());
    {
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("quick"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &color_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_bind_group(0, &bind_group, &[]);
        pass.set_pipeline(&sky);
        pass.draw(0..3, 0..1);
        pass.set_vertex_buffer(0, vbuf.slice(..));
        if !mesh.opaque.is_empty() {
            pass.set_pipeline(&opaque);
            pass.set_index_buffer(ibuf_o.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..mesh.opaque.len() as u32, 0, 0..1);
        }
        if !mesh.translucent.is_empty() {
            pass.set_pipeline(&translucent);
            pass.set_index_buffer(ibuf_t.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..mesh.translucent.len() as u32, 0, 0..1);
        }
        if !mesh.see_through.is_empty() {
            pass.set_pipeline(&see_through);
            pass.set_index_buffer(ibuf_s.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..mesh.see_through.len() as u32, 0, 0..1);
        }
    }
    let row = (f.width * 4).div_ceil(256) * 256;
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("color readback"),
        size: (row * f.height) as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &color,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &staging,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(f.height),
            },
        },
        size,
    );
    gpu.queue.submit([enc.finish()]);
    let bytes = gpu.map_and_read(&staging)?;
    let mut out = Vec::with_capacity((f.width * f.height) as usize);
    for y in 0..f.height {
        let r = &bytes[(y * row) as usize..(y * row + f.width * 4) as usize];
        out.extend(
            r.as_chunks::<4>()
                .0
                .iter()
                .map(|c| u32::from_le_bytes([c[0], c[1], c[2], 255])),
        );
    }
    log::info!(
        "gpu raster: {} triangles in {:.2?}",
        mesh.triangle_count(),
        t0.elapsed()
    );
    Ok(out)
}
