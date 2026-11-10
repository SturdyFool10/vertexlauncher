use vertex_i18n::tr;
use super::*;



pub(super) fn status_text(
    status: PackStatus,
    support: Option<PackSupport>,
    version: Option<Ver>,
) -> String {
    let format = version.and_then(mc_options::packs::resource_format_for);
    let format_note = format.map(|f| {
        tr!("Minecraft {version} uses format {major}.{minor}", version = version.map_or(String::new(), |v| format!("{}.{}", v.0, v.1)), major = f.major, minor = f.minor).into_owned()
    });
    match status {
        PackStatus::Compatible => tr!("Compatible with this version").into_owned(),
        PackStatus::Incompatible => tr!("Not declared for this version{version}", version = format_note.map(|n| format!(" ({n})")).unwrap_or_default()).into_owned(),
        PackStatus::Unknown => {
            if support.is_none() {
                tr!("No readable pack.mcmeta; compatibility unknown").into_owned()
            } else {
                tr!("Compatibility unknown for this version").into_owned()
            }
        }
        PackStatus::ForcedAllow => tr!("Marked as working here").into_owned(),
        PackStatus::ForcedDeny => tr!("Marked as not working here").into_owned(),
    }
}


