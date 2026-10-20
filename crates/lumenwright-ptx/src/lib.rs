//! CUDA kernel entry point. The path tracer itself is `lumenwright-kernel`, the
//! exact same code the CPU backend runs; this crate only maps CUDA threads to pixels.
//!
//! Build with `scripts/build-ptx.sh` (needs the `nvptx64-nvidia-cuda` target and
//! the `llvm-bitcode-linker` rustup component).

#![no_std]
#![feature(abi_ptx, stdarch_nvptx)]

use core::arch::nvptx::{
    _block_dim_x, _block_dim_y, _block_idx_x, _block_idx_y, _thread_idx_x, _thread_idx_y, trap,
};
use core::slice;

use lumenwright_kernel::{GpuLight, GpuModel, GpuQuad, Params, SceneView, render_sample};

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    unsafe { trap() }
}

/// Accumulates `samples` path samples per pixel into `accum` (rgb sum, sample count).
///
/// The path tracer is inlined whole into this kernel (`render_sample` and `trace` are
/// `inline(always)` on nvptx, and `scripts/build-ptx.sh` refuses a kernel that calls anything
/// but a panic). With `render_sample` and `trace` as PTX functions called from here, the
/// optimizing PTX compiler mistranslated the kernel on an RTX 4070 Ti SUPER (sm_89, driver
/// 616.92): the driver's JIT and ptxas 12.4 at -O2 and -O3 rendered about 2% of the pixels
/// wrong and ptxas -O1 never finished a launch, while ptxas -O0 rendered the same PTX exactly
/// as the CPU does. The PTX's float arithmetic is all IEEE (`.rn`, no approximations or
/// contractions), so its meaning does not depend on the optimization level.
///
/// # Safety
/// All pointers must reference device buffers of the given lengths, laid out as
/// described in `lumenwright_kernel::scene`.
#[unsafe(no_mangle)]
pub unsafe extern "ptx-kernel" fn pt_render(
    params: *const Params,
    grid: *const u32,
    grid_len: u32,
    models: *const GpuModel,
    models_len: u32,
    quads: *const GpuQuad,
    quads_len: u32,
    atlas: *const u32,
    atlas_len: u32,
    lights: *const GpuLight,
    lights_len: u32,
    emitters: *const u32,
    emitters_len: u32,
    accum: *mut [f32; 4],
    sample_start: u32,
    samples: u32,
) {
    unsafe {
        let x = (_block_idx_x() * _block_dim_x() + _thread_idx_x()) as u32;
        let y = (_block_idx_y() * _block_dim_y() + _thread_idx_y()) as u32;
        let p = &*params;
        if x >= p.frame[0] || y >= p.frame[1] {
            return;
        }
        let view = SceneView {
            params: p,
            grid: slice::from_raw_parts(grid, grid_len as usize),
            models: slice::from_raw_parts(models, models_len as usize),
            quads: slice::from_raw_parts(quads, quads_len as usize),
            atlas: slice::from_raw_parts(atlas, atlas_len as usize),
            lights: slice::from_raw_parts(lights, lights_len as usize),
            emitters: slice::from_raw_parts(emitters, emitters_len as usize),
        };
        let idx = (y * p.frame[0] + x) as usize;
        let mut acc = *accum.add(idx);
        for s in 0..samples {
            let c = one_sample(&view, x, y, sample_start + s);
            if c.x.is_finite() && c.y.is_finite() && c.z.is_finite() {
                acc[0] += c.x;
                acc[1] += c.y;
                acc[2] += c.z;
            }
            acc[3] += 1.0;
        }
        *accum.add(idx) = acc;
    }
}

/// One path sample, kept out of line. With `render_sample` inlined into the
/// sample loop of `pt_render`, a launch of two or more samples per pixel never
/// finished on an RTX 4070 Ti SUPER (driver 616.92, sm_89), while one sample per
/// launch took 22 ms at 1280x720; out of line, every batch size completes.
#[inline(never)]
fn one_sample(view: &SceneView, x: u32, y: u32, sample: u32) -> lumenwright_kernel::math::V3 {
    render_sample(view, x, y, sample)
}
