fn main() {
    let dir = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let manifest = dir.join("margin.manifest");
    println!("cargo:rerun-if-changed=margin.manifest");
    println!("cargo:rerun-if-changed=assets/margin.ico");
    println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg-bins=/MANIFESTINPUT:{}", manifest.display());

    // App icon and version info shown in Explorer and the file's Properties.
    let mut res = winresource::WindowsResource::new();
    res.set_icon(dir.join("assets/margin.ico").to_str().unwrap());
    res.set("ProductName", "Margin");
    res.set("FileDescription", "Margin writing assistant");
    res.set("LegalCopyright", "Yohann Zapata");
    res.compile().expect("compile Windows resources (needs the Windows SDK's rc.exe)");
}
