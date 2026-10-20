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
| Sun: `chunky/renderer/scene/sky/Sun.java` (`initSun`, `getRandomSunDirection`, `intersect`), `SunSamplingStrategy.java` (`FAST`), `Sky.addSunColor` | `lumenwright/src/render/sky.rs` (`hq_sky`, `sun_texture_axes`), kernel `sample_sun`, `sun_texture` and sun next-event estimation | Lumenwright's sun was tinted warm by altitude and its irradiance was its intensity (default 1.5); later it faded to nothing below a sine of altitude of 1/6; it was drawn as a disc of its radiance. | **Ported**: white sun, irradiance `intensity^2.2` (`Scene.DEFAULT_GAMMA`) at every altitude, default intensity 1.25; uniform sampling of the sun's cone (radius 0.03) as `getRandomSunDirection`, each sampled direction tested against the surface (`frontLight` in `PathTracer.doDiffuseReflection`); `FAST` strategy (sun sampled at every diffuse surface, a diffuse path that reaches the sky does not see the sun); the sun drawn in the sky on direct and specular sky hits as Chunky draws it: the client's sun texture over a square of half-width four times the sun's radius along `initSun`'s axes `su` and `sv`, nearest texel, decoded with `c^2.2`, times 10 × the apparent brightness^2.2 (16.338 at the default apparent brightness, 1.25), added to the sky. **Lumenwright's own**: a sampled sun direction below the horizon gives no light (the ground beyond the world hides it), so a floating world is not lit from below at night; on the dusk frame this changes nothing measurable against Chunky (see Dusk). **Not ported**: `IMPORTANCE` and `HIGH_QUALITY` strategies; the apparent brightness as its own setting (Lumenwright holds Chunky's default). |
| Sky: `chunky/renderer/scene/sky/PreethamSky.java`, `SkyCache.java`, `Sky.java` (`SIMULATED` mode), `math/ColorUtil.java` (`RGBtoHSL`, `RGBfromHSL`) | kernel `preetham_radiance`, `build_sky_cache`, `sky_radiance`; `sky.rs` (`Preetham`, `sky_cache`) | Lumenwright's sky was its own zenith/horizon gradient with a glow around the sun; later the Preetham sky evaluated per direction. | **Ported**: the Preetham sky (Preetham, Shirley and Smits, "A Practical Analytic Model for Daylight", SIGGRAPH 1999, as Chunky implements it at turbidity 2.5) [cited]; below the horizon it holds the horizon value as Chunky's does. Read as Chunky reads it, through its sky cache: a 129 × 129 table of the sky in HSL at resolution 128 (`SkyCache.precalculateSky`), built on the host from the kernel's Preetham function, interpolated bilinearly in HSL and turned back into RGB in the kernel (`calcIncidentLight`, `getColorInterpolated`). `RGBtoHSL` limits each channel to 1 first, so the sky is never brighter than 1 near the sun [cited]; before this port Lumenwright's silhouettes against a setting sun were lighter than Chunky's [measured: see Dusk]. **Lumenwright's own**, because Chunky has no time of day and its Preetham sky keeps the sunset sky at any altitude below the horizon (`PreethamSky.updateSun` clamps the altitude to `[0, π]`): the sky is Chunky's at full light while any part of the sun's disc is above the horizon, then falls linearly to 3% of its light (the night's) at the end of civil twilight, the sun's centre 6° below the horizon, and holds it (`sky.rs`, `twilight`). Chunky's documentation names its Nishita sky as the model that "can also simulate twilight" (`docs/reference/user_interface/chunky/render_controls/sky_and_fog.md` in chunky-dev/docs at `2e94084`); it is not ported (see Not yet at parity). **Not ported**: the Nishita sky, gradients, sky maps, clouds. |
| Fog: `chunky/renderer/scene/Fog.java`, `FogLayer.java` | none | — | **Not ported**: the benchmark scene's fog has `uniformDensity` 0, under which `addGroundFog` scales by `exp(0) = 1` and adds `(1 − 1) × …` [cited], so Chunky's benchmark frame has no fog. The distance haze seen in Chunky's frame is its sky light on far faces: after the sky and emitter ports the far hills match it (see the measurements). |
| Emitters: `PathTracer.sampleEmitter`, `sampleEmitterFace`; `math/Grid.java`; `chunky/renderer/EmitterSamplingStrategy.java` | kernel `sample_emitters`, `sample_emitter`, `sample_emitter_face`, `emitter_cell`; `lumenwright/src/scene/voxel.rs` (`emitter_list`) | Lumenwright gave each block a power of 600 × (level/15)² shared by its glowing quads, picked lights by power over the whole scene (2169 light quads in the benchmark), sampled texel cells, and combined with BSDF hits by MIS. In the before frame the lantern light floods the camp orange and the stone hills, tens of blocks from any lantern, take an orange cast [measured: the before frame and its crops]. The benchmark holds 148 emitters, 146 lanterns and 2 campfires [measured: the build log]. | **Ported**: the emitter grid (cells of 10 blocks, each listing the emitters in it and its 26 neighbours) and the four strategies (`NONE`, `ONE`, `ONE_BLOCK`, `ALL`; `ONE` is Lumenwright's default, Chunky's default is `NONE`); a uniform point on one face; the face counts when the first surface met towards it is at its distance; `|cos| / max(d², 1) × area × emittance × intensity × colour met`, times π; one emitter of the cell picked uniformly with no division by its probability (as Chunky's); `preventNormalEmitterWithSampling`. Cells start at the grid origin, where Chunky's start at its octree origin, the same point when the scene's lowest height is the world's. |
| Emitter materials: `chunky/chunk/BlockPalette.java` (`getDefaultMaterialProperties`), `Scene.DEFAULT_EMITTER_INTENSITY` | `lumenwright/src/assets/blockinfo.rs` (`emittance`); `request.rs` (`DEFAULT_EMITTER_INTENSITY`) | Lumenwright tied emitted light to the block's light level and drew a separate look (`--emitter-glow`). | **Ported**: one emittance per block material, on every face, entry for entry (keys as Chunky has them, so `magma_block` emits nothing as in Chunky, whose key is `minecraft:magma`); emitter intensity 13. **Not ported**: candle flames and the campfire's fire quads, which Chunky draws as particle entities that are its emitters (Lumenwright draws the client's block models; a campfire's whole model is its emitter); the `light` block, which has no faces here. **Same as Chunky, not a gap**: every texel of an emitting block emits its colour squared, so a lantern's dark blue-grey cap glows blue in Chunky's frame as in Lumenwright's [measured: the dusk crops]. Chunky has no per-texel emitter mapping at `156e2bb` nor at its master of 2026-10-20 (`f10ef3f`); a colour-based mapping (`EmitterMappingType`: reference colours, brightest channel, independent channels) exists only in the unmerged pull request chunky-dev/chunky#1627 [cited]. |
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
| + sky and sun until the sun's disc sets (`12737e4`) | 1.70 | 2 | 6 | 15 | 0.0036 | 0.0146 | 0.9593 |
| + the sun drawn with Chunky's texture (`c3017fc`) | 1.70 | 2 | 6 | 15 | 0.0036 | 0.0146 | 0.9593 |
| + Chunky's sky cache (`0b39f8f`) | 1.66 | 2 | 6 | 15 | 0.0035 | 0.0140 | 0.9594 |
| `0b39f8f` on CUDA at 4000 spp (converged against converged) | 1.19 | 1 | 4 | 13 | 0.0026 | 0.0100 | 0.9716 |

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

## Dusk

The same camera at `--time 12800`: Lumenwright puts the sun at altitude −0.005091 rad (a
quarter of the sun's radius below the horizon), azimuth π from +x towards +z, the west
[measured: the `hq sun` log line]. Chunky's scene is the benchmark's with that sun
(`altitude` −0.005091, `azimuth` 3.141593); everything else is Chunky's default as above.
Reference: Chunky at 4000 spp (37.9 minutes on the M2 Pro's CPU, shared with another job).
Each Lumenwright frame is 250 spp on Metal; CUDA gives the same bytes (display mean 0.00/255
between them at every revision below).

| frame | display mean | p50 | p90 | p99 | linear mean | linear p90 | SSIM |
|---|---|---|---|---|---|---|---|
| Chunky 250 spp (run 1) | 1.28 | 1 | 4 | 16 | 0.0012 | 0.0034 | 0.9536 |
| Chunky 250 spp (run 2) | 1.28 | 1 | 4 | 16 | 0.0011 | 0.0034 | 0.9531 |
| Lumenwright before (`ce325c3`, the port's head plus a log line) | 21.62 | 13 | 84 | 89 | 0.0771 | 0.5675 | 0.8527 |
| + sky and sun until the sun's disc sets (`12737e4`) | 1.29 | 1 | 5 | 15 | 0.0014 | 0.0051 | 0.9542 |
| + the sun drawn with Chunky's texture (`c3017fc`) | 1.25 | 1 | 5 | 14 | 0.0012 | 0.0046 | 0.9545 |
| + Chunky's sky cache (`0b39f8f`) | 1.21 | 1 | 4 | 14 | 0.0010 | 0.0033 | 0.9546 |
| `ce325c3` on CUDA at 4000 spp | 21.29 | 12 | 84 | 89 | 0.0769 | 0.5675 | 0.8913 |
| `0b39f8f` on CUDA at 4000 spp (converged against converged) | 0.67 | 1 | 3 | 6 | 0.0005 | 0.0019 | 0.9859 |

* [measured] Before, the camp-dusk frame was nearly black (mean RGB 47.9 46.9 28.5 against
  Chunky's 70.9 72.1 44.6): Lumenwright faded the sun to nothing below a sine of altitude of
  1/6 and the sky to 38% at this sun; Chunky fades neither (`Sun.initSun`,
  `PreethamSky.updateSun`) [cited]. After the first component the mean RGB is 70.7 72.1 44.5.
* [measured] Lumenwright's own horizon rule for the sun (a sampled direction below the
  horizon gives no light) changes nothing measurable here: without it the frame is display
  mean 1.28/255 against the reference, with it 1.29, and the two frames differ by 0.14/255.
* [measured] Chunky draws the sun as the client's texture, a square with a faint yellow ring
  (texels up to 40/255), not a disc; with it the region around the sun (x 880–1120, y 40–240)
  went from a mean difference of 1.95/255 to 1.14/255 against Chunky's 250 spp frame.
* [measured] Chunky's sky cache limits the sky to 1 per channel near the sun, so its hill
  silhouettes against the setting sun are darker than a directly evaluated sky gives. With the
  cache the hills against the sun (x 880–1200, y 80–280) went from +0.29/255 to +0.06/255
  signed against the reference, and the sky band at the top of the frame (y 0–60) from 0.13
  to 0.00/255.
* [measured] The lantern's blue cap and the lanterns' small reach at dusk are Chunky's too:
  the same crops of Chunky's frame show both. Emitters are Chunky's model at Chunky's
  intensity 13 (see Emitter materials).
* [measured] The bridge view at the same time (camera 60, 93, 22 looking at 60, 80, 42, field
  of view 70°, 250 spp each) sees almost no sky; against Chunky's 250 spp frame the mean RGB
  went from 23.6 20.6 11.4 to 32.1 29.9 17.0 (Chunky 32.2 29.9 17.0), display mean 8.83 to
  3.12/255, which is the two frames' noise. It stays a dark frame, in Chunky as here.

## Speed

The same frame, 250 spp, four bounces, at `0b39f8f` (after the dusk components). Chunky and Lumenwright on Metal ran alternately on the M2 Pro (10 CPU cores, 16 GPU
cores, JDK 17 for Chunky); Lumenwright on CUDA ran on the RTX 4070 Ti SUPER (WSL2). Two timers
each: the renderer's own (Chunky's `renderTime`; Lumenwright's `render` stage) and the whole
command (Chunky's includes starting the JVM and building its octree; Lumenwright's includes
reading the world and the jar). The CUDA command's wall time is read from the remote
machine's monotonic clock.

| renderer | runs, own timer (s) | median | runs, whole command (s) | median |
|---|---|---|---|---|
| Chunky 2.5.0-SNAPSHOT.474, CPU | 116.71, 115.51, 117.32 | 116.71 | 122.29, 119.68, 122.52 | 122.29 |
| Lumenwright, Metal | 31.98, 31.98, 32.00 | 31.98 | 32.32, 32.32, 32.36 | 32.32 |
| Lumenwright, CUDA | 6.09, 6.14, 6.16 | 6.14 | 6.67, 6.70, 6.74 | 6.70 |

* [measured] Lumenwright on Metal is 3.6 times as fast as Chunky on the same machine by the
  renderers' own timers (116.71 / 31.98) and 3.8 times by the whole command (122.29 /
  32.32). At `a3d7a74`, by the same method, the medians were 124.73 s and 32.96 s (3.8×).
  The sky cache is no slower than the direct sky: Metal's own timer went from 32.96 s to
  31.98 s.
* [measured] One-minute load average on the M2 Pro before each Chunky run: 7.69, 23.13
  (the previous Chunky run's threads, decaying), 10.71; before each Metal run 34.54, 15.90,
  14.93. Spotlight's indexer (`mds_stores`, 75–275% of a core) ran throughout; no build or
  render of this work ran during the pairs. The remote machine was idle (load 0.03, GPU 0%)
  before the CUDA runs.
* The frames are the measured ones: each Chunky run against the converged reference is
  display mean 1.99/255, and each Metal run is the same bytes as the measured `0b39f8f`
  frame.

## Not yet at parity

* Biome blending: Chunky averages biome colours over a 5 × 5 area; Lumenwright takes each
  block's own biome. Visible only at biome borders.
* Candle and campfire flame particles, fancier translucency, Chunky's water shading and its
  metals: not ported, and not examined in the benchmark frames.
* Twilight and night: Chunky has no time of day, and its Preetham sky keeps the sunset sky
  at any sun below the horizon. Below the sun's disc, Lumenwright's sky falls to a night's 3%
  at the end of civil twilight and its sun gives no light from below the horizon; frames with
  the sun more than its radius below the horizon are Lumenwright's own. Chunky's Nishita sky,
  which its documentation names for twilight, is not ported.
* Where the converged dusk frames still differ most (40-pixel blocks, 0.67/255 overall): the
  tree at the frame's left edge (x 0–40, y 520–640, a mean of 3.1/255; its trunk darker and
  its leaves lighter in Lumenwright's frame) [measured]; the cause is not examined (the leaf
  tint points at biome blending [inferred]). Every other block is within 1.6/255.
