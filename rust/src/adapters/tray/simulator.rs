//! The simulated screen (Windows): a window that plays what the GeekMagic would be showing, for when it isn't there.
//!
//! The window lives on a thread of its own, made when it is opened and gone when it is closed. Everything about it happens on
//! that thread; the rest of the app talks to it by posting it a message.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::Arc;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint, FillRect, InvalidateRect, SetBkMode, SetStretchBltMode, SetTextColor,
    StretchDIBits, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, COLORONCOLOR, DIB_RGB_COLORS, DT_CENTER, DT_SINGLELINE, DT_VCENTER, PAINTSTRUCT,
    SRCCOPY, TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRectEx, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, GetMessageW, LoadCursorW, PostMessageW,
    PostQuitMessage, RegisterClassW, SetForegroundWindow, SetTimer, SetWindowPos, SetWindowTextW, ShowWindow, TranslateMessage, CS_HREDRAW,
    CS_VREDRAW, HWND_NOTOPMOST, HWND_TOPMOST, IDC_HAND, MSG, SWP_NOMOVE, SWP_NOSIZE, SW_RESTORE, SW_SHOWNORMAL, WM_APP, WM_CLOSE, WM_DESTROY,
    WM_ERASEBKGND, WM_LBUTTONUP, WM_PAINT, WM_TIMER, WNDCLASSW, WS_CAPTION, WS_MINIMIZEBOX, WS_OVERLAPPED, WS_SYSMENU,
};

use crate::adapters::render::gif::{all_frames_rgb, FRAME_DELAY_CS};
use crate::application::local_screen::Latest;
use crate::core::themes::BG;
use crate::util::in_background;

const SIDE: i32 = 240; // the real screen is 240 x 240 pixels
const SCALE: i32 = 1; // ...and drawn at that size: pixel for pixel what the real one shows
const TIMER_ID: usize = 1;
const SHOW: usize = 1; // messages the rest of the app posts to the window (WM_APP + n would do too: one is enough)
const CLOSE: usize = 2;
const CLICK: usize = 3; // what a click in the window does, asked for from outside (the tests do)

type LatestFn = Arc<dyn Fn() -> Option<Latest> + Send + Sync>;

struct Shared {
    latest: LatestFn,
    on_click: Arc<dyn Fn() + Send + Sync>,
    title: Arc<dyn Fn() -> String + Send + Sync>,
    hwnd: AtomicIsize, // 0 while there is no window
    open: AtomicBool,  // a window is open or being made
}

pub struct SimulatorWindow {
    shared: Arc<Shared>,
}

/// What the window thread keeps: the frames being played.
struct State {
    shared: Arc<Shared>,
    shown: Option<(String, u64)>,
    frames: Vec<Vec<u8>>, // BGRA, SIDE x SIDE, top-down
    index: usize,
    title: String,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

impl SimulatorWindow {
    pub fn new(latest: LatestFn, on_click: Arc<dyn Fn() + Send + Sync>, title: Arc<dyn Fn() -> String + Send + Sync>) -> Arc<Self> {
        Arc::new(Self { shared: Arc::new(Shared { latest, on_click, title, hwnd: AtomicIsize::new(0), open: AtomicBool::new(false) }) })
    }

    pub fn is_open(&self) -> bool {
        self.shared.open.load(Ordering::SeqCst)
    }

    /// Open the window, or bring it to the front.
    pub fn show(&self) {
        if self.is_open() {
            self.post(SHOW);
        } else {
            self.start();
        }
    }

    /// Open the window, or close it if it is open.
    pub fn toggle(&self) {
        if self.is_open() {
            self.post(CLOSE);
        } else {
            self.start();
        }
    }

    pub fn close(&self) {
        if self.is_open() {
            self.post(CLOSE);
        }
    }

    /// Ask the window to do something. (A window that is still being made shows itself when it is ready anyway.)
    pub fn post(&self, what: usize) {
        let hwnd = self.shared.hwnd.load(Ordering::SeqCst);
        if hwnd != 0 {
            // SAFETY: posting a message to a window handle; if the window is gone the call just fails.
            unsafe {
                PostMessageW(hwnd as HWND, WM_APP, what, 0);
            }
        }
    }

    /// The window's own title, for finding it from outside (the tests).
    pub fn hwnd(&self) -> isize {
        self.shared.hwnd.load(Ordering::SeqCst)
    }

    pub fn click(&self) {
        self.post(CLICK);
    }

