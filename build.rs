fn main() {
    // The Win32 windowing/input APIs used by src/main.rs live in user32.
    println!("cargo:rustc-link-lib=user32");
}
