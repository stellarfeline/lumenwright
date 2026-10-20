# Lumenwright — requirements scope (prototype v0.1)

Lumenwright is an offline, headless renderer: it takes a Minecraft Java Edition world
save and a camera and draws the scene, blocks and entities. It is two crates: the
`lumenwright` library, whose `render(&Request)` returns the image to the calling program,
and `lumenwright-cli`, the `lumenwright` command, which writes it.

## 1. Goals

| # | Requirement | Prototype decision |
|---|-------------|--------------------|
| G1 | Input is a world save directory and CLI args | `lumenwright render --world <dir> ...` reads `level.dat`, `region/*.mca`, `entities/*.mca`. It never writes to the save. |
| G2 | Output is a rendered image | PNG, 8-bit sRGB. HQ mode can also write the linear HDR accumulation as `.pfm`. |
| G3 | Camera is fully controlled by CLI | Position, yaw/pitch in Minecraft's own convention or `--look-at`, vertical FOV, resolution. If you give no camera, it uses the single-player position from `level.dat`. |
| G4 | Render blocks **and** entities | Blocks use the vanilla blockstate and model JSON from the client jar. Entities, and block entities vanilla draws in Java code, use data-driven cuboid model files shipped with Lumenwright, because vanilla hard-codes their geometry in Java code (see §4). |
| G5 | Vanilla textures acquired the way Chunky does it | Chunky does not ship Mojang assets. It reads them from the official client jar, which it downloads from Mojang's version manifest (`piston-meta.mojang.com`) into its own cache. Lumenwright does the same, with this resolution order: `--minecraft-jar` → a launcher's or the cache's jar for the world's version (`--mc-version` overrides it) → Chunky's `~/.chunky/resources/minecraft.jar` if its version matches → a SHA-1-verified download from Mojang. Extra `--resource-pack` zips or directories layer on top of it. The Unifont (CJK and every character no bitmap font covers) is not in the jar: it comes from the version's asset index by the same shape, `--unifont` → the cache (hash-checked against its lock) → a SHA-1 and size verified download unless `--offline`. This resolution and the downloader are `lumenwright-cli`'s; the library takes the jar and the Unifont directory as paths. |
| G6 | **Quick mode**: looks like the vanilla Java client, no ray tracing | A triangle rasterizer with a z-buffer, like vanilla: per-face directional shading, smooth lighting with ambient occlusion from the save's sky and block light, biome tints, cutout and translucent layers, block textures through the client's mip chains with trilinear, 4× anisotropic filtering, the client's world fog, and the client's sky (sky disc, sunrise glow, sun, moon phases, stars) from the jar's environment attributes and day timeline. |
| G7 | **HQ mode**: looks like Chunky | A unidirectional path tracer over a sparse voxel grid: sun and sky lighting, next-event estimation for the sun and for emissive blocks (with MIS), emissive blocks that cast light by their light level, multiple diffuse bounces, Fresnel water with refraction, stochastic translucency, ACES or gamma tonemapping, progressive sample accumulation. |
| G8 | Multiple native rendering backends | One `Backend` trait. **cpu**: Rust and rayon on all cores. **metal / vulkan / dx12**: native GPU APIs through `wgpu`, with WGSL shaders that naga translates to MSL, SPIR-V or HLSL. **cuda**: the same Rust path-tracing kernel crate that the CPU backend runs, compiled to PTX (`nvptx64-nvidia-cuda`) and launched through the CUDA driver API (`cudarc`, cargo feature `cuda`). |
| G9 | Runnable Rust prototype, tested locally | Cargo workspace, unit and integration tests, a parity test between the CPU and GPU backends, and end-to-end renders of a real 1.21.11 save on this M2 Pro. |
| G10 | A library entry point for an embedding program | `lumenwright::render(&Request) -> Result<Rendered>` (`src/request.rs`). The request carries every input: the world or an opened `World`, an inclusive `ChunkRect`, the jar and resource-pack paths, the camera, mode, backend, size, `QuickOptions`, `HqOptions`, `EntityPolicy`, and a `Sky` that is either derived from a time of day or stated by the caller (quick colours and daylight factor, HQ sun and sky). It resolves no jar, performs no network access, writes no file and prints nothing. It returns RGBA8 pixels, the HQ radiance, the backend and the reason `auto` fell back to the CPU, stage timings, loaded counts and the per-kind `EntityStats` and `BlockEntityStats`. A refusal before drawing is a `RequestError`. |

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
* **Items.** Dropped items, item frame contents, item displays, the items in the hands and
  on the head of armour stands, mannequins, players and humanoid mobs, and worn armour.
  A stack resolves through the jar's item model definition (`assets/<ns>/items/<id>.json`,
  1.21.4+) against its components and display context; a property that needs a live
  client (an item in use, the clock, the compass) takes the value of an item at rest. A
  model with elements is baked as a block model and a flat item (`builtin/generated`) is
  extruded as vanilla's `ItemModelGenerator` does; the model's display transform for the
  context poses it. Armour layers come from the jar's `assets/<ns>/equipment/*.json` on the
  humanoid armour meshes, and the shield is drawn; both are data in
  `assets/item_models.json`. `EntityStats` counts the stacks drawn and not drawn by item id.
