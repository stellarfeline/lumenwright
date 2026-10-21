# Backends

[Lumenwright](../README.md)

## Backends

| backend | quick | hq | tested here (Apple M2 Pro) |
|---|---|---|---|
| cpu | software rasterizer | Rust kernel on rayon | yes |
| metal | wgpu render pipeline | WGSL compute | yes, matches the CPU (see below) |
| vulkan / dx12 | same wgpu code | same WGSL | no, but these are the same code paths as Metal |
| cuda | falls back to the CPU rasterizer | Rust kernel as PTX | not on the M2 Pro; on an RTX 4070 Ti SUPER (WSL2) HQ matches the CPU |

The CUDA backend needs the PTX built first:

```sh
scripts/build-ptx.sh                       # rustup target nvptx64-nvidia-cuda + llvm-bitcode-linker
cargo build --release -p lumenwright-cli --features cuda
lumenwright render ... --mode hq --backend cuda
```

`libcuda` is loaded at runtime, so CUDA builds work on machines without the CUDA toolkit.

`scripts/build-ptx.sh` refuses a kernel whose PTX carries an f64 instruction (consumer NVIDIA
GPUs run f64 at a small fraction of f32 speed) or calls a function other than a panic. The
path tracer is inlined whole into the kernel because, with its functions called, the driver's
optimizing PTX compiler (driver 616.92) produced wrong pixels or launches that never finished,
while the same PTX compiled without optimization matched the CPU.

On WSL2 the driver library is `/usr/lib/wsl/lib/libcuda.so.1`. A Linux NVIDIA driver package
installed inside WSL (Ubuntu's `libnvidia-compute-*`, pulled in by `nvidia-utils-*`) puts a
native `libcuda.so` in `/usr/lib/x86_64-linux-gnu`, which the backend loads first and which
finds no device (`cuInit` returns 100, `CUDA_ERROR_NO_DEVICE`). Run with
`LD_LIBRARY_PATH=/usr/lib/wsl/lib`, or remove that package.

An HQ frame logs its kernel time by CUDA events beside the wall clock (`--log info`).
