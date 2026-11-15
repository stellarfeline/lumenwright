#!/usr/bin/env bash
# Compiles lumenwright-kernel to PTX for the CUDA backend.
# Output: crates/lumenwright/ptx/lumenwright.ptx, the file the `cuda` feature embeds. It is
# committed and packaged into the crate, so building the backend needs neither nightly Rust nor
# this script; CI rebuilds it from a clean target and refuses any difference from the committed
# file, so the packaged PTX is the kernel source's.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
# The PTX crate uses `#![feature(abi_ptx)]`, so it needs nightly; the nightly is pinned so the
# PTX is a function of the kernel source (a different nightly is a different LLVM).
toolchain=nightly-2026-10-15
cd "$root/crates/lumenwright-ptx"
rustup toolchain list | grep -q "^$toolchain" || rustup toolchain install "$toolchain" --profile minimal >/dev/null
rustup target add --toolchain "$toolchain" nvptx64-nvidia-cuda >/dev/null
rustup component add --toolchain "$toolchain" llvm-bitcode-linker llvm-tools >/dev/null
cargo "+$toolchain" build --release --locked
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
out="$root/crates/lumenwright/ptx/lumenwright.ptx"
mkdir -p "$(dirname "$out")"
cp "$ptx" "$out"
sum=$( (command -v sha256sum >/dev/null && sha256sum "$out" || shasum -a 256 "$out") | cut -d' ' -f1)
echo "wrote $out ($(grep -c '' "$out") lines, sha256 $sum, 0 f64 instructions, no calls but panics)"
