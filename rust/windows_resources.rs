// Included by the build.rs of the programs: on Windows, give the executable
// the Real-Hi-Res icon and version information, as the Python build does.
// The icon's images are uncompressed (BMP): PNG-compressed icons are
// high-entropy resources, which antivirus heuristics count against a file.

use std::io::Write;
use std::path::Path;

/// An .ico with uncompressed 32-bit BMP images of the app icon.
fn write_ico(png: &Path, out: &Path) {
    let img = image::open(png).expect("icon.png").to_rgba8();
    let sizes = [16u32, 24, 32, 48, 64, 128, 256];
    let mut images: Vec<Vec<u8>> = vec![];
    for &s in &sizes {
        let im = image::imageops::resize(&img, s, s, image::imageops::FilterType::Lanczos3);
        let mask_row = s.div_ceil(32) * 4;
        let mut d = Vec::new();
        for v in [40u32, s, 2 * s] {
            d.extend_from_slice(&v.to_le_bytes());
        }
        d.extend_from_slice(&1u16.to_le_bytes()); // planes
        d.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        d.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
        d.extend_from_slice(&(s * s * 4 + mask_row * s).to_le_bytes());
        d.extend_from_slice(&[0u8; 16]); // resolution, palette
        for y in (0..s).rev() {
            for x in 0..s {
                let p = im.get_pixel(x, y).0;
                d.extend_from_slice(&[p[2], p[1], p[0], p[3]]);
            }
        }
        d.extend(std::iter::repeat_n(0u8, (mask_row * s) as usize)); // AND mask: alpha decides
        images.push(d);
    }
    let mut f = std::fs::File::create(out).expect("icon file");
    let n = sizes.len() as u16;
    f.write_all(&[0, 0, 1, 0]).unwrap();
    f.write_all(&n.to_le_bytes()).unwrap();
    let mut offset = 6 + 16 * n as u32;
    for (s, d) in sizes.iter().zip(&images) {
        let b = if *s >= 256 { 0 } else { *s as u8 };
        f.write_all(&[b, b, 0, 0]).unwrap();
        f.write_all(&1u16.to_le_bytes()).unwrap();
        f.write_all(&32u16.to_le_bytes()).unwrap();
        f.write_all(&(d.len() as u32).to_le_bytes()).unwrap();
        f.write_all(&offset.to_le_bytes()).unwrap();
        offset += d.len() as u32;
    }
    for d in &images {
        f.write_all(d).unwrap();
    }
}

pub fn windows_resources(name: &str, description: &str) {
    let png = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../realhires/assets/icon.png");
    println!("cargo:rerun-if-changed={}", png.display());
    println!("cargo:rerun-if-changed=../../windows_resources.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let ico = Path::new(&std::env::var("OUT_DIR").unwrap()).join("realhires.ico");
    write_ico(&png, &ico);
    let version = env!("CARGO_PKG_VERSION");
    let mut res = winresource::WindowsResource::new();
    res.set_icon(ico.to_str().unwrap())
        .set("CompanyName", "SU2H1")
        .set("FileDescription", description)
        .set("FileVersion", version)
        .set("InternalName", name)
        .set("LegalCopyright", "Copyright (c) SU2H1")
        .set("OriginalFilename", &format!("{name}.exe"))
        .set("ProductName", "Universal Music Player")
        .set("ProductVersion", version)
        .set("Comments", "https://github.com/SU2H1/Universal-Music-Player");
    res.compile().expect("Windows resources");
}
