// The CUDA backend embeds the PTX produced by scripts/build-ptx.sh.
fn main() {
    println!("cargo:rerun-if-changed=../../target/ptx/lumenwright.ptx");
    if std::env::var_os("CARGO_FEATURE_CUDA").is_some() {
        let ptx = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/ptx/lumenwright.ptx");
        if !ptx.exists() {
            panic!(
                "the `cuda` feature needs {}: run scripts/build-ptx.sh first",
                ptx.display()
            );
        }
    }
}
