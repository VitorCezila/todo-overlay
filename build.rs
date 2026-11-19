fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");

    // Embute o ícone no .exe quando ele existir (marco 5).
    if std::path::Path::new("assets/icon.ico").exists()
        && std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        if let Err(e) = res.compile() {
            println!("cargo:warning=falha ao embutir ícone: {e}");
        }
    }
}
