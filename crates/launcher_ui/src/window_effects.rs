#![cfg_attr(target_os = "macos", allow(unexpected_cfgs))]
use eframe::CreationContext;

/// Returns whether the current target should opt into native blur effects.
///
/// macOS is intentionally excluded for now because the current AppKit-based
/// implementation is not stable enough to make it part of the default launch path.
pub const fn platform_supports_blur() -> bool {
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    {
        true
    }

    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        false
    }
}

/// Returns whether native blur needs an alpha-capable transparent viewport.
///
/// On Windows, DWM backdrop attributes do not require requesting a transparent
/// swapchain from wgpu. On Linux compositors, transparency is typically needed
/// for blur regions to be visible.
pub const fn blur_requires_transparent_viewport() -> bool {
    #[cfg(target_os = "linux")]
    {
        true
    }

    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

/// Applies platform-specific window blur/backdrop effects when enabled.
pub fn apply(
    cc: &CreationContext<'_>,
    blur_enabled: bool,
    windows_backdrop_type: config::WindowsBackdropType,
) -> Result<(), String> {
    if !blur_enabled || !platform_supports_blur() {
        return Ok(());
    }

    apply_impl(cc, windows_backdrop_type)
}

fn apply_impl(
    cc: &CreationContext<'_>,
    windows_backdrop_type: config::WindowsBackdropType,
) -> Result<(), String> {
    #[cfg(not(target_os = "windows"))]
    let _ = windows_backdrop_type;

    #[cfg(target_os = "windows")]
    return windows::apply(cc, windows_backdrop_type);
    #[cfg(target_os = "linux")]
    return linux::apply(cc);
    #[cfg(target_os = "macos")]
    return macos::apply(cc);

    #[allow(unreachable_code)]
    Err("window blur is not supported on this platform".to_owned())
}

#[cfg(target_os = "windows")]
mod windows {
    use config::WindowsBackdropType;
    use core::ffi::c_void;
    use core::mem::size_of;
    use eframe::CreationContext;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::Graphics::Dwm::{
        DWM_BB_ENABLE, DWM_BLURBEHIND, DwmEnableBlurBehindWindow, DwmSetWindowAttribute,
    };

    const DWMWA_SYSTEMBACKDROP_TYPE: i32 = 38;
    const DWMWA_REDIRECTIONBITMAP_ALPHA: i32 = 39;
    const DWMWA_USE_HOSTBACKDROPBRUSH: i32 = 17;
    const DWMWA_MICA_EFFECT: i32 = 1029;
    const DWMSBT_AUTO: i32 = 0;
    const DWMSBT_MAINWINDOW: i32 = 2;
    const DWMSBT_TRANSIENTWINDOW: i32 = 3;
    const DWMSBT_TABBEDWINDOW: i32 = 4;
    const WCA_ACCENT_POLICY: i32 = 19;
    const ACCENT_FLAG_DRAW_ALL_BORDERS: u32 = 0x20 | 0x40 | 0x80 | 0x100;
    const ACCENT_ENABLE_BLURBEHIND: i32 = 3;
    const ACCENT_ENABLE_ACRYLICBLURBEHIND: i32 = 4;
    const ACCENT_ENABLE_HOSTBACKDROP: i32 = 5;

    #[repr(C)]
    struct AccentPolicy {
        accent_state: i32,
        accent_flags: u32,
        gradient_color: u32,
        animation_id: u32,
    }

    #[repr(C)]
    struct WindowCompositionAttribData {
        attrib: i32,
        pv_data: *mut c_void,
        cb_data: usize,
    }

    unsafe extern "system" {
        fn GetModuleHandleA(module_name: *const u8) -> isize;
        fn GetProcAddress(module: isize, proc_name: *const u8) -> *const c_void;
    }

    pub fn apply(
        cc: &CreationContext<'_>,
        windows_backdrop_type: WindowsBackdropType,
    ) -> Result<(), String> {
        let window_handle = cc
            .window_handle()
            .map_err(|error| format!("window handle unavailable: {error}"))?;
        let RawWindowHandle::Win32(handle) = window_handle.as_raw() else {
            return Err("unsupported window handle for Windows blur".to_owned());
        };
        let hwnd: HWND = handle.hwnd.get() as HWND;
        let _ = set_bool_window_attribute(hwnd, DWMWA_USE_HOSTBACKDROPBRUSH, true);
        let _ = set_bool_window_attribute(hwnd, DWMWA_REDIRECTIONBITMAP_ALPHA, true);
        apply_backdrop_with_fallback(hwnd, windows_backdrop_type)
    }

    fn apply_backdrop_with_fallback(
        hwnd: HWND,
        windows_backdrop_type: WindowsBackdropType,
    ) -> Result<(), String> {
        let mut failures = Vec::new();
        let gradient_color = default_gradient_color();

        let attempts: Vec<WindowsBackdropType> =
            if matches!(windows_backdrop_type, WindowsBackdropType::Auto) {
                vec![
                    WindowsBackdropType::Acrylic,
                    WindowsBackdropType::Mica,
                    WindowsBackdropType::MicaAlt,
                    WindowsBackdropType::LegacyBlur,
                ]
            } else {
                vec![windows_backdrop_type]
            };

        for attempt in &attempts {
            if apply_specific_backdrop(hwnd, *attempt, gradient_color).is_ok() {
                tracing::info!(
                    target: "vertexlauncher/window_blur",
                    backdrop = %attempt.label(),
                    "Windows backdrop applied."
                );
                return Ok(());
            }
        }

        for attempt in attempts {
            if let Err(err) = apply_specific_backdrop(hwnd, attempt, gradient_color) {
                failures.push(format!("{}: {err}", attempt.label()));
            }
        }

        Err(format!(
            "Windows blur/backdrop APIs were rejected by this system: {}",
            failures.join(" | ")
        ))
    }

    fn apply_specific_backdrop(
        hwnd: HWND,
        backdrop_type: WindowsBackdropType,
        gradient_color: u32,
    ) -> Result<(), String> {
        match backdrop_type {
            WindowsBackdropType::Auto => {
                if set_system_backdrop(hwnd, DWMSBT_AUTO).is_ok() {
                    Ok(())
                } else {
                    Err("DWMSBT_AUTO was rejected".to_owned())
                }
            }
            WindowsBackdropType::Mica => {
                if set_system_backdrop(hwnd, DWMSBT_MAINWINDOW).is_ok()
                    || set_mica_effect(hwnd).is_ok()
                {
                    Ok(())
                } else {
                    Err("Mica APIs were rejected".to_owned())
                }
            }
            WindowsBackdropType::Acrylic => {
                if set_system_backdrop(hwnd, DWMSBT_TRANSIENTWINDOW).is_ok()
                    || set_window_accent(
                        hwnd,
                        ACCENT_ENABLE_ACRYLICBLURBEHIND,
                        ACCENT_FLAG_DRAW_ALL_BORDERS,
                        gradient_color,
                    )
                    .is_ok()
                    || set_window_accent(
                        hwnd,
                        ACCENT_ENABLE_HOSTBACKDROP,
                        ACCENT_FLAG_DRAW_ALL_BORDERS,
                        gradient_color,
                    )
                    .is_ok()
                {
                    Ok(())
                } else {
                    Err("Acrylic APIs were rejected".to_owned())
                }
            }
            WindowsBackdropType::MicaAlt => {
                if set_system_backdrop(hwnd, DWMSBT_TABBEDWINDOW).is_ok() {
                    Ok(())
                } else {
                    Err("Mica Alt API was rejected".to_owned())
                }
            }
            WindowsBackdropType::LegacyBlur => {
                if set_window_accent(
                    hwnd,
                    ACCENT_ENABLE_BLURBEHIND,
                    ACCENT_FLAG_DRAW_ALL_BORDERS,
                    gradient_color,
                )
                .is_ok()
                    || set_legacy_blur(hwnd).is_ok()
                {
                    Ok(())
                } else {
                    Err("Legacy blur APIs were rejected".to_owned())
                }
            }
        }
    }

    fn set_window_accent(
        hwnd: HWND,
        accent_state: i32,
        accent_flags: u32,
        gradient_color: u32,
    ) -> Result<(), String> {
        type SetWindowCompositionAttributeFn =
            unsafe extern "system" fn(HWND, *mut WindowCompositionAttribData) -> i32;

        let user32 = unsafe { GetModuleHandleA(b"user32.dll\0".as_ptr()) };
        if user32 == 0 {
            return Err("GetModuleHandleA(user32.dll) failed".to_owned());
        }
        let proc = unsafe { GetProcAddress(user32, b"SetWindowCompositionAttribute\0".as_ptr()) };
        if proc.is_null() {
            return Err("SetWindowCompositionAttribute is unavailable".to_owned());
        }
        let set_window_composition_attribute: SetWindowCompositionAttributeFn =
            unsafe { std::mem::transmute(proc) };

        let mut policy = AccentPolicy {
            accent_state,
            accent_flags,
            gradient_color,
            animation_id: 0,
        };
        let mut data = WindowCompositionAttribData {
            attrib: WCA_ACCENT_POLICY,
            pv_data: (&mut policy as *mut AccentPolicy).cast::<c_void>(),
            cb_data: size_of::<AccentPolicy>(),
        };
        let ok = unsafe { set_window_composition_attribute(hwnd, &mut data as *mut _) };
        if ok != 0 {
            Ok(())
        } else {
            Err(format!(
                "SetWindowCompositionAttribute failed for accent_state={accent_state}"
            ))
        }
    }

    const fn default_gradient_color() -> u32 {
        0x70000000
    }

    fn set_system_backdrop(hwnd: HWND, backdrop_type: i32) -> Result<(), String> {
        let result = unsafe {
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_SYSTEMBACKDROP_TYPE as _,
                &backdrop_type as *const _ as *const _,
                size_of::<i32>() as u32,
            )
        };
        if result == 0 {
            Ok(())
        } else {
            Err(format!(
                "DWMWA_SYSTEMBACKDROP_TYPE rejected value {backdrop_type} ({result:#x})"
            ))
        }
    }

    fn set_bool_window_attribute(hwnd: HWND, attribute: i32, value: bool) -> Result<(), String> {
        let as_bool: i32 = if value { 1 } else { 0 };
        let result = unsafe {
            DwmSetWindowAttribute(
                hwnd,
                attribute as _,
                &as_bool as *const _ as *const _,
                size_of::<i32>() as u32,
            )
        };
        if result == 0 {
            Ok(())
        } else {
            Err(format!(
                "DwmSetWindowAttribute bool attr {attribute} rejected value {as_bool} ({result:#x})"
            ))
        }
    }

    fn set_mica_effect(hwnd: HWND) -> Result<(), String> {
        let enabled: i32 = 1;
        let result = unsafe {
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_MICA_EFFECT as _,
                &enabled as *const _ as *const _,
                size_of::<i32>() as u32,
            )
        };
        if result == 0 {
            Ok(())
        } else {
            Err(format!(
                "DWMWA_MICA_EFFECT attribute was rejected ({result:#x})"
            ))
        }
    }

    fn set_legacy_blur(hwnd: HWND) -> Result<(), String> {
        let blur_behind = DWM_BLURBEHIND {
            dwFlags: DWM_BB_ENABLE,
            fEnable: 1,
            hRgnBlur: 0,
            fTransitionOnMaximized: 0,
        };
        let result = unsafe { DwmEnableBlurBehindWindow(hwnd, &blur_behind) };
        if result == 0 {
            Ok(())
        } else {
            Err(format!("DwmEnableBlurBehindWindow failed ({result:#x})"))
        }
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use eframe::CreationContext;
    use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
    use std::ffi::CStr;
    use std::ffi::c_void;
    use std::os::raw::c_uchar;
    use wayland_backend::client::{Backend, ObjectId};
    use wayland_client::globals::GlobalListContents;
    use wayland_client::protocol::{wl_registry, wl_surface::WlSurface};
    use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, delegate_noop};
    use wayland_protocols_plasma::blur::client::org_kde_kwin_blur::OrgKdeKwinBlur;
    use wayland_protocols_plasma::blur::client::org_kde_kwin_blur_manager::OrgKdeKwinBlurManager;
    use x11_dl::xlib;

    static KDE_BLUR_ATOM: &CStr = c"_KDE_NET_WM_BLUR_BEHIND_REGION";
    static CARDINAL_ATOM: &CStr = c"CARDINAL";

    pub fn apply(cc: &CreationContext<'_>) -> Result<(), String> {
        let window_handle = cc
            .window_handle()
            .map_err(|error| format!("window handle unavailable: {error}"))?;
        let display_handle = cc
            .display_handle()
            .map_err(|error| format!("display handle unavailable: {error}"))?;

        let result = match (display_handle.as_raw(), window_handle.as_raw()) {
            (RawDisplayHandle::Xlib(display), RawWindowHandle::Xlib(window)) => {
                let Some(display) = display.display else {
                    return Ok(());
                };
                apply_x11(display.as_ptr().cast::<xlib::Display>(), window.window)
            }
            (RawDisplayHandle::Wayland(display), RawWindowHandle::Wayland(window)) => {
                apply_wayland(
                    display.display.as_ptr().cast::<c_void>(),
                    window.surface.as_ptr().cast::<c_void>(),
                )
            }
            _ => Ok(()),
        };

        match result {
            Ok(()) => Ok(()),
            Err(error) => {
                tracing::info!(
                    target: "vertexlauncher/window_blur",
                    error = %error,
                    "Linux compositor-specific blur hook was unavailable; keeping transparent viewport enabled for best-effort blur compatibility."
                );
                Ok(())
            }
        }
    }

    fn apply_x11(display: *mut xlib::Display, window: std::os::raw::c_ulong) -> Result<(), String> {
        let xlib =
            xlib::Xlib::open().map_err(|_| "failed to load Xlib for blur support".to_owned())?;

        let kde_blur_atom =
            unsafe { (xlib.XInternAtom)(display, KDE_BLUR_ATOM.as_ptr(), xlib::False) };
        let cardinal_atom =
            unsafe { (xlib.XInternAtom)(display, CARDINAL_ATOM.as_ptr(), xlib::False) };
        if kde_blur_atom == 0 || cardinal_atom == 0 {
            return Err("KDE X11 blur atoms are unavailable on this display".to_owned());
        }

        unsafe {
            // Empty region means "blur the whole window".
            (xlib.XChangeProperty)(
                display,
                window,
                kde_blur_atom,
                cardinal_atom,
                32,
                xlib::PropModeReplace,
                std::ptr::null::<c_uchar>(),
                0,
            );
            (xlib.XFlush)(display);
        }
        Ok(())
    }

    struct KdeBlurState;

    impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for KdeBlurState {
        fn event(
            _: &mut Self,
            _: &wl_registry::WlRegistry,
            _: wl_registry::Event,
            _: &GlobalListContents,
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    delegate_noop!(KdeBlurState: ignore OrgKdeKwinBlurManager);
    delegate_noop!(KdeBlurState: ignore OrgKdeKwinBlur);

    fn apply_wayland(display: *mut c_void, surface: *mut c_void) -> Result<(), String> {
        if display.is_null() || surface.is_null() {
            return Err("Wayland display or surface pointer unavailable".to_owned());
        }

        let backend = unsafe { Backend::from_foreign_display(display.cast()) };
        let conn = Connection::from_backend(backend);

        let surface_id = unsafe { ObjectId::from_ptr(WlSurface::interface(), surface.cast()) };
        let Ok(surface_id) = surface_id else {
            return Err("failed to resolve Wayland surface object id".to_owned());
        };
        let Ok(surface) = WlSurface::from_id(&conn, surface_id) else {
            return Err("failed to resolve Wayland surface proxy".to_owned());
        };

        let Ok((globals, mut queue)) =
            wayland_client::globals::registry_queue_init::<KdeBlurState>(&conn)
        else {
            return Err("failed to initialize Wayland registry for blur support".to_owned());
        };

        let qh = queue.handle();
        let Ok(manager) = globals.bind::<OrgKdeKwinBlurManager, _, _>(&qh, 1..=1, ()) else {
            return Err("KDE Wayland blur manager is unavailable".to_owned());
        };

        let blur = manager.create(&surface, &qh, ());
        blur.commit();

        let _ = conn.flush();
        let _ = queue.dispatch_pending(&mut KdeBlurState);
        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use eframe::CreationContext;
    use objc::runtime::Object;
    use objc::{class, msg_send, sel, sel_impl};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct NSPoint {
        x: f64,
        y: f64,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct NSSize {
        width: f64,
        height: f64,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct NSRect {
        origin: NSPoint,
        size: NSSize,
    }

    const NS_VISUAL_EFFECT_BLENDING_MODE_BEHIND_WINDOW: isize = 0;
    const NS_VISUAL_EFFECT_MATERIAL_UNDER_WINDOW_BACKGROUND: isize = 21;
    const NS_VISUAL_EFFECT_STATE_ACTIVE: isize = 1;
    const NS_VIEW_WIDTH_SIZABLE: usize = 1 << 1;
    const NS_VIEW_HEIGHT_SIZABLE: usize = 1 << 4;

    pub fn apply(cc: &CreationContext<'_>) -> Result<(), String> {
        let window_handle = cc
            .window_handle()
            .map_err(|error| format!("window handle unavailable: {error}"))?;
        let RawWindowHandle::AppKit(handle) = window_handle.as_raw() else {
            return Err("unsupported window handle for macOS blur".to_owned());
        };

        let ns_view = handle.ns_view.as_ptr().cast::<Object>();
        unsafe {
            let ns_window: *mut Object = msg_send![ns_view, window];
            if ns_window.is_null() {
                return Err("AppKit window pointer unavailable".to_owned());
            }

            let content_view: *mut Object = msg_send![ns_window, contentView];
            if content_view.is_null() {
                return Err("AppKit content view unavailable".to_owned());
            }
            let bounds: NSRect = msg_send![content_view, bounds];
            let effect_view: *mut Object = msg_send![class!(NSVisualEffectView), alloc];
            let effect_view: *mut Object = msg_send![effect_view, initWithFrame: bounds];
            if effect_view.is_null() {
                return Err("failed to allocate NSVisualEffectView".to_owned());
            }

            let _: () = msg_send![effect_view, setAutoresizingMask: (NS_VIEW_WIDTH_SIZABLE | NS_VIEW_HEIGHT_SIZABLE)];
            let _: () = msg_send![effect_view, setBlendingMode: NS_VISUAL_EFFECT_BLENDING_MODE_BEHIND_WINDOW];
            let _: () = msg_send![effect_view, setMaterial: NS_VISUAL_EFFECT_MATERIAL_UNDER_WINDOW_BACKGROUND];
            let _: () = msg_send![effect_view, setState: NS_VISUAL_EFFECT_STATE_ACTIVE];
            let _: () = msg_send![content_view, addSubview: effect_view positioned: 0isize relativeTo: std::ptr::null::<Object>()];
        }
        Ok(())
    }
}
