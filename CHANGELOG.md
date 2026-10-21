# Changelog

All notable changes to Lumenwright are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/).

## [0.2.0] - 2026-10-21

### Added

- HQ mode draws the game's own sky (`HqSkyModel::Game`, `--sky game`), lit as the 1.21.11
  client lights the tick, by the method Delvewright's engine lights its Chunky scenes with
  (`docs/research/game-sky.md`): the sun casts no light; the sky casts the colour the
  client's lightmap gives a cell open to it, from the jar's `visual/sky_light_factor` and
  `visual/sky_light_color` (now in `EnvironmentState`), with the directions of the client's
  sky; the lens sees the client's sky as quick mode draws it, its disc, fog, sunrise glow,
  sun, moon and stars. The host draws the sky into two equirectangular maps that every
  backend reads (`Params::sky_map`; `integrator::sky_lens`, `sky_light`). A jar without
  environment attributes is refused under it.
- The game look's emitter intensity, `GAME_EMITTER_INTENSITY` = 137.9628 (13 × 10.6125),
  authored by Lumenwright: fitted so that the Treehouse Camp's lantern-lit pixels at dusk
  read as Delvewright's engine's Chunky render of the frame does. `--emitter-intensity`
  takes any value over it.
- `--tonemap filmic-mid-grey` (`Tonemap::FilmicMidGrey`, `post::game_tone_curve`): Chunky's
  Hable curve read 1.463761 times brighter, so linear 0.18 shows at 0.18^(1/2.2) as the
  game's transfer shows it.

### Changed

- HQ mode's default sky goes by the time of day (`SkyLook::ByTime`, `--sky auto`): Chunky's
  Preetham sky and sun on the client's day plateau (ticks 730 to 11270, where
  `visual/sky_light_factor` is 1), the game's sky on the night plateau, and between them one
  render mixing the two by where the factor stands between its night and day values, with no
  jump from one tick to the next. `--sky chunky` and `--sky game` force either look.
  `HqSkyModel` is a struct (`look`, `render_distance`, `sun_intensity`, `sky_intensity`);
  `Sky::Vanilla` carries it in place of `sun_intensity` and `sky_intensity`.
  `--sun-intensity` and `--sky-intensity` are refused only under `--sky game`.
- `HqOptions::tonemap` and `emitter_intensity` are `Option`s: unstated, each look takes its
  own (`gamma` and 13 for Chunky's, `filmic-mid-grey` and `GAME_EMITTER_INTENSITY` for the
  game's), mixed by the day weight.
- `Params` is 26 vec4s (416 bytes): `sky_map` gives the sizes of the sky's maps. A sun of
  colour 0 draws no next-event sample.
- `Tonemap::Filmic` is evaluated as Chunky's `processPixel` evaluates it (the parameter
  products in float, the rest in double) through the one `HableCurve`.
- The HQ timing stage `sky cache` is `sky`.
- `lumenwright-cli` is published to crates.io with the other two crates (`cargo install
  lumenwright-cli`); the release workflow publishes `lumenwright-kernel`, `lumenwright` and
  `lumenwright-cli` in that order and refuses unless all three are absent at the version.
- `lumenwright-kernel` declares `rust-version` 1.88, the first release it builds on (its `if let`
  chains); 0.1.0 declared 1.85, on which it does not compile.

## [0.1.0] - 2026-10-20

The first release. The `lumenwright` library and its kernel `lumenwright-kernel` are published on
crates.io; the `lumenwright` command (`lumenwright-cli`) is installed from the repository at
the tag `v0.1.0`. 0.1.0 draws Minecraft Java Edition worlds in the vanilla client's look
or path traced as Chunky traces it, on the CPU, Metal, Vulkan, DX12 and CUDA.

### Added

