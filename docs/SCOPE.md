# Lumenwright — requirements scope (prototype v0.1)

Lumenwright is an offline, headless renderer: it takes a Minecraft Java Edition world
save plus CLI arguments and writes an image of the scene, blocks and entities, seen from
a given camera.

## 1. Goals

| # | Requirement | Prototype decision |
|---|-------------|--------------------|
| G1 | Input is a world save directory and CLI args | `lumenwright render --world <dir> ...` reads `level.dat`, `region/*.mca`, `entities/*.mca`. It never writes to the save. |
| G2 | Output is a rendered image | PNG, 8-bit sRGB. HQ mode can also write the linear HDR accumulation as `.pfm`. |
| G3 | Camera is fully controlled by CLI | Position, yaw/pitch in Minecraft's own convention or `--look-at`, vertical FOV, resolution. If you give no camera, it uses the single-player position from `level.dat`. |
| G4 | Render blocks **and** entities | Blocks use the vanilla blockstate and model JSON from the client jar. Entities use a data-driven cuboid model file shipped with Lumenwright, because vanilla hard-codes mob geometry in Java code (see §4). |
| G5 | Vanilla textures acquired the way Chunky does it | Chunky does not ship Mojang assets. It reads them from the official client jar, which it downloads from Mojang's version manifest (`piston-meta.mojang.com`) into its own cache. Lumenwright does the same, with this resolution order: `--minecraft-jar` → a launcher's or the cache's jar for the world's version (`--mc-version` overrides it) → Chunky's `~/.chunky/resources/minecraft.jar` if its version matches → a SHA-1-verified download from Mojang. Extra `--resource-pack` zips or directories layer on top of it. |
| G6 | **Quick mode**: looks like the vanilla Java client, no ray tracing | A triangle rasterizer with a z-buffer, like vanilla: per-face directional shading, smooth lighting with ambient occlusion from the save's sky and block light, biome tints, cutout and translucent layers, distance fog, and a sky gradient. |
| G7 | **HQ mode**: looks like Chunky | A unidirectional path tracer over a sparse voxel grid: sun and sky lighting with next-event estimation for the sun, emissive blocks, multiple diffuse bounces, Fresnel water with refraction, stochastic translucency, ACES or gamma tonemapping, progressive sample accumulation. |
| G8 | Multiple native rendering backends | One `Backend` trait. **cpu**: Rust and rayon on all cores. **metal / vulkan / dx12**: native GPU APIs through `wgpu`, with WGSL shaders that naga translates to MSL, SPIR-V or HLSL. **cuda**: the same Rust path-tracing kernel crate that the CPU backend runs, compiled to PTX (`nvptx64-nvidia-cuda`) and launched through the CUDA driver API (`cudarc`, cargo feature `cuda`). |
| G9 | Runnable Rust prototype, tested locally | Cargo workspace, unit and integration tests, a parity test between the CPU and GPU backends, and end-to-end renders of a real 1.21.11 save on this M2 Pro. |

## 2. In scope for the prototype

* **World format.** Anvil with paletted sections (1.18+ `sections[].block_states`) and the
  1.16/1.17 `Level.Sections` layout. Region compression types gzip, zlib, none and LZ4.
  Entity region files (1.17+) and legacy in-chunk entities. Stored `SkyLight` and `BlockLight`
  are used when present, with a heightmap-based fallback. Overworld, Nether and End are
  selected with `--dimension`.
* **Block models.** Blockstate `variants` (including weighted random variants) and
  `multipart` (`when`, with OR/AND). Model parent inheritance, texture variables, element
  rotation with rescale, x/y model rotation, an approximation of uvlock, face UV rotation,
  cullface, tintindex, and `shade`.
* **Special blocks.** Fluid geometry for water and lava (level heights, neighbour face
  culling) is generated in code. Missing models are drawn as the vanilla magenta and black
  "missing" texture so gaps stay visible.
* **Tints.** Grass, foliage and water colours per biome, using the jar's colormaps and a
  table of biome temperature and downfall. Fixed tints for birch, spruce and lily pad.
* **Entities.** Every entity listed in `assets/entity_models.json`, posed by `Pos` and
  `Rotation`. Variant textures come from the NBT `variant` field. Block displays, paintings
  and item frames (through their vanilla block models) are built in code. Players from `level.dat` are optional (`--players`). An unknown entity is
  drawn as a box the size of its hitbox (`--unknown-entities box|skip`).
* **Lighting controls.** `--time` (ticks), or `--sun-altitude` and `--sun-azimuth`.
  `--exposure`, `--tonemap`, `--spp`, `--max-bounces`, `--ssaa` (quick mode).
* **Tooling.** An `info` subcommand prints the world version, spawn, player position, and
  block and entity statistics. An `assets` subcommand fetches the client jar.

## 3. Out of scope (explicitly deferred)

