# Backends

[Lumenwright](../README.md)

## Backends

| backend | quick | hq | where it opens |
|---|---|---|---|
| cpu | software rasterizer | Rust kernel on rayon | everywhere |
| metal | wgpu render pipeline | WGSL compute | macOS |
| vulkan | same wgpu code | same WGSL | Linux, Windows, with a hardware Vulkan driver |
| dx12 | same wgpu code | same WGSL | Windows |
| gl | same wgpu code | same WGSL | Linux, Windows (OpenGL through EGL or WGL) |
| cuda | falls back to the CPU rasterizer | Rust kernel as PTX | an NVIDIA GPU and its driver |

`gpu` names the best hardware wgpu adapter on any of Metal, Vulkan, DX12 and GL: a discrete
GPU before an integrated one before any other, and among equals Metal, Vulkan, DX12, then GL.
A software adapter (llvmpipe, lavapipe, WARP) is never taken; the error names it.

Every backend is in a default build. `lumenwright backends` prints what this machine can open
and what `auto` takes in each mode.

## What `auto` takes

`BackendKind::Auto` takes the first backend of `render::auto_order(mode)` that opens:

* **HQ**: `cuda`, then `gpu`, then `cpu`. CUDA comes first because it runs the CPU's own
  kernel compiled to PTX, so its frame is nearest the reference kernel's (below: 0.0015% of
  pixels off, by 1/255, against Metal's 0.13% by up to 7/255); `gpu` runs the WGSL port of
  the kernel. CUDA against Vulkan on one NVIDIA GPU has not been timed.
* **Quick**: `gpu`, then `cpu`. CUDA has no rasterizer, so it is passed over.

`Rendered::backend_fallback` names each backend `auto` passed over and why, in order, for
example `cuda: CUDA driver library (libcuda) not found` on a Mac, or
`cuda: no rasterizer for quick mode; gpu: only a software gpu adapter (llvmpipe ... via Vulkan, ...)`
on WSL2 with no Vulkan driver. A backend named outright that cannot open is an error.

## Pixels against the CPU, and time

The showcase (`demo-world`) at 1280×720, HQ, 64 spp, default camera, each backend's PNG
compared with the CPU's pixel by pixel; time is the command's wall clock (`/usr/bin/time -p`,
loading the world and the jar included), the release build of `lumenwright-cli`. The CUDA
row's tolerance holds on Delvewright's gallery `hall-exterior` camera too (1600×900, 300
spp): 62 pixels (0.0043%) differ from the CPU's, each by 1/255.

| machine | backend | pixels against the CPU | wall |
|---|---|---|---|
| Apple M2 Pro, macOS | cpu (10 threads) | (reference) | 17.21 s |
| Apple M2 Pro, macOS | metal | not byte-identical: 0.13% of pixels differ, by at most 7/255; mean luminance equal to 3 decimals | 3.70 s, 3.66 s |
| RTX 4070 Ti SUPER, Ryzen 7 7800X3D, WSL2 | cpu (16 threads) | (reference) | 14.56 s, 17.71 s (other jobs sharing the CPU) |
| RTX 4070 Ti SUPER, Ryzen 7 7800X3D, WSL2 | cuda | not byte-identical: 14 pixels (0.0015%) differ, each by 1/255; mean luminance equal to 3 decimals | 2.23 s, 2.19 s |

Vulkan, DX12 and GL run the same wgpu and WGSL code as Metal; no hardware adapter of those
APIs was at hand to measure (WSL2 offers llvmpipe only, for Vulkan and GL).

On the CPU backend a request gives byte-identical pixels for any thread count. A wgpu frame is
byte-stable on one machine and driver, not across them.

## The CUDA kernel's PTX

The CUDA backend embeds `crates/lumenwright/ptx/lumenwright.ptx`, the shared kernel compiled
for `nvptx64-nvidia-cuda`. It is committed and packaged in the crate, so the backend builds
from crates.io with the stable toolchain, no CUDA toolkit and nothing downloaded. `libcuda` is
loaded at run time: a machine without the driver builds and runs, and reports the backend
unavailable.

After a change to `lumenwright-kernel` or `crates/lumenwright-ptx`, rebuild it and commit
the result:

```sh
scripts/build-ptx.sh    # pinned nightly, nvptx64-nvidia-cuda, llvm-bitcode-linker
```

The script pins its nightly (a different nightly is a different LLVM, so different PTX). CI's
`ptx` job runs it twice from a clean target and fails when the first build differs from the
committed file or the second from the first. Two clean builds on an Apple M2 Pro and one on
x86-64 Linux gave the same 843832 bytes (sha256
`bff8122b741d9fd672a3f5565acdbf5c36a6ab6b97339f704a61f826f5e576dc`).

`scripts/build-ptx.sh` refuses a kernel whose PTX carries an f64 instruction (consumer NVIDIA
GPUs run f64 at a small fraction of f32 speed) or calls a function other than a panic. The
path tracer is inlined whole into the kernel because, with its functions called, the driver's
optimizing PTX compiler (driver 616.92) produced wrong pixels or launches that never finished,
while the same PTX compiled without optimization matched the CPU.

## CUDA under WSL2

The driver library is `/usr/lib/wsl/lib/libcuda.so`. A Linux NVIDIA driver package installed
inside WSL (Ubuntu's `libnvidia-compute-*`, pulled in by `nvidia-utils-*`) puts a native
`libcuda.so` in `/usr/lib/x86_64-linux-gnu`, which the loader's cache gives for the name
`libcuda.so` and which finds no device (`CUDA_ERROR_NO_DEVICE`). The backend's error then
names the library it loaded and the remedy: run with `LD_LIBRARY_PATH=/usr/lib/wsl/lib`, or
remove that package.

An HQ frame logs its kernel time by CUDA events beside the wall clock (`--log info`).
