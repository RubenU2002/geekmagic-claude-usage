//! Computes RENDER_ID: a hash of everything that decides how a screen looks (the drawing code and the pure logic it draws
//! from). The device keeps the images it was sent, so a changed id makes the app redraw every stored view once.
use std::{
    fs, io,
    path::{Path, PathBuf},
};

fn collect(dir: &Path, out: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    Ok(())
}

fn main() {
    let mut files = Vec::new();
    for dir in ["src/adapters/render", "src/core"] {
        let _ = collect(Path::new(dir), &mut files);
    }
    files.sort();
    let mut hash: u64 = 0xcbf29ce484222325; // FNV-1a
    for file in &files {
        for byte in fs::read(file).unwrap_or_default() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
    }
    println!("cargo:rustc-env=RENDER_ID={:012x}", hash & 0xffff_ffff_ffff);
    println!("cargo:rerun-if-changed=src/adapters/render");
    println!("cargo:rerun-if-changed=src/core");
    println!("cargo:rerun-if-changed=assets");
}
