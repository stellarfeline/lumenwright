// The CUDA backend embeds the PTX produced by scripts/build-ptx.sh.
//
// The PTX is a build product of a clone of the repository (nightly + nvptx64-nvidia-cuda),
// so it is not packaged. The `cuda` feature takes it from `LUMENWRIGHT_PTX` when that is set,
// otherwise from the clone's `target/ptx/lumenwright.ptx`, and refuses to build without it.
// Nothing is fetched.
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-env-changed=LUMENWRIGHT_PTX");
    if std::env::var_os("CARGO_FEATURE_CUDA").is_none() {
        return;
    }
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let clone = manifest.join("../../scripts/build-ptx.sh").is_file();
    let ptx: PathBuf = match std::env::var_os("LUMENWRIGHT_PTX") {
        Some(p) => PathBuf::from(p),
        None if clone => manifest.join("../../target/ptx/lumenwright.ptx"),
        None => panic!(
            "the `cuda` feature needs the kernel's PTX: build it with scripts/build-ptx.sh in a clone of \
             https://github.com/stellarfeline/lumenwright and set LUMENWRIGHT_PTX to the file it writes \
             (target/ptx/lumenwright.ptx)"
        ),
    };
    if !ptx.is_file() {
        panic!(
            "the `cuda` feature needs {}: run scripts/build-ptx.sh first{}",
            ptx.display(),
            if clone {
                ""
            } else {
                ", or set LUMENWRIGHT_PTX to the PTX it writes"
            }
        );
    }
    let ptx = ptx.canonicalize().expect("canonicalize the PTX path");
    println!("cargo:rerun-if-changed={}", ptx.display());
    println!("cargo:rustc-env=LUMENWRIGHT_PTX_FILE={}", ptx.display());
}
