use std::sync::atomic::{AtomicBool, Ordering};

static TASKBAR_PROGRESS_AVAILABLE: AtomicBool = AtomicBool::new(true);

pub fn set_install_progress(frame: &eframe::Frame, progress_0_to_1: Option<f32>) {
    if !TASKBAR_PROGRESS_AVAILABLE.load(Ordering::Relaxed) {
        return;
    }

    if let Err(err) = platform::set_install_progress(frame, progress_0_to_1) {
        TASKBAR_PROGRESS_AVAILABLE.store(false, Ordering::Relaxed);
        tracing::error!(
            target: "vertexlauncher/platform/taskbar_progress",
            error = %err,
            "platform taskbar progress integration failed; disabling it for the rest of the session"
        );
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use eframe::Frame;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use std::cell::RefCell;
    use std::thread_local;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Com::{
        CLSCTX_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    };
    use windows::Win32::UI::Shell::{ITaskbarList4, TBPF_NOPROGRESS, TBPF_NORMAL, TaskbarList};

    pub fn set_install_progress(frame: &Frame, progress_0_to_1: Option<f32>) -> Result<(), String> {
        let Ok(window_handle) = frame.window_handle() else {
            return Ok(());
        };
        let RawWindowHandle::Win32(handle) = window_handle.as_raw() else {
            return Ok(());
        };
        let hwnd = HWND(handle.hwnd.get());

        TASKBAR.with(|cell| {
            let mut cached = cell.borrow_mut();
            if cached.is_none() {
                *cached = init_taskbar();
            }
            let Some(taskbar) = cached.as_ref() else {
                return;
            };

            unsafe {
                if let Some(progress) = progress_0_to_1 {
                    let value = (progress.clamp(0.0, 1.0) * 100.0).round() as u64;
                    let _ = taskbar.SetProgressState(hwnd, TBPF_NORMAL);
                    let _ = taskbar.SetProgressValue(hwnd, value, 100);
                } else {
                    let _ = taskbar.SetProgressState(hwnd, TBPF_NOPROGRESS);
                }
            }
        });
        Ok(())
    }

    thread_local! {
        static TASKBAR: RefCell<Option<ITaskbarList4>> = const { RefCell::new(None) };
    }

    fn init_taskbar() -> Option<ITaskbarList4> {
        unsafe {
            let _ = CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED);
            CoCreateInstance(&TaskbarList, None, CLSCTX_SERVER).ok()
        }
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use core::ffi::{c_char, c_int, c_void};
    use eframe::Frame;
    use std::ffi::CString;
    use std::sync::OnceLock;

    const DEFAULT_DESKTOP_ID: &str = "vertexlauncher.desktop";
    const RTLD_LAZY: c_int = 0x0001;
    const RTLD_LOCAL: c_int = 0;

    type UnityGetForDesktopId = unsafe extern "C" fn(*const c_char) -> *mut c_void;
    type UnitySetProgress = unsafe extern "C" fn(*mut c_void, f64) -> c_int;
    type UnitySetProgressVisible = unsafe extern "C" fn(*mut c_void, c_int) -> c_int;

    pub fn set_install_progress(
        _frame: &Frame,
        progress_0_to_1: Option<f32>,
    ) -> Result<(), String> {
        let Some(unity) = unity_taskbar() else {
            return Ok(());
        };
        unity.set(progress_0_to_1);
        Ok(())
    }

    struct UnityTaskbar {
        _library: *mut c_void,
        entry: *mut c_void,
        set_progress: UnitySetProgress,
        set_progress_visible: UnitySetProgressVisible,
    }

    unsafe impl Send for UnityTaskbar {}
    unsafe impl Sync for UnityTaskbar {}

    impl UnityTaskbar {
        fn set(&self, progress_0_to_1: Option<f32>) {
            unsafe {
                match progress_0_to_1 {
                    Some(value) => {
                        let normalized = value.clamp(0.0, 1.0) as f64;
                        let _ = (self.set_progress_visible)(self.entry, 1);
                        let _ = (self.set_progress)(self.entry, normalized);
                    }
                    None => {
                        let _ = (self.set_progress_visible)(self.entry, 0);
                    }
                }
            }
        }
    }

    fn unity_taskbar() -> Option<&'static UnityTaskbar> {
        static UNITY: OnceLock<Option<UnityTaskbar>> = OnceLock::new();
        UNITY.get_or_init(load_unity).as_ref()
    }

    fn load_unity() -> Option<UnityTaskbar> {
        unsafe {
            let library = open_unity_library()?;
            let get_for_desktop_id: UnityGetForDesktopId =
                load_symbol(library, b"unity_launcher_entry_get_for_desktop_id\0")?;
            let set_progress: UnitySetProgress =
                load_symbol(library, b"unity_launcher_entry_set_progress\0")?;
            let set_progress_visible: UnitySetProgressVisible =
                load_symbol(library, b"unity_launcher_entry_set_progress_visible\0")?;

            let desktop_id_raw = std::env::var("VERTEX_DESKTOP_ID")
                .unwrap_or_else(|_| DEFAULT_DESKTOP_ID.to_owned());
            let desktop_id = CString::new(desktop_id_raw).ok()?;
            let entry = get_for_desktop_id(desktop_id.as_ptr());
            if entry.is_null() {
                return None;
            }

            Some(UnityTaskbar {
                _library: library,
                entry,
                set_progress,
                set_progress_visible,
            })
        }
    }

    unsafe fn open_unity_library() -> Option<*mut c_void> {
        for candidate in [
            "libunity.so.4",
            "libunity.so.6",
            "/usr/lib/libunity.so.4",
            "/usr/lib/libunity.so.6",
            "/usr/lib/x86_64-linux-gnu/libunity.so.4",
            "/usr/lib/x86_64-linux-gnu/libunity.so.6",
        ] {
            let name = CString::new(candidate).ok()?;
            let handle = unsafe { dlopen(name.as_ptr(), RTLD_LAZY | RTLD_LOCAL) };
            if !handle.is_null() {
                return Some(handle);
            }
        }
        None
    }

    unsafe fn load_symbol<T>(library: *mut c_void, symbol: &[u8]) -> Option<T> {
        let ptr = unsafe { dlsym(library, symbol.as_ptr().cast::<c_char>()) };
        if ptr.is_null() {
            return None;
        }
        Some(unsafe { std::mem::transmute_copy::<*mut c_void, T>(&ptr) })
    }

    unsafe extern "C" {
        fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    }
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
mod platform {
    use eframe::Frame;

    pub fn set_install_progress(
        _frame: &Frame,
        _progress_0_to_1: Option<f32>,
    ) -> Result<(), String> {
        Ok(())
    }
}
