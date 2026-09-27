//! Windows: gives the executable its icon and version information, so Explorer, Task Manager, the
//! taskbar and GPUI's windows (which load icon resource 1) show them. The resources are written
//! as a `.res` file, which the MSVC linker takes directly, so no resource compiler is needed.

use std::path::PathBuf;

/// Resource types and the ids used here.
const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
const RT_VERSION: u16 = 16;
/// US English, the language of the strings below.
const LANGUAGE: u16 = 0x0409;
/// Unicode (UTF-16) code page for the string table.
const CODE_PAGE: u16 = 1200;

fn main() {
    let ico = PathBuf::from("../tachyon-platform/assets/tachyon.ico");
    println!("cargo:rerun-if-changed={}", ico.display());
    let windows = std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "windows");
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").is_ok_and(|env| env == "msvc");
    if !windows || !msvc {
        return;
    }
    let ico = std::fs::read(&ico).expect("the icon is in the repository");
    let version = std::env::var("CARGO_PKG_VERSION").expect("set by cargo");
    let mut res = Vec::new();
    // A .res file starts with an empty entry.
    push_resource(&mut res, 0, 0, &[]);
    push_icons(&mut res, &ico);
    push_resource(&mut res, RT_VERSION, 1, &version_info(&version));
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("set by cargo")).join("tachyon.res");
    std::fs::write(&out, res).expect("OUT_DIR is writable");
    println!("cargo:rustc-link-arg-bins={}", out.display());
}

/// Appends one resource: a header (numeric type and name), then the data, both DWORD-aligned.
fn push_resource(res: &mut Vec<u8>, kind: u16, name: u16, data: &[u8]) {
    let empty = kind == 0;
    res.extend((data.len() as u32).to_le_bytes());
    res.extend(32u32.to_le_bytes()); // header size
    // Type and name as ordinals (0xffff, then the number); the leading empty entry uses 0 and 0.
    res.extend([0xff, 0xff]);
    res.extend(kind.to_le_bytes());
    res.extend([0xff, 0xff]);
    res.extend(name.to_le_bytes());
    res.extend(0u32.to_le_bytes()); // data version
    // Moveable, pure, discardable, the usual flags for these resources.
    res.extend((if empty { 0u16 } else { 0x1030 }).to_le_bytes());
    res.extend((if empty { 0 } else { LANGUAGE }).to_le_bytes());
    res.extend(0u32.to_le_bytes()); // version
    res.extend(0u32.to_le_bytes()); // characteristics
    res.extend(data);
    while !res.len().is_multiple_of(4) {
        res.push(0);
    }
}

/// The `.ico` file's images as `RT_ICON` resources 1..=n, and group icon 1 listing them.
fn push_icons(res: &mut Vec<u8>, ico: &[u8]) {
    let count = u16::from_le_bytes([ico[4], ico[5]]);
    let mut group = Vec::new();
    group.extend(0u16.to_le_bytes());
    group.extend(1u16.to_le_bytes()); // icons
    group.extend(count.to_le_bytes());
    for i in 0..usize::from(count) {
        let entry = &ico[6 + 16 * i..22 + 16 * i];
        let len = u32::from_le_bytes([entry[8], entry[9], entry[10], entry[11]]) as usize;
        let offset = u32::from_le_bytes([entry[12], entry[13], entry[14], entry[15]]) as usize;
        let id = i as u16 + 1;
        push_resource(res, RT_ICON, id, &ico[offset..offset + len]);
        // The group entry is the file entry with the image's offset replaced by its id.
        group.extend(&entry[..12]);
        group.extend(id.to_le_bytes());
    }
    push_resource(res, RT_GROUP_ICON, 1, &group);
}

/// `VS_VERSIONINFO` with the fixed file info and an English string table.
fn version_info(version: &str) -> Vec<u8> {
    let parts: Vec<u16> =
        version.split(['.', '-']).take(3).map(|p| p.parse().unwrap_or(0)).collect();
    let part = |i: usize| u32::from(parts.get(i).copied().unwrap_or(0));
    let (ms, ls) = ((part(0) << 16) | part(1), part(2) << 16);
    let mut fixed = Vec::new();
    for value in [
        0xfeef04bd,  // signature
        0x0001_0000, // structure version
        ms,
        ls,
        ms,
        ls,
        0x3f,        // flags mask
        0,           // flags
        0x0004_0004, // VOS_NT_WINDOWS32
        1,           // VFT_APP
        0,
        0,
        0,
    ] {
        fixed.extend(u32::to_le_bytes(value));
    }
    let strings: Vec<u8> = [
        ("CompanyName", "Tachyon"),
        ("FileDescription", "Tachyon"),
        ("FileVersion", version),
        ("InternalName", "tachyon"),
        ("LegalCopyright", "MIT OR Apache-2.0"),
        ("OriginalFilename", "tachyon.exe"),
        ("ProductName", "Tachyon"),
        ("ProductVersion", version),
    ]
    .into_iter()
    .flat_map(|(key, value)| block(key, 1, &utf16z(value), &[]))
    .collect();
    let table = block(&format!("{LANGUAGE:04x}{CODE_PAGE:04x}"), 1, &[], &strings);
    let string_info = block("StringFileInfo", 1, &[], &table);
    let translation = [LANGUAGE.to_le_bytes(), CODE_PAGE.to_le_bytes()].concat();
    let var_info = block("VarFileInfo", 1, &[], &block("Translation", 0, &translation, &[]));
    block("VS_VERSION_INFO", 0, &fixed, &[string_info, var_info].concat())
}

/// A version-info block: length, value length (in words for text, bytes for binary), type,
/// UTF-16 key, padding, value, padding, children.
fn block(key: &str, text: u16, value: &[u8], children: &[u8]) -> Vec<u8> {
    let mut out = vec![0; 6];
    out.extend(utf16z(key));
    pad(&mut out);
    out.extend(value);
    pad(&mut out);
    out.extend(children);
    let value_len = if text == 1 { value.len() / 2 } else { value.len() };
    // The length leaves out the padding after the block.
    let len = out.len();
    out[0..2].copy_from_slice(&(len as u16).to_le_bytes());
    out[2..4].copy_from_slice(&(value_len as u16).to_le_bytes());
    out[4..6].copy_from_slice(&text.to_le_bytes());
    // Children must start aligned, so each block ends padded.
    pad(&mut out);
    out
}

fn utf16z(text: &str) -> Vec<u8> {
    text.encode_utf16().chain(Some(0)).flat_map(u16::to_le_bytes).collect()
}

fn pad(out: &mut Vec<u8>) {
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
}
