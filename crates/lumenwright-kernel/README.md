# lumenwright-kernel

The path-tracing kernel of [Lumenwright](https://crates.io/crates/lumenwright), an offline
renderer for Minecraft Java Edition worlds.

It traces one path sample per call through a scene laid out in flat `#[repr(C)]` buffers: a
two-level grid of 16³ voxel bricks holding block-model ids, the models' quads, sparse entity
overlay cells and a texture atlas. The same source runs on the host, where Lumenwright's CPU
backend calls it on every core, and on NVIDIA GPUs, compiled to PTX (`nvptx64-nvidia-cuda`) for
the CUDA backend. It is `#![no_std]` without the `std` feature and allocates nothing.

It does not read worlds, bake models or write images. Most programs want
[`lumenwright`](https://crates.io/crates/lumenwright), which builds the buffers and calls this
crate.

## Features

* `libm` (default): pure-Rust float math, for `no_std` targets.
* `std`: the standard library's float math, for host builds. When both are enabled, `std` is
  used.

## Licence

GPL-3.0-or-later. See [LICENSE](LICENSE).
