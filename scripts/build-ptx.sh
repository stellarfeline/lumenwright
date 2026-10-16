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
cp "$root/crates/lumenwright-ptx/target/nvptx64-nvidia-cuda/release/lumenwright_ptx.ptx" "$root/target/ptx/lumenwright.ptx"
echo "wrote $root/target/ptx/lumenwright.ptx ($(grep -c '' "$root/target/ptx/lumenwright.ptx") lines)"
