//! Windows: embed the app icon as the executable's first icon resource.
//! The window class (`LoadIconW(hinstance, 1)`), the tray's no-icon
//! fallback and Explorer all read that resource; `icon.ico` is generated
//! from `icon.png` (resized 16–256).

fn main() {
    #[cfg(target_os = "windows")]
    {
        println!("cargo:rerun-if-changed=icon.ico");
        let mut res = winresource::WindowsResource::new();
        res.set_icon("icon.ico");
        res.set("FileDescription", "mdrv-oq");
        res.set("ProductName", "mdrv-oq");
        res.set("ProductVersion", env!("CARGO_PKG_VERSION"));
        res.compile().expect("embed the icon resource");
    }
}