    fn start(&self) {
        if self.shared.open.swap(true, Ordering::SeqCst) {
            return; // (already being made)
        }
        let shared = self.shared.clone();
        let spawned = std::thread::Builder::new().name("simulated-screen".into()).stack_size(1024 * 1024).spawn({
            let shared = shared.clone();
            move || {
                run_window(&shared);
                shared.hwnd.store(0, Ordering::SeqCst);
                shared.open.store(false, Ordering::SeqCst);
            }
        });
        if spawned.is_err() {
            shared.open.store(false, Ordering::SeqCst);
        }
    }
}

/// The window's thread: make it, play, and return when it is closed.
fn run_window(shared: &Arc<Shared>) {
    // SAFETY: plain Win32 window creation and the standard message loop, all on this thread; the state the window
    // procedure reads is thread-local to it.
    unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        let class = wide("GeekMagicSimulatedScreen");
        let mut wc: WNDCLASSW = std::mem::zeroed();
        wc.style = CS_HREDRAW | CS_VREDRAW;
        wc.lpfnWndProc = Some(window_proc);
        wc.hInstance = instance;
        wc.hCursor = LoadCursorW(std::ptr::null_mut(), IDC_HAND);
        wc.lpszClassName = class.as_ptr();
        RegisterClassW(&wc); // (a second time it says "already there", which is fine)

        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
        let mut rect = RECT { left: 0, top: 0, right: SIDE * SCALE, bottom: SIDE * SCALE };
        AdjustWindowRectEx(&mut rect, style, 0, 0);
        let title = wide(&(shared.title)());
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            title.as_ptr(),
            style,
            120,
            120,
            rect.right - rect.left,
            rect.bottom - rect.top,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            log::warn!("could not open the simulated screen");
            return;
        }
        STATE.with(|s| *s.borrow_mut() = Some(State { shared: shared.clone(), shown: None, frames: Vec::new(), index: 0, title: String::new() }));
        shared.hwnd.store(hwnd as isize, Ordering::SeqCst);
        SetTimer(hwnd, TIMER_ID, u32::from(FRAME_DELAY_CS) * 10, None);
        tick(hwnd);
        bring_to_front(hwnd);

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        STATE.with(|s| *s.borrow_mut() = None);
    }
}

