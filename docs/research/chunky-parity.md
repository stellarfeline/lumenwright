# HQ mode against Chunky: the component map and what was ported

HQ mode reaches Chunky's image quality by porting Chunky's rendering algorithms and constants,
not by inventing its own. Chunky is GPL-3.0-or-later, as Lumenwright is; ported code cites the
Chunky source file in a comment. The revision read is the one the benchmark's Chunky build
was made from: Chunky 2.5.0-SNAPSHOT.474 is `chunky-core-2.5.0-SNAPSHOT.474.g156e2bb.jar`, so
chunky-dev/chunky at `156e2bba2526` (2026-07-26). Paths below are under
`chunky/src/java/se/llbit/` at that revision.

Each statement is marked **[cited]** (the Chunky source or the paper it cites says it),
**[measured]** (a render or a test shows it) or **[inferred]**.

## Component map

| Chunky component (file) | Lumenwright counterpart (file) | How they differed | Port, and its state |
|---|---|---|---|
| Path tracer: `chunky/renderer/scene/PathTracer.java` (`pathTrace`, `doDiffuseReflection`) | `lumenwright-kernel/src/integrator.rs` (`render_sample`), mirrored in `lumenwright/src/render/gpu/pathtrace.wgsl` | Lumenwright weighted emitter light by MIS against BSDF hits and added emitted light at the surface after the last bounce. | **Ported**: Chunky's diffuse event (an emitting surface adds `colour² × emittance × intensity` and is not lit by emitter sampling; any other surface is; then sun sampling; then a cosine-weighted bounce) and its ray depth (the surface after the last bounce adds nothing). **Kept**: Russian roulette after the third bounce, which Chunky does not do (unbiased, so it changes noise, not the image); specular events do not count towards the depth, where Chunky's `ray.depth` counts them. |
| First-ray reuse: `branchCount` in `PathTracer.pathTrace` | none | Chunky reuses each camera ray's first hit for `branchCount` (10) paths. | **Not ported**: it changes speed and noise, not the converged image [inferred from the code: the paths average the same estimator]. |
| Sun: `chunky/renderer/scene/sky/Sun.java` (`initSun`, `getRandomSunDirection`), `SunSamplingStrategy.java` (`FAST`) | `lumenwright/src/render/sky.rs` (`hq_sky`), kernel `sample_sun` and sun next-event estimation | Lumenwright's sun was tinted warm by altitude and its irradiance was its intensity (default 1.5). | **Ported**: white sun, irradiance `intensity^2.2` (`Scene.DEFAULT_GAMMA`), default intensity 1.25; uniform sampling of the sun's cone (radius 0.03) as `getRandomSunDirection`; `FAST` strategy (sun sampled at every diffuse surface, a diffuse path that reaches the sky does not see the sun). **Lumenwright's own**: the sun fades to nothing below a sine of altitude of 1/6, so an open world is not lit from below at night. **Not ported**: the sun drawn in the sky (Chunky draws the client's sun texture at 10 × its apparent brightness; Lumenwright draws a disk of the sun's radiance); `IMPORTANCE` and `HIGH_QUALITY` strategies. |
| Sky: `chunky/renderer/scene/sky/PreethamSky.java`, `SkyCache.java`, `Sky.java` (`SIMULATED` mode) | kernel `sky_radiance`; `sky.rs` (`Preetham`) | Lumenwright's sky was its own zenith/horizon gradient with a glow around the sun. | **Ported**: the Preetham sky (Preetham, Shirley and Smits, "A Practical Analytic Model for Daylight", SIGGRAPH 1999, as Chunky implements it at turbidity 2.5) [cited], evaluated per direction in the kernel; below the horizon it holds the horizon value as Chunky's does. Chunky reads it through a 128 × 128 cache interpolated in HSL; the kernel evaluates it directly. **Lumenwright's own**: below a sine of 0.15 the sky dims (to 3% at −0.1), so a night is dark; Chunky's Preetham sky keeps the sunset sky. **Not ported**: the Nishita sky, gradients, sky maps, clouds. |
| Fog: `chunky/renderer/scene/Fog.java`, `FogLayer.java` | none | — | **Not ported**: the benchmark scene's fog has `uniformDensity` 0, under which `addGroundFog` scales by `exp(0) = 1` and adds `(1 − 1) × …` [cited], so Chunky's benchmark frame has no fog. The distance haze seen in Chunky's frame is its sky light on far faces: after the sky and emitter ports the far hills match it (see the measurements). |
| Emitters: `PathTracer.sampleEmitter`, `sampleEmitterFace`; `math/Grid.java`; `chunky/renderer/EmitterSamplingStrategy.java` | kernel `sample_emitters`, `sample_emitter`, `sample_emitter_face`, `emitter_cell`; `lumenwright/src/scene/voxel.rs` (`emitter_list`) | Lumenwright gave each block a power of 600 × (level/15)² shared by its glowing quads, picked lights by power over the whole scene (2169 light quads in the benchmark), sampled texel cells, and combined with BSDF hits by MIS. In the before frame the lantern light floods the camp orange and the stone hills, tens of blocks from any lantern, take an orange cast [measured: the before frame and its crops]. The benchmark holds 148 emitters, 146 lanterns and 2 campfires [measured: the build log]. | **Ported**: the emitter grid (cells of 10 blocks, each listing the emitters in it and its 26 neighbours) and the four strategies (`NONE`, `ONE`, `ONE_BLOCK`, `ALL`; `ONE` is Lumenwright's default, Chunky's default is `NONE`); a uniform point on one face; the face counts when the first surface met towards it is at its distance; `|cos| / max(d², 1) × area × emittance × intensity × colour met`, times π; one emitter of the cell picked uniformly with no division by its probability (as Chunky's); `preventNormalEmitterWithSampling`. Cells start at the grid origin, where Chunky's start at its octree origin, the same point when the scene's lowest height is the world's. |
| Emitter materials: `chunky/chunk/BlockPalette.java` (`getDefaultMaterialProperties`), `Scene.DEFAULT_EMITTER_INTENSITY` | `lumenwright/src/assets/blockinfo.rs` (`emittance`); `request.rs` (`DEFAULT_EMITTER_INTENSITY`) | Lumenwright tied emitted light to the block's light level and drew a separate look (`--emitter-glow`). | **Ported**: one emittance per block material, on every face, entry for entry (keys as Chunky has them, so `magma_block` emits nothing as in Chunky, whose key is `minecraft:magma`); emitter intensity 13. **Not ported**: candle flames and the campfire's fire quads, which Chunky draws as particle entities that are its emitters (Lumenwright draws the client's block models; a campfire's whole model is its emitter); the `light` block, which has no faces here. |
| Tone mapping: `chunky/renderer/postprocessing/GammaCorrectionFilter.java`, `ACESFilmicFilter.java`, `HableToneMappingFilter.java`, `SimplePixelPostProcessingFilter.java` | `lumenwright/src/render/post.rs` | Lumenwright's default was ACES with the sRGB curve. | **Ported**: the gamma filter (`v^(1/2.2)`) as the default; ACES and Hable ending in `v^(1/2.2)`; Chunky's rounding (`ColorUtil.getRGB`). **Not ported**: `TONEMAP1`, the UE4 filter. |
| Texture decoding: `math/ColorUtil.java` (`toLinearLut`) | kernel `math::gamma_to_linear` | Lumenwright decoded with a polynomial fit of the sRGB curve. | **Ported**: `c^2.2`, computed in `f32` alone (`exp(2.2 ln c)`) so CPU and CUDA agree; within 4.2e-7 relative of `f64` at every byte value [measured]. |
| One-sided block faces: `math/Quad.java` (`intersect`), `math/AABB.java`, `chunky/model/AABBModel.java` | `lumenwright-kernel/src/trace.rs` (`test_quad`), `voxel.rs` (`FLAG_FRONT_ONLY` on block-model quads) | Lumenwright met every quad from both sides, so a ray through a hole in a leaf face met the far face of the same block from behind: two layers of leaf texture per block, which read as a flat solid block [inferred from the code; the leaves' measured difference fell after the port]. | **Ported**: block-model faces are met from the front only; water surfaces stay two-sided (the kernel refracts out of water through them). |
| Octree traversal bounds: `math/Octree.java` (`enterBlock`) | `trace.rs` (`trace`) | Lumenwright summed each axis's step to find the next cell boundary; the drift hid behind two-sided faces until faces became one-sided, then distant trees read as see-through. | **Ported**: each boundary is computed from the cell, as Chunky computes each node's bounds. |
| Overlapping model faces: `chunky/model/minecraft/GrassBlockModel.java` (tinted overlay box first) | `trace.rs` (`test_quad`, `COPLANAR`) | Lumenwright kept the first of two coplanar quads, so a grass block's untinted side, with its pre-coloured lime fringe, hid the tinted overlay: the harsh banding on the hills. | **Ported**: a later coplanar quad (within 1e-5 blocks) is met over an earlier one, as the client draws later elements over earlier ones and as Chunky draws its overlay. |
| Biome colours: `chunky/world/biome/Biomes.java` (`unknown`, `loadGrassColors`, `loadFoliageColors`) | `lumenwright/src/assets/biome.rs` (`biome_info`) | Lumenwright gave an unknown biome (a data pack's) the plains climate; Chunky gives it temperature 0.5, downfall 0.5 and takes its colours from the colormaps. The benchmark world has two biomes, `windswept_forest` and the data pack's `the-treehouse-camp:void` [measured: the region files]; the camp's leaves are in the second [inferred: their colour difference went away with this port]. | **Ported**: an unknown biome gets Chunky's unknown climate. **Not ported**: biome blending (`biomeBlendingRadius`, 2 in the benchmark): it would make every block's tint depend on its neighbours, and HQ mode holds at most 65535 distinct block instances. |
| Translucency: `PathTracer.translucentRayColor` with `fancierTranslucency` | kernel stochastic pass-through | Chunky's "fancier translucency" splits absorption by colour; Lumenwright blends by alpha. | **Not ported**; not examined in the benchmark frame. |
| Water: `chunky/renderer/scene/*WaterShader.java`, `waterOpacity`, `waterVisibility`, water `specular` 0.255 in `BlockPalette` | kernel Fresnel dielectric and absorption | Chunky shades water with a tiled normal map and its own opacity and visibility. | **Not ported**; not examined in the benchmark frame. |
| Metals and specular materials: `BlockPalette` (`specular`, `metalness`, `setPerceptualSmoothness`), `PathTracer.doSpecularReflection` | none (every block surface is diffuse) | Gold, iron and copper blocks reflect in Chunky. | **Not ported**; not examined in the benchmark frame. |
| Camera: `chunky/renderer/projection/PinholeProjector.java`, `Camera.clampedFovTan` | `request.rs` (`CameraRays`), kernel camera | — | **Same**: vertical field of view, `2 tan(fov/2)` over the image height [cited]. |

## Measurements

The benchmark frame: the whole-camp view of *The Treehouse Camp* at 1280 × 720, camera at
140, 118, 14 looking at 56, 82, 58, vertical field of view 60°, sun at altitude 45° from the
south-east, 625 chunks, four bounces (Chunky `rayDepth` 5). Chunky's scene: Preetham sky,
sun intensity 1.25, emitters at intensity 13 with sampling `ONE`, gamma tone map, exposure 1.
Lumenwright at each revision with its defaults, which after the port are the same settings.

The reference is Chunky's frame at 4000 samples per pixel (35.5 minutes on the M2 Pro's
CPU). Each Lumenwright frame is 250 spp on Metal (M2 Pro) unless stated. Display: absolute
difference of 8-bit values, mean over channels and percentiles of the per-pixel largest
channel difference. Linear: the same after decoding both images with `c^2.2`. SSIM: Wang et
al. 2004 on display luma, Gaussian window σ 1.5. The script is a single file (numpy only).

| frame | display mean | p50 | p90 | p99 | linear mean | linear p90 | SSIM |
|---|---|---|---|---|---|---|---|
| Chunky 250 spp (run 1) | 1.99 | 2 | 7 | 15 | 0.0046 | 0.0194 | 0.9575 |
| Chunky 250 spp (run 2) | 1.99 | 2 | 7 | 15 | 0.0046 | 0.0194 | 0.9574 |
| Lumenwright before the port (`ad390dc`) | 24.15 | 29 | 63 | 156 | 0.0901 | 0.2501 | 0.7511 |
| + display gamma 2.2 (`d5ff056`) | 14.41 | 21 | 49 | 143 | 0.0503 | 0.2625 | 0.8060 |
| + Chunky's sun (`fc38dfe`) | 14.75 | 21 | 50 | 143 | 0.0517 | 0.2625 | 0.8087 |
| + Preetham sky (`c7247e2`) | 9.73 | 8 | 48 | 142 | 0.0246 | 0.1159 | 0.8108 |
| + Chunky's emitters (`0db7c0f`) | 2.80 | 2 | 12 | 29 | 0.0059 | 0.0221 | 0.9323 |
| + one-sided block faces (`45bf3b1`) | 2.58 | 2 | 10 | 29 | 0.0055 | 0.0211 | 0.9343 |
| + Chunky's unknown biome (`ad97ef9`) | 2.37 | 2 | 8 | 29 | 0.0053 | 0.0205 | 0.9344 |
| + traversal bounds (`731fab7`) | 1.92 | 2 | 6 | 24 | 0.0042 | 0.0160 | 0.9401 |
| + coplanar quads (`a3d7a74`) | 1.70 | 2 | 6 | 15 | 0.0036 | 0.0146 | 0.9593 |
| `a3d7a74` on CUDA at 4000 spp (converged against converged) | 1.25 | 1 | 4 | 14 | 0.0027 | 0.0109 | 0.9716 |

* [measured] After the port Lumenwright's 250 spp frame is as close to Chunky's converged
  frame as Chunky's own 250 spp frames are (display mean 1.70 against 1.99, SSIM 0.9593
  against 0.9575).
* [measured] The sky band at the top of the frame: Chunky 139.3 173.2 201.0 (mean RGB),
  Lumenwright 152.4 180.9 228.0 before the sky port and 139.3 173.3 201.0 after.
* [measured] The CUDA and Metal frames of `a3d7a74` agree to a display mean of 0.00/255.
* [measured] Where the converged frames still differ most (40-pixel blocks): the border
  between the camp's biome and the hills' (Chunky blends biome colours over 5 × 5 blocks),
  a faint +1 to +3 in blue on the camp's grass, and texture-level noise.

The images: the side-by-side and the crops are kept beside the benchmark, not in the
repository.

## Speed

The same frame, 250 spp, four bounces, after the port (`a3d7a74`; the code is unchanged
since). Chunky and Lumenwright on Metal ran alternately on the M2 Pro (10 CPU cores, 16 GPU
cores, JDK 17 for Chunky); Lumenwright on CUDA ran on the RTX 4070 Ti SUPER (WSL2). Two timers
each: the renderer's own (Chunky's `renderTime`; Lumenwright's `render` stage) and the whole
command (Chunky's includes starting the JVM and building its octree; Lumenwright's includes
reading the world and the jar). The CUDA command's wall time is read from the remote
machine's monotonic clock.

| renderer | runs, own timer (s) | median | runs, whole command (s) | median |
|---|---|---|---|---|
| Chunky 2.5.0-SNAPSHOT.474, CPU | 126.70, 124.73, 121.77 | 124.73 | 131.79, 129.61, 125.89 | 129.61 |
| Lumenwright, Metal | 32.96, 33.01, 32.62 | 32.96 | 33.26, 33.33, 33.00 | 33.26 |
| Lumenwright, CUDA | 6.03, 6.15, 6.18 | 6.15 | 6.62, 6.72, 6.75 | 6.72 |

* [measured] Lumenwright on Metal is 3.8 times as fast as Chunky on the same machine by the
  renderers' own timers (124.73 / 32.96) and 3.9 times by the whole command (129.61 /
  33.26). An earlier measurement of the same frame before the port, by the same method, had
  Metal at 48.36 s and Chunky at 115.4 s.
* [measured] A third Chunky run (136.02 s own, 140.95 s whole; the Metal run after it 32.55
  s, 32.87 s) overlapped a short build of this branch and is left out of the medians; a
  fourth pair replaced it. With it the Chunky median is 125.7 s and the ratio 3.8.
* [measured] One-minute load average on the M2 Pro before each counted Chunky run: 1.92,
  23.18 (the first Chunky run's threads, decaying), 4.04; no build or render of this work ran on
  it during the counted pairs. The remote machine was idle (load 0.37, GPU 0%) around the
  CUDA runs.
* The frames are the measured ones: each Chunky run against the converged reference is
  display mean 1.99/255, and each Metal run is the same bytes as the measured `a3d7a74`
  frame.

## Not yet at parity

* Biome blending: Chunky averages biome colours over a 5 × 5 area; Lumenwright takes each
  block's own biome. Visible only at biome borders.
* The sun drawn in the sky, candle and campfire flame particles, fancier translucency,
  Chunky's water shading and its metals: not ported, and not examined in the benchmark frame.
* A night: Chunky's Preetham sky and sun do not darken below the horizon; Lumenwright's fade
  them, so night frames are Lumenwright's own.
