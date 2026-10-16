//! CPU backend: software rasterizer + the shared Rust path-tracing kernel on rayon.

pub mod raster;

use std::sync::atomic::{AtomicU32, Ordering};

use anyhow::Result;
use lumenwright_kernel::{SceneView, render_sample};
use rayon::prelude::*;

use super::{Backend, HqFrame, QuickFrame};

/// The CPU backend: the software rasterizer for quick mode and the shared
/// Rust path-tracing kernel on rayon for HQ mode. It is always available.
pub struct CpuBackend;

impl CpuBackend {
    /// Creates the CPU backend. It uses rayon's current thread pool.
    pub fn new() -> CpuBackend {
        CpuBackend
    }
}

impl Default for CpuBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl Backend for CpuBackend {
    fn name(&self) -> String {
        format!("cpu ({} threads)", rayon::current_num_threads())
    }

    fn render_quick(&mut self, frame: &QuickFrame) -> Result<Vec<u32>> {
        Ok(raster::rasterize(frame))
    }

    fn render_hq(
        &mut self,
        frame: &HqFrame,
        progress: &(dyn Fn(u32, u32) + Sync),
    ) -> Result<Vec<[f32; 3]>> {
        let s = frame.scene;
        let mut params = frame.params;
        let (w, h) = (params.frame[0], params.frame[1]);
        let spp = frame.spp.max(1);
        let rows_done = AtomicU32::new(0);
        let mut out = vec![[0f32; 3]; (w * h) as usize];
        params.frame[2] = 0;
        let view = SceneView {
            params: &params,
            grid: &s.grid,
            models: &s.models,
            quads: &s.quads,
            atlas: &s.atlas,
        };
        out.par_chunks_mut(w as usize)
            .enumerate()
            .for_each(|(y, row)| {
                for (x, px) in row.iter_mut().enumerate() {
                    let mut acc = [0f64; 3];
                    for sample in 0..spp {
                        let c = render_sample(&view, x as u32, y as u32, sample);
                        // Guard against fireflies from NaN/inf.
                        if c.x.is_finite() && c.y.is_finite() && c.z.is_finite() {
                            acc[0] += c.x as f64;
                            acc[1] += c.y as f64;
                            acc[2] += c.z as f64;
                        }
                    }
                    *px = [
                        (acc[0] / spp as f64) as f32,
                        (acc[1] / spp as f64) as f32,
                        (acc[2] / spp as f64) as f32,
                    ];
                }
                let done = rows_done.fetch_add(1, Ordering::Relaxed) + 1;
                if done.is_multiple_of(16) || done == h {
                    progress(done, h);
                }
            });
        Ok(out)
    }
}
