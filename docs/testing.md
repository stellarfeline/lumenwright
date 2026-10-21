# Testing

[Lumenwright](../README.md)

## Testing

```sh
cargo test --workspace --release
```

* **Unit tests** cover region and NBT decoding, the LZ4 framing, palette unpacking, multipart
  rules, model baking orientation and UVs, the camera convention, sky and day cycle, the
  environment attribute layering, mip chains (the client's lookup tables and cutout
  coverage), quick-mode texture sampling, the lightmap, the tonemapper, and entity box-UV
  and orientation.
* **Kernel tests** use hand-built buffers to check brick skipping, uniform bricks, entity
  overlay cells, cutout alpha, one-sided faces met far along a ray, coplanar faces, the
  sky, Chunky's emitter sampling against its expectation, and pinned RNG values; the kernel's
  `f32` `ln`, `acos` and gamma decoding are held to bounds against `f64`. A unit test holds
  the kernel's sky to Chunky's Preetham sky transcribed in `f64`.
* **Integration tests** (in `lumenwright-cli`, which owns the fixture worlds) write fixture
  worlds and render them end to end. They check terrain
  colours seen from above, entity coverage, block entities in both modes, sign text, items
  in both modes (every holder kind, each item's spot against the same frame with the
  holders empty), that every item definition in the jar resolves, HQ sanity, CPU-vs-GPU
  parity in both modes (showcase, block-entity gallery and item gallery) and CPU-vs-CUDA
  parity in HQ mode, determinism, and the CLI. The library tests check that the CLI's PNG
  and the library's pixels are byte-equal in both modes, that a missing jar is refused by
  name, that a stated sky is drawn as given, that a stated sky at a tick draws `Sky::Vanilla`'s frame at it byte for byte (quick, and HQ under the game's look and by time), that a stated sky at no tick draws 0.2.0's frame byte for byte (0.2.0 as published is a dev-dependency), that the vanilla sky's colour at noon, dusk and
  midnight matches the jar's day timeline within 2/255, and that CPU pixels are
  bit-identical on 1, 3 and 8 threads (showcase and item gallery). A far-plane test measures
  the high-frequency energy and the change under a sub-pixel camera move with and without
  mipmaps. Tests that need the
  client jar or a GPU skip with a message when it is missing. `LUMENWRIGHT_MC_JAR` points the tests at a jar.
