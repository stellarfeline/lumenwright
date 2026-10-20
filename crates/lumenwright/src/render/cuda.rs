//! CUDA backend: runs `lumenwright-kernel` compiled to PTX (see `crates/lumenwright-ptx`).
//!
//! The kernel is the same Rust source the CPU backend executes, so CPU and CUDA
//! results agree sample for sample. The driver JIT-compiles the embedded PTX for
//! the installed GPU; `libcuda` is loaded dynamically, so this backend builds on
//! machines without the CUDA toolkit.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use cudarc::driver::{CudaContext, LaunchConfig, PushKernelArg, sys};
use cudarc::nvrtc::Ptx;

use super::{Backend, HqFrame, QuickFrame};

const PTX: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../target/ptx/lumenwright.ptx"
));
const TARGET_LAUNCH: Duration = Duration::from_millis(150);

/// The CUDA backend: HQ mode runs the Rust kernel compiled to PTX through the
/// CUDA driver API. Quick mode falls back to the CPU rasterizer. `libcuda` is
/// loaded at run time.
pub struct CudaBackend {
    ctx: Arc<CudaContext>,
    name: String,
}

fn open() -> Result<Arc<CudaContext>> {
    // CudaContext::new panics rather than erroring when libcuda is missing;
    // catch it quietly and report a normal error instead.
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let r = std::panic::catch_unwind(|| CudaContext::new(0));
    std::panic::set_hook(prev_hook);
    match r {
        Ok(Ok(ctx)) => Ok(ctx),
        Ok(Err(e)) => Err(anyhow!("CUDA driver error: {e:?}")),
        Err(_) => Err(anyhow!("CUDA driver library (libcuda) not found")),
    }
}

/// Describes device 0 and its compute capability, or says why CUDA is unavailable.
pub fn probe() -> Result<String> {
    let ctx = open()?;
    let (major, minor) = ctx.compute_capability().map_err(|e| anyhow!("{e:?}"))?;
    Ok(format!(
        "{} (sm_{major}{minor})",
        ctx.name().map_err(|e| anyhow!("{e:?}"))?
    ))
}

impl CudaBackend {
    /// Opens CUDA device 0. Fails when the driver library is missing or reports an error.
    pub fn new() -> Result<CudaBackend> {
        let ctx = open()?;
        // The Rust kernel keeps its traversal state in local memory.
        ctx.set_limit(sys::CUlimit::CU_LIMIT_STACK_SIZE, 16 * 1024)
            .map_err(|e| anyhow!("{e:?}"))?;
        let name = ctx.name().map_err(|e| anyhow!("{e:?}"))?;
        Ok(CudaBackend { ctx, name })
    }
}

impl Backend for CudaBackend {
    fn name(&self) -> String {
        format!("{} (cuda)", self.name).to_lowercase()
    }

    fn render_quick(&mut self, frame: &QuickFrame) -> Result<Vec<u32>> {
        log::warn!(
            "quick mode has no CUDA rasterizer; using the CPU rasterizer (use --backend vulkan for GPU raster)"
        );
        Ok(super::cpu::raster::rasterize(frame))
    }

    fn render_hq(
        &mut self,
        f: &HqFrame,
        progress: &(dyn Fn(u32, u32) + Sync),
    ) -> Result<Vec<[f32; 3]>> {
        let e = |e: cudarc::driver::DriverError| anyhow!("CUDA: {e:?}");
        let s = f.scene;
        let (w, h) = (f.params.frame[0], f.params.frame[1]);
        let spp = f.spp.max(1);
        let stream = self.ctx.default_stream();
        let jit = Instant::now();
        let module = self
            .ctx
            .load_module(Ptx::from_src(PTX))
            .map_err(e)
            .context("loading PTX")?;
        let func = module.load_function("pt_render").map_err(e)?;
        let jit = jit.elapsed();

        let params_words: &[u32] = bytemuck::cast_slice(std::slice::from_ref(&f.params));
        let d_params = stream.clone_htod(params_words).map_err(e)?;
        let d_grid = stream.clone_htod(&s.grid).map_err(e)?;
        let d_models = stream
            .clone_htod(bytemuck::cast_slice::<_, u32>(&s.models))
            .map_err(e)?;
        let d_quads = stream
            .clone_htod(bytemuck::cast_slice::<_, u32>(&s.quads))
            .map_err(e)?;
        let d_atlas = stream.clone_htod(&s.atlas).map_err(e)?;
        let d_emitters = stream
            .clone_htod(bytemuck::cast_slice::<_, u32>(&s.emitters))
            .map_err(e)?;
        let d_emitter_grid = stream.clone_htod(&s.emitter_grid).map_err(e)?;
        let emitter_grid_len = s.emitter_grid.len() as u32;
        let mut d_accum = stream.alloc_zeros::<f32>((w * h * 4) as usize).map_err(e)?;
        let (grid_len, models_len, quads_len, atlas_len, emitters_len) = (
            s.grid.len() as u32,
            s.models.len() as u32,
            s.quads.len() as u32,
            s.atlas.len() as u32,
            s.emitters.len() as u32,
        );

        let cfg = LaunchConfig {
            grid_dim: (w.div_ceil(16), h.div_ceil(16), 1),
            block_dim: (16, 16, 1),
            shared_mem_bytes: 0,
        };
        let (mut done, mut batch) = (0u32, 1u32);
        // The kernel's own clock: a timing event on the stream either side of every launch.
        let (mut kernel_ms, mut launches) = (0f64, 0u32);
        let timed = Some(sys::CUevent_flags::CU_EVENT_DEFAULT);
        let t0 = Instant::now();
        while done < spp {
            let n = batch.min(spp - done);
            let start = Instant::now();
            let mut b = stream.launch_builder(&func);
            b.arg(&d_params);
            b.arg(&d_grid);
            b.arg(&grid_len);
            b.arg(&d_models);
            b.arg(&models_len);
            b.arg(&d_quads);
            b.arg(&quads_len);
            b.arg(&d_atlas);
            b.arg(&atlas_len);
            b.arg(&d_emitters);
            b.arg(&emitters_len);
            b.arg(&d_emitter_grid);
            b.arg(&emitter_grid_len);
            b.arg(&mut d_accum);
            b.arg(&done);
            b.arg(&n);
            let before = stream.record_event(timed).map_err(e)?;
            // SAFETY: argument order and types match `pt_render` in crates/lumenwright-ptx.
            unsafe { b.launch(cfg) }.map_err(e)?;
            let after = stream.record_event(timed).map_err(e)?;
            stream.synchronize().map_err(e)?;
            kernel_ms += before.elapsed_ms(&after).map_err(e)? as f64;
            launches += 1;
            let took = start.elapsed();
            done += n;
            progress(done, spp);
            if took < TARGET_LAUNCH / 2 {
                batch = (batch * 2).min(64);
            } else if took > TARGET_LAUNCH * 2 && batch > 1 {
                batch /= 2;
            }
        }
        log::info!(
            "cuda path tracing: {spp} spp at {w}x{h} in {:.2?} ({kernel_ms:.1} ms in {launches} launches by CUDA events; PTX load {jit:.2?})",
            t0.elapsed()
        );
        let acc: Vec<f32> = stream.clone_dtoh(&d_accum).map_err(e)?;
        Ok(acc
            .as_chunks::<4>()
            .0
            .iter()
            .map(|a| {
                let n = a[3].max(1.0);
                [a[0] / n, a[1] / n, a[2] / n]
            })
            .collect())
    }
}
