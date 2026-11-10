use vertex_i18n::tr;
use super::*;



pub(super) fn open_instance_folder(
    state: &mut InstanceScreenState,
    instances: &InstanceStore,
    config: &Config,
    instance_id: &str,
) {
    if let Some(instance) = instances.find(instance_id) {
        let installations_root = config.minecraft_installations_root_path().to_path_buf();
        let instance_root = instances::instance_root_path(&installations_root, instance);
        match desktop::open_in_file_manager(instance_root.as_path()) {
            Ok(()) => {
                state.status_message = Some(tr!("Opened instance folder: {instance_root}", instance_root = instance_root.display()).into_owned());
            }
            Err(err) => {
                state.status_message = Some(tr!("Failed to open instance folder: {err}", err = err).into_owned());
            }
        }
    } else {
        state.status_message = Some(tr!("Instance was removed before opening its folder.").into_owned());
    }
}

pub(super) fn prepare_move_instance_modal(
    state: &mut InstanceScreenState,
    instances: &InstanceStore,
    config: &Config,
    instance_id: &str,
) {
    if let Some(instance) = instances.find(instance_id) {
        let installations_root = config.minecraft_installations_root_path().to_path_buf();
        let current_root = instances::instance_root_path(&installations_root, instance);
        state.move_instance_dest_input = current_root.display().to_string();
    } else {
        state.move_instance_dest_input = String::new();
    }
    state.move_instance_dest_valid = false;
    state.move_instance_dest_error = None;
    state.show_move_instance_modal = true;
}
