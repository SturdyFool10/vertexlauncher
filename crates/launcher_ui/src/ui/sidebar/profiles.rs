use std::{
    collections::{HashMap, HashSet, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
    sync::{Arc, Mutex, OnceLock, mpsc},
};

use egui::{Button, Ui};

use crate::{
    app::tokio_runtime,
    assets,
    ui::{components::icon_button, context_menu, instance_context_menu, style},
};

use super::{ProfileShortcut, SidebarOutput};

/// Renders the instance shortcut list and emits click or context-menu actions.
pub fn render(
    ui: &mut Ui,
    profile_shortcuts: &[ProfileShortcut],
    output: &mut SidebarOutput,
    max_icon_width: f32,
) {
    if profile_shortcuts.is_empty() {
        return;
    }

    poll_thumbnail_results();
    let row_height = max_icon_width.max(1.0);
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = style::SPACE_SM;

        for profile in profile_shortcuts {
            let icon_id = format!("user_profile_{}", profile.id);
            let context_id =
                ui.make_persistent_id(("sidebar_instance_context", profile.id.as_str()));
            let response = ui
                .allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), row_height),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        let thumbnail = profile
                            .thumbnail_path
                            .as_deref()
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                            .and_then(|path| {
                                let key = thumbnail_cache_key(profile.id.as_str(), path);
                                match thumbnail_cache().lock() {
                                    Ok(cache) => cache.get(key.as_str()).cloned().flatten(),
                                    Err(_) => None,
                                }
                            });
                        if thumbnail.is_none()
                            && let Some(path) = profile
                                .thumbnail_path
                                .as_deref()
                                .map(str::trim)
                                .filter(|value| !value.is_empty())
                        {
                            request_thumbnail(profile.id.as_str(), path.to_owned());
                        }
                        render_profile_icon(
                            ui,
                            icon_id.as_str(),
                            profile.name.as_str(),
                            max_icon_width,
                            thumbnail.as_deref(),
                        )
                    },
                )
                .inner;

            if response.clicked() {
                output.selected_profile_id = Some(profile.id.clone());
            }

            if response.secondary_clicked() {
                let anchor = response
                    .interact_pointer_pos()
                    .or_else(|| ui.ctx().pointer_latest_pos())
                    .unwrap_or(response.rect.left_bottom());
                instance_context_menu::request_for_instance(ui.ctx(), context_id, anchor, true);
            }

            if let Some(action) = instance_context_menu::take(ui.ctx(), context_id) {
                output
                    .instance_context_actions
                    .push((profile.id.clone(), action));
            }
        }
    });

    context_menu::show(ui.ctx());
}

fn render_profile_icon(
    ui: &mut Ui,
    icon_id: &str,
    tooltip: &str,
    max_icon_width: f32,
    thumbnail_bytes: Option<&[u8]>,
) -> egui::Response {
    if let Some(bytes) = thumbnail_bytes {
        let button_size = ui.available_width().min(max_icon_width).max(1.0);
        let icon_size = (button_size - 8.0).clamp(10.0, button_size);
        let mut hasher = DefaultHasher::new();
        icon_id.hash(&mut hasher);
        bytes.hash(&mut hasher);
        let image = egui::Image::from_bytes(
            format!("bytes://sidebar/profile-thumb/{}", hasher.finish()),
            bytes.to_vec(),
        )
        .fit_to_exact_size(egui::vec2(icon_size, icon_size));
        return ui.add_sized(
            [button_size, button_size],
            Button::image(image)
                .frame(true)
                .corner_radius(egui::CornerRadius::same(10))
                .stroke(egui::Stroke::new(
                    1.0,
                    ui.visuals().widgets.inactive.bg_stroke.color,
                ))
                .fill(ui.visuals().widgets.inactive.weak_bg_fill),
        );
    }

    icon_button::svg(
        ui,
        icon_id,
        assets::LIBRARY_SVG,
        tooltip,
        false,
        max_icon_width,
    )
}

fn thumbnail_cache() -> &'static Mutex<HashMap<String, Option<Arc<[u8]>>>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<Arc<[u8]>>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn thumbnail_in_flight() -> &'static Mutex<HashSet<String>> {
    static IN_FLIGHT: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    IN_FLIGHT.get_or_init(|| Mutex::new(HashSet::new()))
}

fn thumbnail_results_channel() -> &'static (
    mpsc::Sender<(String, Option<Arc<[u8]>>)>,
    Arc<Mutex<mpsc::Receiver<(String, Option<Arc<[u8]>>)>>>,
) {
    static CHANNEL: OnceLock<(
        mpsc::Sender<(String, Option<Arc<[u8]>>)>,
        Arc<Mutex<mpsc::Receiver<(String, Option<Arc<[u8]>>)>>>,
    )> = OnceLock::new();
    CHANNEL.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<(String, Option<Arc<[u8]>>)>();
        (tx, Arc::new(Mutex::new(rx)))
    })
}

fn thumbnail_cache_key(instance_id: &str, path: &str) -> String {
    format!("{instance_id}\n{path}")
}

fn request_thumbnail(instance_id: &str, path: String) {
    let key = thumbnail_cache_key(instance_id, path.as_str());
    if let Ok(cache) = thumbnail_cache().lock()
        && cache.contains_key(key.as_str())
    {
        return;
    }
    if let Ok(mut in_flight) = thumbnail_in_flight().lock() {
        if in_flight.contains(key.as_str()) {
            return;
        }
        in_flight.insert(key.clone());
    }
    let tx = thumbnail_results_channel().0.clone();
    let _ = tokio_runtime::spawn_detached(async move {
        let bytes = std::fs::read(path.as_str())
            .ok()
            .map(|bytes| Arc::<[u8]>::from(bytes.into_boxed_slice()));
        let _ = tx.send((key, bytes));
    });
}

fn poll_thumbnail_results() {
    let rx = thumbnail_results_channel().1.clone();
    let Ok(receiver) = rx.lock() else {
        return;
    };
    let mut updates = Vec::new();
    while let Ok(update) = receiver.try_recv() {
        updates.push(update);
    }
    if updates.is_empty() {
        return;
    }
    if let Ok(mut cache) = thumbnail_cache().lock()
        && let Ok(mut in_flight) = thumbnail_in_flight().lock()
    {
        for (key, bytes) in updates {
            in_flight.remove(key.as_str());
            cache.insert(key, bytes);
        }
    }
}