* **Block entities.** Signs and hanging signs with their text (vanilla bitmap font from the
  jar, dye colour, glowing text and outline, `translate` keys from the `en_us` language
  files of the jar and resource packs), bells, lectern and enchanting-table books,
  chests (single, double, trapped, ender, copper), banners with pattern layers, decorated
  pots with sherds, skulls and heads. Geometry is `assets/block_entity_models.json`;
  placement and per-block data come from the chunk's `block_entities`. The quads join the
  entity quads, so both modes and every backend draw the same geometry.
  `--no-block-entities` leaves them out.
* **Text displays.** The `text` component (colour, bold, italic, `translate`) in the jar's
  `minecraft:default` font, split as `Font.split` splits it at `line_width`, `alignment`,
  `background` or `default_background`, `shadow`, `see_through`, `billboard` (turned to the
  camera) and `transformation`, placed as `TextDisplayRenderer` places them. They are
  one-sided; a see-through display is drawn last with no depth test in quick mode, and in HQ
  mode the camera path meets it behind whatever hid it. Not read: `text_opacity`,
  `brightness`. Characters outside the jar's bitmap fonts are drawn from the Unifont of the
  launcher's asset index (the jar's `include/unifont` reference has no providers), as the client's
  unihex provider cuts and spaces them.
* **Lighting controls.** `--time` (ticks), or `--sun-altitude` and `--sun-azimuth`.
  `--exposure`, `--tonemap`, `--spp`, `--max-bounces`, `--ssaa` (quick mode).
* **Tooling.** An `info` subcommand prints the world version, spawn, player position, and
  block and entity statistics. An `assets` subcommand fetches the client jar and shows which
  jar a version resolves to. `backends` lists the backends available on the machine, and
  `demo-world` writes generated worlds to try things on.

## 3. Out of scope (explicitly deferred)

