//! Embeds the application manifest (asInvoker, PerMonitorV2, Common Controls v6)
//! and the app icon. Uses the MSVC linker's native manifest support, and writes
//! the icon as a tiny `.res` file directly (the linker accepts `.res` inputs),
//! so no resource-compiler crate or rc.exe is needed.

use std::path::Path;

fn main() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest = dir.join("memmanager.exe.manifest");
    let icon = dir.join("memmanager.ico");
    println!("cargo:rerun-if-changed={}", manifest.display());
    println!("cargo:rerun-if-changed={}", icon.display());

    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if target_os == "windows" && target_env == "msvc" {
        println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg-bins=/MANIFESTINPUT:{}",
            manifest.display()
        );
        println!("cargo:rustc-link-arg-bins=/MANIFESTUAC:NO");
        if let Ok(ico) = std::fs::read(&icon) {
            let out = Path::new(&std::env::var("OUT_DIR").unwrap()).join("icon.res");
            std::fs::write(&out, icon_res(&ico)).expect("write icon.res");
            println!("cargo:rustc-link-arg-bins={}", out.display());
        }
    }
}

/// One RES record: header (ordinal type and name) followed by DWORD-aligned data.
fn record(out: &mut Vec<u8>, ty: u16, name: u16, flags: u16, data: &[u8]) {
    let le32 = |v: u32| v.to_le_bytes();
    let le16 = |v: u16| v.to_le_bytes();
    out.extend(le32(data.len() as u32)); // DataSize
    out.extend(le32(32)); // HeaderSize
    out.extend(le16(0xFFFF));
    out.extend(le16(ty));
    out.extend(le16(0xFFFF));
    out.extend(le16(name));
    out.extend(le32(0)); // DataVersion
    out.extend(le16(flags)); // MemoryFlags
    out.extend(le16(0x0409)); // LanguageId: en-US
    out.extend(le32(0)); // Version
    out.extend(le32(0)); // Characteristics
    out.extend_from_slice(data);
    while out.len() % 4 != 0 {
        out.push(0);
    }
}

/// Converts an .ico file into RT_ICON images plus an RT_GROUP_ICON (id 1).
fn icon_res(ico: &[u8]) -> Vec<u8> {
    const RT_ICON: u16 = 3;
    const RT_GROUP_ICON: u16 = 14;
    let u16_at = |o: usize| u16::from_le_bytes([ico[o], ico[o + 1]]);
    let u32_at = |o: usize| u32::from_le_bytes([ico[o], ico[o + 1], ico[o + 2], ico[o + 3]]);
    let count = u16_at(4) as usize;
    let mut res = Vec::new();
    record(&mut res, 0, 0, 0, &[]); // the empty leading record every .res starts with
    let mut group = Vec::new();
    group.extend(0u16.to_le_bytes());
    group.extend(1u16.to_le_bytes());
    group.extend((count as u16).to_le_bytes());
    for i in 0..count {
        let e = 6 + 16 * i;
        let (bytes, offset) = (u32_at(e + 8) as usize, u32_at(e + 12) as usize);
        let id = (i + 1) as u16;
        record(&mut res, RT_ICON, id, 0x1010, &ico[offset..offset + bytes]);
        group.extend_from_slice(&ico[e..e + 12]); // width..bytes-in-resource
        group.extend(id.to_le_bytes());
    }
    record(&mut res, RT_GROUP_ICON, 1, 0x1030, &group);
    res
}