- CJK and every other character no bitmap font covers are drawn, on signs and on text displays, from the Unifont the client reads out of the launcher's asset index (the jar's own unifont provider list is empty), cut and spaced as the client's unihex provider does. `Request::unifont` takes the directory (`minecraft/font/include/unifont.json` and the hex zip it names) and refuses a missing or unusable file by name (`RequestError::Unifont`), with no network. The command resolves it by `--unifont`, then its cache, then a SHA-1 and size verified download through the asset index unless `--offline`; `lumenwright assets fetch-unifont` fetches ahead.
- `lumenwright::render(&Request)`: one frame of a Minecraft Java Edition world as RGBA8 pixels,
  with every input explicit (world, chunk rectangle, client jar, resource packs, camera, mode,
  backend, size, options, entity policy, sky). It resolves no jar, uses no network, writes no
  file and prints nothing; a request refused before drawing carries a `RequestError`.
- Quick mode: a rasterizer with the vanilla client's look, with smooth lighting, ambient
  occlusion, biome tints, cutout and translucent layers, the client's world fog, and the
  client's sky (sky disc, sunrise glow, sun, moon phases and stars) from the jar's environment
  attributes and day timeline. Block textures are read through the client's mip chains with
  trilinear, 4× anisotropic filtering (`QuickOptions::mipmap_levels`, `--mipmap-levels`).
- HQ mode: a path tracer ported from Chunky's (chunky-dev/chunky, GPL-3.0; see
  `ACKNOWLEDGEMENTS.md` and `docs/research/chunky-parity.md`): the Preetham sky; Chunky's sun
  (irradiance intensity^2.2, default intensity 1.25) with next-event estimation; Chunky's
  emitters, with an emittance per block material, quadratic emitted light, intensity 13, the
  emitter grid and its sampling strategies (`--emitter-intensity`, `--emitter-sampling`,
  `--prevent-normal-emitter`); one-sided block faces; diffuse bounces; Fresnel water with
  refraction; stochastic translucency; textures decoded and frames encoded with the display
  gamma 2.2, and Chunky's gamma (default), ACES and Hable filters. The sky is read through
  Chunky's sky cache (a 129 × 129 table in HSL, each channel at most 1), and the sun is drawn
  with the client's sun texture as Chunky draws it. Chunky's sky and sun hold while any part
  of the sun's disc is above the horizon; below it the sky falls to a night's 3% at the end
  of civil twilight, and a sun direction below the horizon gives no light (Lumenwright's own,
  as Chunky has no time of day). On the treehouse benchmark
  frame its 250-sample frame is as close to Chunky's converged frame as Chunky's own
  250-sample frame is.
- Backends: CPU on every core, Metal, Vulkan and DX12 through wgpu (feature `gpu`, on by
  default) and CUDA (feature `cuda`, with the PTX `scripts/build-ptx.sh` builds in a clone of the
  repository, found in the clone or named by `LUMENWRIGHT_PTX`). On the CPU, a
  request gives the same bytes for any thread count. The CUDA kernel is the path tracer
  inlined whole, in f32 alone: `scripts/build-ptx.sh` refuses a kernel with an f64
  instruction or a call other than a panic. An HQ frame logs its kernel time by CUDA events.
- Worlds from 1.16 onward: Anvil regions (gzip, zlib, LZ4, uncompressed), 1.16/1.17 and 1.18+
  chunk layouts, stored sky and block light, entity regions, the Overworld, the Nether and the
  End.
- Blocks from the client jar's blockstates and models, with resource packs layered on top;
  water and lava geometry; entities from 57 bundled cuboid models, plus block displays,
  paintings, item frames, mannequins and players built in code, with a hitbox box for the rest; and the
  block entities vanilla draws in code: signs and hanging signs with their text, chests,
  banners, bells, lectern and enchanting-table books, decorated pots, skulls and heads.
- The `lumenwright` command: `render` (PNG, and a linear `.pfm` in HQ mode), `info`, `assets`
  (find or download the client jar from Mojang, SHA-1 verified), `backends` and `demo-world`.