/// To the front once (it is not kept there).
unsafe fn bring_to_front(hwnd: HWND) {
    ShowWindow(hwnd, SW_SHOWNORMAL);
    ShowWindow(hwnd, SW_RESTORE);
    SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
    SetWindowPos(hwnd, HWND_NOTOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
    SetForegroundWindow(hwnd);
}

/// One beat: take a new picture if there is one, else the next frame; keep the title current; ask for a repaint.
unsafe fn tick(hwnd: HWND) {
    let fresh = STATE.with(|s| {
        let mut s = s.borrow_mut();
        let state = s.as_mut()?;
        let found = (state.shared.latest)();
        match found {
            Some((view, version, gif)) if state.shown.as_ref() != Some(&(view.clone(), version)) => {
                state.shown = Some((view.clone(), version));
                match all_frames_rgb(&gif) {
                    Ok((w, h, frames)) if (w as i32, h as i32) == (SIDE, SIDE) => {
                        state.frames = frames.iter().map(|rgb| to_bgra(rgb)).collect();
                        state.index = 0;
                    }
                    Ok(_) => log::warn!("the picture of {view} is not {SIDE} x {SIDE}"),
                    Err(e) => log::warn!("could not show the picture of {view}: {e}"),
                }
            }
            _ if state.frames.len() > 1 => state.index = (state.index + 1) % state.frames.len(),
            _ => {}
        }
        let title = (state.shared.title)();
        (title != state.title).then(|| {
            state.title = title.clone();
            title
        })
    });
    if let Some(title) = fresh {
        SetWindowTextW(hwnd, wide(&title).as_ptr());
    }
    InvalidateRect(hwnd, std::ptr::null(), 0);
}

fn to_bgra(rgb: &[u8]) -> Vec<u8> {
    rgb.chunks_exact(3).flat_map(|p| [p[2], p[1], p[0], 0]).collect()
}

unsafe fn paint(hwnd: HWND) {
    let mut ps: PAINTSTRUCT = std::mem::zeroed();
    let hdc = BeginPaint(hwnd, &mut ps);
    let mut client: RECT = std::mem::zeroed();
    GetClientRect(hwnd, &mut client);
    STATE.with(|s| {
        let s = s.borrow();
        match s.as_ref().and_then(|st| st.frames.get(st.index)) {
            Some(frame) => {
                let mut info: BITMAPINFO = std::mem::zeroed();
                info.bmiHeader = BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: SIDE,
                    biHeight: -SIDE, // top-down
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB,
                    biSizeImage: 0,
                    biXPelsPerMeter: 0,
                    biYPelsPerMeter: 0,
                    biClrUsed: 0,
                    biClrImportant: 0,
                };
                SetStretchBltMode(hdc, COLORONCOLOR as i32); // hard pixel edges, like the real screen
                StretchDIBits(
                    hdc,
                    0,
                    0,
                    client.right,
                    client.bottom,
                    0,
                    0,
                    SIDE,
                    SIDE,
                    frame.as_ptr().cast(),
                    &info,
                    DIB_RGB_COLORS,
                    SRCCOPY,
                );
            }
            None => {
                let brush = CreateSolidBrush(u32::from(BG.0) | (u32::from(BG.1) << 8) | (u32::from(BG.2) << 16));
                FillRect(hdc, &client, brush);
                DeleteObject(brush);
                SetBkMode(hdc, TRANSPARENT as i32);
                SetTextColor(hdc, 0x00BC_C2C7); // the muted grey (BGR)
                let mut text = wide("Esperando datos...");
                let mut area = client;
                DrawTextW(hdc, text.as_mut_ptr(), -1, &mut area, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
            }
        }
    });
    EndPaint(hwnd, &ps);
}

unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        WM_TIMER => {
            tick(hwnd);
            0
        }
        WM_PAINT => {
            paint(hwnd);
            0
        }
        WM_ERASEBKGND => 1, // (everything is painted)
        WM_LBUTTONUP => {
            click();
            0
        }
        WM_APP => {
            match wparam {
                SHOW => bring_to_front(hwnd),
                CLOSE => {
                    DestroyWindow(hwnd);
                }
                CLICK => click(),
                _ => {}
            }
            0
        }
        WM_CLOSE => {
            DestroyWindow(hwnd);
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

/// A click in the window: what the app does for it (off this thread: it may wait on the device).
fn click() {
    let on_click = STATE.with(|s| s.borrow().as_ref().map(|st| st.shared.on_click.clone()));
    if let Some(on_click) = on_click {
        in_background(move || on_click());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::render::canvas::Canvas;
    use crate::adapters::render::gif::encode_gif;
    use crate::util::LockExt;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    fn gif() -> Arc<Vec<u8>> {
        let frames: Vec<Canvas> = [BG, crate::core::themes::Rgb(250, 84, 7)].iter().map(|c| Canvas::new(240, 240, *c)).collect();
        Arc::new(encode_gif(&frames).unwrap())
    }

    fn wait_for(what: &str, cond: impl Fn() -> bool) {
        let started = Instant::now();
        while !cond() {
            assert!(started.elapsed() < Duration::from_secs(10), "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(30));
        }
    }

    /// Opens a real (briefly visible) window: run with `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn it_opens_plays_forwards_a_click_and_closes() {
        let current: Arc<Mutex<Option<Latest>>> = Arc::new(Mutex::new(None));
        let (c, titles, clicks) = (current.clone(), Arc::new(Mutex::new(0usize)), Arc::new(Mutex::new(0usize)));
        let (t, k) = (titles.clone(), clicks.clone());
        let window = SimulatorWindow::new(
            Arc::new(move || c.lk().clone()),
            Arc::new(move || *k.lk() += 1),
            Arc::new(move || {
                *t.lk() += 1;
                "GeekMagic (simulada)".into()
            }),
        );
        assert!(!window.is_open());
        window.toggle();
        assert!(window.is_open());
        wait_for("the window", || window.hwnd() != 0);
        wait_for("it to tick with nothing to show", || *titles.lk() > 3);
        *current.lk() = Some(("claude".into(), 1, gif()));
        window.click();
        wait_for("the click to reach the app", || *clicks.lk() == 1);
        window.show(); // (to the front, no second window)
        window.toggle();
        wait_for("the window to close", || !window.is_open());
        assert_eq!(window.hwnd(), 0);
        window.show();
        wait_for("it to open again", || window.hwnd() != 0);
        window.close();
        wait_for("it to close again", || !window.is_open());
    }
}
