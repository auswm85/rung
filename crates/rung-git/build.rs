fn main() {
    // Link Windows system libraries required by libgit2
    if std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default() == "windows" {
        println!("cargo:rustc-link-lib=advapi32");
    }
}
