#![cfg_attr(target_os = "macos", allow(unexpected_cfgs))]

use eframe::egui;
use image::ImageFormat;
use std::sync::Arc;

const APP_ICON_WEBP: &[u8] = include_bytes!("../../../../Vertex.webp");

pub(crate) fn egui_icon() -> Option<Arc<egui::IconData>> {
    let decoded = image::load_from_memory_with_format(APP_ICON_WEBP, ImageFormat::WebP)
        .ok()?
        .into_rgba8();
    let (width, height) = decoded.dimensions();

    Some(Arc::new(egui::IconData {
        rgba: decoded.into_raw(),
        width,
        height,
    }))
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
pub(crate) fn tao_icon() -> Option<crate::app::webview_runtime::tao::window::Icon> {
    let decoded = image::load_from_memory_with_format(APP_ICON_WEBP, ImageFormat::WebP)
        .ok()?
        .into_rgba8();
    let (width, height) = decoded.dimensions();
    crate::app::webview_runtime::tao::window::Icon::from_rgba(decoded.into_raw(), width, height)
        .ok()
}

#[cfg(target_os = "macos")]
pub(crate) fn apply_macos_dock_icon() {
    if let Err(err) = apply_macos_dock_icon_inner() {
        tracing::error!(
            target: "vertexlauncher/platform/app_icon",
            error = %err,
            "macOS dock icon integration failed; leaving the default dock icon in place"
        );
    }
}

#[cfg(target_os = "macos")]
fn apply_macos_dock_icon_inner() -> Result<(), String> {
    use image::codecs::png::PngEncoder;
    use image::{ColorType, ImageEncoder};
    use objc::rc::autoreleasepool;
    use objc::runtime::{Class, Object};
    use objc::{msg_send, sel, sel_impl};
    use std::ffi::c_char;
    use std::path::PathBuf;

    unsafe fn nsstring_from_path(path: &PathBuf) -> *mut Object {
        let Some(ns_string_class) = Class::get("NSString") else {
            return std::ptr::null_mut();
        };
        let path_str = path.to_string_lossy();
        let bytes = path_str.as_bytes();
        let obj: *mut Object = msg_send![ns_string_class, alloc];
        let obj: *mut Object = msg_send![
            obj,
            initWithBytes: bytes.as_ptr().cast::<c_char>()
            length: bytes.len()
            encoding: 4usize
        ];
        obj
    }

    let decoded = match image::load_from_memory_with_format(APP_ICON_WEBP, ImageFormat::WebP) {
        Ok(value) => value.into_rgba8(),
        Err(err) => return Err(format!("failed to decode bundled app icon: {err}")),
    };

    let mut png_bytes = Vec::new();
    {
        let encoder = PngEncoder::new(&mut png_bytes);
        if encoder
            .write_image(
                decoded.as_raw(),
                decoded.width(),
                decoded.height(),
                ColorType::Rgba8.into(),
            )
            .is_err()
        {
            return Err("failed to encode app icon as PNG".to_owned());
        }
    }

    let icon_path = std::env::temp_dir().join("vertexlauncher-dock-icon.png");
    std::fs::write(&icon_path, png_bytes)
        .map_err(|err| format!("failed to write temporary dock icon: {err}"))?;

    autoreleasepool(|| unsafe {
        let ns_string = nsstring_from_path(&icon_path);
        if ns_string.is_null() {
            return Err("failed to create NSString for dock icon path".to_owned());
        }
        let Some(ns_image_class) = Class::get("NSImage") else {
            return Err("NSImage class unavailable".to_owned());
        };
        let Some(ns_app_class) = Class::get("NSApplication") else {
            return Err("NSApplication class unavailable".to_owned());
        };

        let image: *mut Object = msg_send![ns_image_class, alloc];
        let image: *mut Object = msg_send![image, initWithContentsOfFile: ns_string];
        if image.is_null() {
            return Err("failed to initialize NSImage from dock icon path".to_owned());
        }

        let app: *mut Object = msg_send![ns_app_class, sharedApplication];
        if app.is_null() {
            return Err("shared NSApplication instance unavailable".to_owned());
        }
        let _: () = msg_send![app, setApplicationIconImage: image];
        Ok::<_, String>(())
    })?;
    Ok(())
}
