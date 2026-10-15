#!/usr/bin/env bash
# Compiles lumenwright-kernel to PTX for the CUDA backend.
# Output: target/ptx/lumenwright.ptx (embedded by `cargo build --features cuda`).
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
rustup target add nvptx64-nvidia-cuda >/dev/null
rustup component add llvm-bitcode-linker llvm-tools >/dev/null
cd "$root/crates/lumenwright-ptx"
cargo build --release
mkdir -p "$root/target/ptx"
cp "$root/crates/lumenwright-ptx/target/nvptx64-nvidia-cuda/release/lumenwright_ptx.ptx" "$root/target/ptx/lumenwright.ptx"
echo "wrote $root/target/ptx/lumenwright.ptx ($(grep -c '' "$root/target/ptx/lumenwright.ptx") lines)"
