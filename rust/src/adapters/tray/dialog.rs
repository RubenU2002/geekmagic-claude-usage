//! The system's own "save as" window (Windows).

use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows_sys::Win32::UI::Controls::Dialogs::{GetSaveFileNameW, OFN_NOCHANGEDIR, OFN_OVERWRITEPROMPT, OFN_PATHMUSTEXIST, OPENFILENAMEW};

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Ask where to save a picture (a .png, or a .gif to keep it animated): the path chosen, None if cancelled. It opens in
/// `start_dir`, with `suggested_name` filled in.
pub fn ask_save_path(start_dir: &Path, suggested_name: &str) -> Option<String> {
    const CAPACITY: usize = 2048;
    let mut file: Vec<u16> = suggested_name.encode_utf16().collect();
    file.resize(CAPACITY, 0);
    let filter: Vec<u16> = "Imagen PNG\0*.png\0GIF animado\0*.gif\0\0".encode_utf16().collect();
    let title = wide("Guardar pantallazo");
    let extension = wide("png");
    let dir: Vec<u16> = start_dir.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    // SAFETY: every pointer in the struct outlives the call, the buffer is as long as `nMaxFile` says, and the struct is
    // zero-initialised before the fields we use are set.
    unsafe {
        let mut ofn: OPENFILENAMEW = std::mem::zeroed();
        ofn.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
        ofn.lpstrFilter = filter.as_ptr();
        ofn.lpstrFile = file.as_mut_ptr();
        ofn.nMaxFile = CAPACITY as u32;
        ofn.lpstrInitialDir = dir.as_ptr();
        ofn.lpstrTitle = title.as_ptr();
        ofn.lpstrDefExt = extension.as_ptr();
        ofn.Flags = OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR;
        if GetSaveFileNameW(&mut ofn) == 0 {
            return None; // cancelled
        }
    }
    let end = file.iter().position(|c| *c == 0).unwrap_or(file.len());
    let chosen = String::from_utf16_lossy(&file[..end]);
    (!chosen.is_empty()).then_some(chosen)
}
