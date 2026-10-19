# Changelog

All notable changes to Lumenwright are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/).

## [0.1.0] - Unreleased

The first release: `lumenwright`, `lumenwright-cli` and `lumenwright-kernel`.

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
- HQ mode: a path tracer in the style of Chunky, with sun and sky light, emissive blocks
  with next-event estimation and multiple importance sampling (each block casts light by its
  light level, calibrated against the game's block light; `--emitter-intensity`,
  `--emitter-glow`, `--no-light-sampling`), diffuse bounces, Fresnel water with refraction, stochastic translucency and ACES, filmic or
  gamma tonemapping.
- Backends: CPU on every core, Metal, Vulkan and DX12 through wgpu (feature `gpu`, on by
  default) and CUDA (feature `cuda`, built from a clone of the repository). On the CPU, a
  request gives the same bytes for any thread count.
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
