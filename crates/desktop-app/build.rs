//! Embeds `assets/re-search.ico` as the executable icon on Windows. If no resource compiler
//! is available the build still succeeds, only without the icon.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/re-search.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/re-search.ico")
        .set("FileDescription", "reSearch")
        .set("ProductName", "reSearch")
        .set("InternalName", "re-search-app");
    if let Err(e) = res.compile() {
        println!("cargo:warning=re-search-app: executable icon not embedded ({e})");
    }
}
