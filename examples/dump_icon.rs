//! Dumps the tray icon as PNGs so its rendering can be inspected.
//! `cargo run --example dump_icon`.
use nsh_core::svg;

fn main() {
    for (name, svg) in [
        ("icon42", mdrv_oq_core::render::icon_svg(42.0, 90.0)),
        ("icon90", mdrv_oq_core::render::icon_svg(90.0, 90.0)),
        ("placeholder", mdrv_oq_core::render::icon_svg_placeholder()),
    ] {
        let r = svg::rasterize(svg.as_bytes(), 64).expect("rasterize");
        let mut rgba = r.pixels.clone();
        for p in rgba.chunks_exact_mut(4) {
            let a = p[3] as u32;
            if a > 0 {
                p[0] = ((p[0] as u32 * 255 + a / 2) / a).min(255) as u8;
                p[1] = ((p[1] as u32 * 255 + a / 2) / a).min(255) as u8;
                p[2] = ((p[2] as u32 * 255 + a / 2) / a).min(255) as u8;
            }
        }
        let path = format!("/tmp/opencode/{name}.rgba");
        std::fs::write(&path, &rgba).unwrap();
        println!("{name}: {}x{}", r.width, r.height);
        std::process::Command::new("magick")
            .args([
                "-size",
                &format!("{}x{}", r.width, r.height),
                "-depth",
                "8",
                &format!("rgba:{path}"),
                &format!("/tmp/opencode/{name}.png"),
            ])
            .status()
            .expect("magick");
    }
}