| Deferred | Why | Path forward |
|----------|-----|--------------|
| Block entities drawn by Java code: chests, beds, signs, banners, skulls, shulker boxes | Like mobs, they have no JSON models | Same cuboid model file as entities (the format already supports them) |
| Item rendering: dropped items, item frame contents, display entities | Needs the item model pipeline (`items/*.json`) | Reuse block baking for block items; extruded sprites for flat items |
| Clouds, rain and snow, mob animation, armor, held items, player skins from the network | Not needed for the core scene | Incremental |
| Biome blending, mipmaps, anisotropic filtering in quick mode | Cosmetic | Incremental |
| Denoising (Chunky uses an OIDN plugin) | External dependency | Optional `oidn` feature |
| Emitter next-event estimation, volumetric fog, a physical sky model (Chunky's Nishita sky) | Quality improvements | The kernel structure already allows them |
| CUDA quick mode | Rasterization is served by `wgpu` (Vulkan on NVIDIA) | Quick mode on `--backend cuda` falls back to the CPU rasterizer with a warning |
| Pre-1.16 worlds (numeric IDs, pre-flattening) | Legacy | Not planned |
| Executing the CUDA backend locally | No NVIDIA GPU on the development machine (Apple M2 Pro) | The kernel is compiled to PTX and the host code builds with `--features cuda`. It needs a run on an NVIDIA box. |

## 4. Key design decisions

1. **One scene, two representations.** The world is baked once into a list of quads in
   block-local or world space, with atlas UVs. Quick mode turns visible quads into a
   triangle mesh. HQ mode instances per-block models inside a two-level voxel grid of
   16³ bricks: empty bricks are skipped, uniform or fully buried bricks are collapsed, and
   voxels are u16 model-instance IDs. Entity quads sit in sparse "overlay" bricks, so a ray
   needs only one traversal loop and no BVH.
2. **Backend parity by construction.** The path-tracing kernel is a `#![no_std]` crate
   (`lumenwright-kernel`) with `#[repr(C)]` scene types. The CPU backend calls it directly
   and the CUDA backend runs the same source compiled to PTX. The WGSL kernel mirrors it
   line by line, uses the same buffer layout and the same RNG (PCG hash), and is checked
   against the CPU backend by a statistical parity test.
3. **Texture sampling is buffer-based and nearest-neighbour** in every backend, which
   matches Minecraft's pixel look and keeps CPU and GPU output identical.
4. **Entity models are data, not code.** Vanilla mob geometry lives in Java classes, so
   Lumenwright ships `entity_models.json` in vanilla's own part/cube/box-UV terms. New mobs
   need no Rust changes.

## 5. Acceptance criteria for v0.1

* `cargo test --workspace` passes. Tests that need the client jar or a GPU skip cleanly
  when either is missing.
* `lumenwright render --world <1.21.11 save> --mode quick --backend cpu|metal` and
  `--mode hq --backend cpu|metal` all produce plausible images of the same view with
  blocks and entities visible.
* Quick mode on the GPU renders a 1920×1080 frame of a 12-chunk radius in seconds. HQ
  mode at 1280×720 and 256 spp on Metal finishes in minutes, not hours.
* The CPU and Metal backends agree: quick mode differs in under 1% of pixels, and HQ
  mean luminance differs by under 3%.
* `--features cuda` builds, and the PTX kernel compiles.

## 6. Status at v0.1 (2026-10-14, Apple M2 Pro, Minecraft 1.21.11 assets)

| Criterion | Result |
|-----------|--------|
| Tests | `cargo test --workspace --release`: 33 unit, 8 kernel and 7 integration tests pass. The integration tests ran for real (jar and Metal adapter present). |
| Real save, both modes, CPU + Metal | `datapack_test` (1.21.11) renders in all four combinations. Blocks, water, mannequins, an enderman, 68 rotated and scaled block displays, and glow squids (drawn as fallback boxes) all appear. |
| Quick mode performance | 33×33 chunks at 1920×1080: 0.33 s end to end on Metal (24 ms raster) and 0.29 s on CPU (65 ms raster). |
| HQ mode performance | 1280×720 at 256 spp: 21 s on Metal for 33×33 chunks, 11.8 s for the showcase. The CPU is about 6× slower. |
| Backend parity | Quick mode, CPU vs Metal: 0.075% of pixels differ by more than 8/255. HQ at 32 spp: 0.03% of pixels differ, because both backends use the same RNG streams. |
| Determinism | Identical inputs give bit-identical images on CPU and Metal, in both modes. |
| CUDA | `scripts/build-ptx.sh` compiles the shared kernel to PTX: ISA 7.0, sm_70, no unresolved externs. `--features cuda` builds, and without a driver the backend reports `libcuda not found` cleanly. **Not executed:** there is no NVIDIA GPU on the development machine. |
| Vulkan / DX12 | Same wgpu and WGSL code as Metal. Not exercised on macOS: wgpu builds neither backend there by default. |

Follow-ups, in priority order:
1. Run the CUDA backend and the Vulkan/DX12 parity tests on a Linux or Windows machine with an NVIDIA GPU.
2. Entity layers with a second texture: sheep wool, drowned outer layer, glowing eyes.
3. Block-entity models: chests, beds, signs, banners, skulls.
4. Emitter next-event estimation, for faster convergence in caves and at night.
5. Item rendering for item frames and item displays.
6. A stress test on a dense, full-height natural world. Only a flat test world and generated fixtures were available locally.
