fn main() {
    tauri_build::build();
    // Mock-runtime IPC tests retain Tauri's Windows dialog imports. Unlike the
    // packaged executable, Rust test harnesses do not inherit its resource
    // manifest; without Common Controls v6 they fail before main is entered.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg=/MANIFESTDEPENDENCY:type='win32' name='Microsoft.Windows.Common-Controls' version='6.0.0.0' processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'"
        );
        // Binaries already receive Tauri's complete resource.lib manifest.
        // Keep that resource and disable the linker's duplicate generated one.
        println!("cargo:rustc-link-arg-bins=/MANIFEST:NO");
    }
}