| Deferred | Why | Path forward |
|----------|-----|--------------|
| Beds, shulker boxes, the conduit; open chest lids, ringing bells, italic or obfuscated sign text | Not needed yet | Add models to `block_entity_models.json` and a placement in `scene/block_entities.rs` |
| Player head skins from the network | Renders read the save and local packs only | A skin given as a resource-pack `texture` in the profile is used |
| Items vanilla draws in Java code other than the shield (`minecraft:special`: chests, beds, banners, heads, shulker boxes, decorated pots, the conduit, copper golem statues); a filled map's picture; armour trims and the elytra | Each is a Java model or needs data beyond the item (the save's map data) | Models in `item_models.json`, as the shield is; the skull and chest models already exist in `block_entity_models.json` |
| Clouds, rain and snow, mob animation, player skins from the network | Not needed for the core scene | Incremental |
| Biome blending in quick mode; the End's sky box and the below-horizon dark disc in the quick sky; the 1.21.11 sky-light attributes in the quick lightmap | Cosmetic | Incremental |
| Denoising (Chunky uses an OIDN plugin) | External dependency | Optional `oidn` feature |
| A spatial light structure (light BVH or ReSTIR), volumetric fog, a physical sky model (Chunky's Nishita sky) | Quality improvements | The kernel structure already allows them |
| CUDA quick mode | Rasterization is served by `wgpu` (Vulkan on NVIDIA) | Quick mode on `--backend cuda` falls back to the CPU rasterizer with a warning |
| Pre-1.16 worlds (numeric IDs, pre-flattening) | Legacy | Not planned |

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
   need no Rust changes. Block entities drawn in Java code use the same format in
   `block_entity_models.json`, each model citing its client class; the placement of each
   kind (the renderer's pose transform) and its per-block data are code.

5. **One render path.** `render(&Request)` is the pipeline. The CLI's `render` subcommand
   (`lumenwright-cli`'s `job::run`) resolves the jar, derives the camera and a square of chunks around it from
   the flags, calls `render`, and writes the PNG and PFM. Nothing draws a frame any other
   way, so the CLI and an embedding program get the same pixels for the same inputs.
6. **The library is a library only.** The command line is its own crate,
   `lumenwright-cli`, which builds the `lumenwright` binary and calls only the library's
   public API. It owns argument parsing (`clap`), logging set-up (`env_logger`), the jar
   resolver and downloader (`ureq`, its TLS stack, the SHA-1 check), the PNG and PFM
   writers, and the fixture worlds. Its flag values mirror the library's `Mode`,
   `BackendKind`, `Tonemap` and `UnknownEntities` with exhaustive conversions both ways.
   The library's `zip` is built without default features and reads deflate only, through
   the `flate2` decoder the region reader uses. With the default features (`gpu`) the
   library's normal dependency tree contains no C build script and no `*-sys` crate other
   than platform bindings (`wayland-sys`, `windows-sys`, `renderdoc-sys`, and for Android
   and the web `ndk-sys`, `jni-sys`, `js-sys`, `web-sys`).
7. **CPU determinism across thread counts.** On the CPU backend the pixels and the HQ
   radiance depend on the request alone, not on the rayon thread count. The library tests
   render one request in both modes on 1, 3 and 8 threads and compare the bits.

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
* `-p lumenwright-cli --features cuda` builds, and the PTX kernel compiles.
* The CLI's PNG and `render`'s pixels for the same fixture and view are byte-equal in both
  modes. A request whose jar path does not exist fails with `RequestError::JarNotFound` before
  anything is read.
* `cargo check -p lumenwright --lib --no-default-features --features gpu --target <t>`
  passes from a macOS host with no cross C compiler for `x86_64-unknown-linux-gnu`,
  `aarch64-unknown-linux-gnu`, `x86_64-apple-darwin`, `aarch64-apple-darwin` and
  `x86_64-pc-windows-msvc`.
* On the CPU backend, one request gives bit-identical pixels and radiance on 1, 3 and 8
  threads.

## 6. Status at v0.1 (2026-10-14, Apple M2 Pro, Minecraft 1.21.11 assets)

| Criterion | Result |
|-----------|--------|
| Tests | `cargo test --workspace`: 42 library unit, 2 CLI unit, 8 kernel and 13 integration tests (9 render, 4 library) and 1 doc test pass. The integration tests ran for real (jar and Metal adapter present). |
| One render path | The CLI's PNG equals the library's pixels in both modes on the block-entity gallery (integration test) and on the showcase at 1280×720 quick and 640×360 HQ, CPU and Metal. |
| Embedding build | The library's default features (`gpu`): 96 packages in the normal tree on macOS, 98 on Linux gnu, 109 on Windows MSVC; `cargo check` passes for the five targets above. |
| Real save, both modes, CPU + Metal | *The Treehouse Camp* (1.21.11, CC BY-SA 4.0, from `stellarfeline/delvewright-campaigns`, saved by its own server as the README describes) renders in all four combinations: hilly terrain, giant trees, houses, rope bridges, lanterns, ladders and mannequins. The README shows it at day and at dusk in both modes. `datapack_test` (1.21.11) also renders in all four: blocks, water, mannequins, an enderman, 68 rotated and scaled block displays, and glow squids (drawn as fallback boxes). |
| Quick mode performance | The Treehouse Camp, 625 chunks at 1920×1080: 0.53 s end to end on Metal (77 ms raster, 982k triangles) and 0.51 s on CPU (144 ms raster). |
| HQ mode performance | The Treehouse Camp at 1280×720: 21 s at 256 spp on Metal, 8.3 s at 16 spp on CPU (about 6× slower per sample). |
| Backend parity | The Treehouse Camp, CPU vs Metal: in quick mode at 1920×1080, 0.36% of pixels differ by more than 8/255; in HQ at 32 spp, 0.16% do, because both backends use the same RNG streams. |
| Determinism | Identical inputs give bit-identical images on CPU and Metal, in both modes. On the CPU, the same Treehouse Camp render twice gives byte-identical PNGs in quick mode and byte-identical PNG and PFM in HQ mode. |
| CUDA | `scripts/build-ptx.sh` compiles the shared kernel to PTX: ISA 7.0, sm_70, no unresolved externs, no f64 instruction, no call but panics. `-p lumenwright-cli --features cuda` builds, and without a driver the backend reports `libcuda not found` cleanly. On an RTX 4070 Ti SUPER (WSL2, driver 616.92) `cuda_matches_cpu_in_hq_mode` passes with 0.000% of pixels off on all three scenes, and the showcase at 1280×720 and 64 spp takes 0.85 s in the kernel. |
| Vulkan / DX12 | Same wgpu and WGSL code as Metal. Not exercised on macOS: wgpu builds neither backend there by default. |

Follow-ups, in priority order:
1. Run the Vulkan/DX12 parity tests on a Linux or Windows machine with an NVIDIA GPU.
2. Entity layers with a second texture: sheep wool, drowned outer layer, glowing eyes.
3. Remaining block-entity models: beds, shulker boxes, the conduit.
4. A spatial light structure, so emitter sampling stays efficient with many emitters in view.
5. Item rendering for item frames and item displays.
6. A stress test on a dense, full-height natural world. The real saves tested are a flat test world and The Treehouse Camp, whose terrain is placed by the map's datapack over a void world, not grown by world generation.
