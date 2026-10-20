#!/usr/bin/env bash
# Compiles lumenwright-kernel to PTX for the CUDA backend.
# Output: target/ptx/lumenwright.ptx (embedded by `cargo build --features cuda`).
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root/crates/lumenwright-ptx"
# The PTX crate uses `#![feature(abi_ptx)]`, so it builds with nightly whatever the default toolchain is.
rustup toolchain list | grep -q '^nightly' || rustup toolchain install nightly --profile minimal >/dev/null
rustup target add --toolchain nightly nvptx64-nvidia-cuda >/dev/null
rustup component add --toolchain nightly llvm-bitcode-linker llvm-tools >/dev/null
cargo +nightly build --release
mkdir -p "$root/target/ptx"
ptx="$root/crates/lumenwright-ptx/target/nvptx64-nvidia-cuda/release/lumenwright_ptx.ptx"
# Consumer NVIDIA GPUs run f64 at a small fraction of f32 speed: refuse a kernel that carries
# any f64 instruction (an opcode with a .f64 type; `.reg .f64` declarations are not counted).
f64=$(awk '$1 !~ /^[.\/$({})]/ { op = ($1 ~ /^@/) ? $2 : $1; if (op ~ /\.f64/) n++ } END { print n + 0 }' "$ptx")
if [ "$f64" -ne 0 ]; then
    echo "refused: $ptx carries $f64 f64 instructions; find their source (an f64 libm routine is the usual one) and use an f32 form" >&2
    exit 1
fi
# The path tracer is one function: with render_sample and trace called as PTX functions, the
# driver's optimizing JIT mistranslated the kernel (see `pt_render` in crates/lumenwright-ptx).
# Refuse a call to anything but a panic, which only traps.
calls=$(grep -E '^\s*(@!?%p[0-9]+\s+)?call(\.uni)?\s' "$ptx" | grep -vc 'panicking' || true)
if [ "$calls" -ne 0 ]; then
    echo "refused: $ptx makes $calls calls to functions other than panics; keep the kernel inlined whole" >&2
    exit 1
fi
cp "$ptx" "$root/target/ptx/lumenwright.ptx"
echo "wrote $root/target/ptx/lumenwright.ptx ($(grep -c '' "$root/target/ptx/lumenwright.ptx") lines, 0 f64 instructions, no calls but panics)"
