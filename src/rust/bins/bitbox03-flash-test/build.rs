// SPDX-License-Identifier: Apache-2.0

fn main() {
    // cortex-m-rt's link.x includes memory.x from the linker search path.
    let out_dir = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    std::fs::copy("memory.x", out_dir.join("memory.x")).unwrap();
    println!("cargo::rerun-if-changed=memory.x");
    println!("cargo::rustc-link-search={}", out_dir.display());
    println!("cargo::rustc-link-arg=-Tlink.x");
}
