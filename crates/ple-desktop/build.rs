fn main() {
    println!("cargo:rerun-if-changed=../../packaging/icons/kohdalab-ple.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("../../packaging/icons/kohdalab-ple.ico")
            .set("ProductName", "KohdaLab PLE")
            .set("FileDescription", "KohdaLab PLE")
            .compile()
            .expect("compile KohdaLab PLE Windows icon resource");
    }
}
