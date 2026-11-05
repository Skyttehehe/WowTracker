//! Grabs the top-left corner of the game window from the screen, where the
//! addon draws its live strip. Windows only: it copies the pixels from the
//! desktop with GDI, so the corner must be visible (windowed or "windowed
//! fullscreen" mode, not covered by another window).

/// Pixels grabbed: wide and tall enough for the strip at the largest square size.
pub const GRAB_WIDTH: usize = crate::strip::CELLS * crate::strip::MAX_CELL + 8;
pub const GRAB_HEIGHT: usize = crate::strip::MAX_CELL + 2;

/// A grabbed image: 32-bit BGRA, rows top-down.
pub struct Image {
    pub pixels: Vec<u8>,
    pub width: usize,
    pub height: usize,
}

#[derive(Debug, PartialEq)]
pub enum GrabError {
    /// No game window found.
    NoWindow,
    /// The game window is minimized.
    Minimized,
    Failed(String),
    Unsupported,
}

impl std::fmt::Display for GrabError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GrabError::NoWindow => write!(f, "no World of Warcraft window"),
            GrabError::Minimized => write!(f, "the game window is minimized"),
            GrabError::Failed(e) => write!(f, "screen grab failed: {e}"),
            GrabError::Unsupported => write!(f, "reading the strip is only supported on Windows"),
        }
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::ptr::null_mut;
    use windows_sys::Win32::Foundation::{HWND, POINT, RECT};
    use windows_sys::Win32::Graphics::Gdi::{
        BitBlt, ClientToScreen, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits,
        ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, SRCCOPY,
    };
    use windows_sys::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
    use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, GetClientRect, IsIconic};

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub struct Grabber {
        title: Vec<u16>,
    }

    impl Grabber {
        pub fn new() -> Grabber {
            // Without this, Windows scales coordinates on high-DPI screens and
            // the grab would miss the strip.
            unsafe {
                SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            }
            Grabber { title: wide("World of Warcraft") }
        }

        fn window(&self) -> Result<HWND, GrabError> {
            let hwnd = unsafe { FindWindowW(std::ptr::null(), self.title.as_ptr()) };
            if hwnd.is_null() {
                return Err(GrabError::NoWindow);
            }
            if unsafe { IsIconic(hwnd) } != 0 {
                return Err(GrabError::Minimized);
            }
            Ok(hwnd)
        }

        pub fn grab(&mut self) -> Result<Image, GrabError> {
            let hwnd = self.window()?;
            let (w, h) = (GRAB_WIDTH as i32, GRAB_HEIGHT as i32);
            unsafe {
                let mut rect = RECT { left: 0, top: 0, right: 0, bottom: 0 };
                if GetClientRect(hwnd, &mut rect) == 0 || rect.right < w || rect.bottom < h {
                    return Err(GrabError::Failed("game window too small".into()));
                }
                let mut origin = POINT { x: 0, y: 0 };
                if ClientToScreen(hwnd, &mut origin) == 0 {
                    return Err(GrabError::Failed("ClientToScreen".into()));
                }
                let screen = GetDC(null_mut());
                if screen.is_null() {
                    return Err(GrabError::Failed("GetDC".into()));
                }
                let mem = CreateCompatibleDC(screen);
                let bmp = CreateCompatibleBitmap(screen, w, h);
                let old = SelectObject(mem, bmp);
                let copied = BitBlt(mem, 0, 0, w, h, screen, origin.x, origin.y, SRCCOPY);

                let mut info: BITMAPINFO = std::mem::zeroed();
                info.bmiHeader = BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h, // negative: rows top-down
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB,
                    ..std::mem::zeroed()
                };
                let mut pixels = vec![0u8; GRAB_WIDTH * GRAB_HEIGHT * 4];
                SelectObject(mem, old);
                let lines = GetDIBits(mem, bmp, 0, h as u32, pixels.as_mut_ptr().cast(), &mut info, DIB_RGB_COLORS);
                DeleteObject(bmp);
                DeleteDC(mem);
                ReleaseDC(null_mut(), screen);
                if copied == 0 || lines != h {
                    return Err(GrabError::Failed("BitBlt/GetDIBits".into()));
                }
                Ok(Image { pixels, width: GRAB_WIDTH, height: GRAB_HEIGHT })
            }
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;

    pub struct Grabber;

    impl Grabber {
        pub fn new() -> Grabber {
            Grabber
        }

        pub fn grab(&mut self) -> Result<Image, GrabError> {
            Err(GrabError::Unsupported)
        }
    }
}

pub use imp::Grabber;

impl Default for Grabber {
    fn default() -> Self {
        Grabber::new()
    }
}
