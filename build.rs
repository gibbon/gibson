// Windows: give gibson.exe its icon and version info.
fn main() {
    println!("cargo:rerun-if-changed=assets/icon/gibson.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon/gibson.ico");
        res.set("ProductName", "Gibson");
        res.set("FileDescription", "Gibson - a 3D file browser");
        if let Err(e) = res.compile() {
            println!("cargo:warning=no icon embedded: {e}");
        }
    }
}
