use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use instances::{InstanceRecord, InstanceStore, ServerSyncScope};

use crate::nbt::{self, NbtFile, Tag, TagList};
use crate::{SyncOptions, sync_all};

struct Fixture {
    root: PathBuf,
    store: InstanceStore,
}

impl Fixture {
    fn new(name: &str, versions: &[(&str, &str)]) -> Self {
        let root = std::env::temp_dir().join(format!(
            "vertex-instance-sync-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let mut store = InstanceStore::default();
        for (id, version) in versions {
            fs::create_dir_all(root.join(id)).unwrap();
            store.instances.push(InstanceRecord {
                id: (*id).to_owned(),
                minecraft_root: (*id).to_owned(),
                game_version: (*version).to_owned(),
                ..InstanceRecord::default()
            });
        }
        Self { root, store }
    }

    fn path(&self, id: &str, file: &str) -> PathBuf {
        self.root.join(id).join(file)
    }

    fn run(&self, options: SyncOptions, skip: &[&str]) -> crate::SyncReport {
        let skip: HashSet<String> = skip.iter().map(|id| (*id).to_owned()).collect();
        sync_all(
            &self.store,
            &self.root,
            &self.root.join("_shared"),
            options,
            &skip,
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn servers_file(entries: &[(&str, &str)]) -> Vec<u8> {
    let items = entries
        .iter()
        .map(|(name, ip)| {
            Tag::Compound(vec![
                ("name".to_owned(), Tag::String(name.as_bytes().to_vec())),
                ("ip".to_owned(), Tag::String(ip.as_bytes().to_vec())),
                ("hidden".to_owned(), Tag::Byte(0)),
            ])
        })
        .collect();
    nbt::write(&NbtFile {
        name: String::new(),
        root: vec![(
            "servers".to_owned(),
            Tag::List(TagList {
                element_id: 10,
                items,
            }),
        )],
    })
}

fn server_ips(path: &PathBuf) -> Vec<String> {
    let Ok(bytes) = fs::read(path) else {
        return Vec::new();
    };
    let file = nbt::parse(&bytes).unwrap();
    let Some(Tag::List(list)) = nbt::get(&file.root, "servers") else {
        return Vec::new();
    };
    list.items
        .iter()
        .filter_map(|entry| match entry {
            Tag::Compound(c) => nbt::get(c, "ip").and_then(Tag::as_str),
            _ => None,
        })
        .collect()
}

const SERVERS_ONLY: SyncOptions = SyncOptions {
    servers: true,
    command_history: false,
    hotbars: false,
    servers_default_all_instances: false,
};

#[test]
fn server_scopes_control_which_instances_receive_a_server() {
    let mut fx = Fixture::new("scopes", &[("a", "1.21"), ("b", "1.21"), ("c", "1.21")]);
    fs::write(
        fx.path("a", "servers.dat"),
        servers_file(&[
            ("Everywhere", "all.example.net"),
            ("Private", "solo.example.net"),
            ("Some", "pick.example.net"),
            ("Default", "default.example.net"),
        ]),
    )
    .unwrap();
    fx.store
        .set_server_sync_scope("ALL.example.net ", ServerSyncScope::AllInstances);
    fx.store
        .set_server_sync_scope("solo.example.net", ServerSyncScope::ThisInstanceOnly);
    fx.store.set_server_sync_scope(
        "pick.example.net",
        ServerSyncScope::Selected(BTreeSet::from(["c".to_owned()])),
    );

    let report = fx.run(SERVERS_ONLY, &[]);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(
        server_ips(&fx.path("b", "servers.dat")),
        ["all.example.net"]
    );
    let mut c = server_ips(&fx.path("c", "servers.dat"));
    c.sort();
    assert_eq!(c, ["all.example.net", "pick.example.net"]);
    // Nothing is removed from the source instance.
    assert_eq!(server_ips(&fx.path("a", "servers.dat")).len(), 4);

    // Second pass is a no-op, and flipping the default adds the unscoped server.
    assert!(!fx.run(SERVERS_ONLY, &[]).changed_anything());
    let report = fx.run(
        SyncOptions {
            servers_default_all_instances: true,
            ..SERVERS_ONLY
        },
        &[],
    );
    assert!(report.changed_anything());
    assert!(server_ips(&fx.path("b", "servers.dat")).contains(&"default.example.net".to_owned()));
    assert!(!server_ips(&fx.path("b", "servers.dat")).contains(&"solo.example.net".to_owned()));
}

#[test]
fn running_instances_are_not_written() {
    let mut fx = Fixture::new("skip", &[("a", "1.21"), ("b", "1.21")]);
    fs::write(
        fx.path("a", "servers.dat"),
        servers_file(&[("S", "s.example.net")]),
    )
    .unwrap();
    fx.store
        .set_server_sync_scope("s.example.net", ServerSyncScope::AllInstances);
    fx.run(SERVERS_ONLY, &["b"]);
    assert!(!fx.path("b", "servers.dat").exists());
}

#[test]
fn command_history_merges_and_caps_at_fifty() {
    let fx = Fixture::new("history", &[("a", "1.21"), ("b", "1.21")]);
    let older: Vec<String> = (0..40).map(|i| format!("say old {i}")).collect();
    fs::write(fx.path("a", "command_history.txt"), older.join("\n")).unwrap();
    let newer: Vec<String> = (0..30)
        .map(|i| format!("say new {i}"))
        .chain(["say old 3".to_owned()])
        .collect();
    let newer_path = fx.path("b", "command_history.txt");
    fs::write(&newer_path, newer.join("\n")).unwrap();
    fs::File::options()
        .write(true)
        .open(&newer_path)
        .unwrap()
        .set_modified(SystemTime::now() + Duration::from_secs(60))
        .unwrap();

    let options = SyncOptions {
        command_history: true,
        ..SyncOptions::default()
    };
    fx.run(options, &[]);
    let a = fs::read_to_string(fx.path("a", "command_history.txt")).unwrap();
    assert_eq!(a, fs::read_to_string(&newer_path).unwrap());
    let lines: Vec<&str> = a.lines().collect();
    assert_eq!(lines.len(), 50);
    assert_eq!(*lines.last().unwrap(), "say old 3");
    assert_eq!(lines.iter().filter(|l| **l == "say old 3").count(), 1);
    assert!(!fx.run(options, &[]).changed_anything());
}

#[test]
fn hotbars_only_flow_to_same_or_newer_versions() {
    let fx = Fixture::new(
        "hotbars",
        &[("old", "1.20.1"), ("new", "1.21.4"), ("other", "1.21.4")],
    );
    fs::write(fx.path("old", "hotbar.nbt"), b"old-bar").unwrap();
    let options = SyncOptions {
        hotbars: true,
        ..SyncOptions::default()
    };
    fx.run(options, &[]);
    assert_eq!(fs::read(fx.path("new", "hotbar.nbt")).unwrap(), b"old-bar");
    assert_eq!(
        fs::read(fx.path("other", "hotbar.nbt")).unwrap(),
        b"old-bar"
    );

    // A newer save in a newer instance reaches its peer but never the older version.
    let newer_path = fx.path("new", "hotbar.nbt");
    fs::write(&newer_path, b"new-bar").unwrap();
    fs::File::options()
        .write(true)
        .open(&newer_path)
        .unwrap()
        .set_modified(SystemTime::now() + Duration::from_secs(60))
        .unwrap();
    fx.run(options, &[]);
    assert_eq!(
        fs::read(fx.path("other", "hotbar.nbt")).unwrap(),
        b"new-bar"
    );
    assert_eq!(fs::read(fx.path("old", "hotbar.nbt")).unwrap(), b"old-bar");
    assert!(!fx.run(options, &[]).changed_anything());
}

mod world_tests {
    use super::*;
    use crate::{WorldPaths, disable_world_sync, enable_world_sync, set_world_members};
    use instances::{SyncedWorld, WorldLinkMode};

    fn make_world(fx: &Fixture, id: &str, folder: &str, contents: &str) {
        let dir = fx.path(id, "saves").join(folder);
        fs::create_dir_all(dir.join("region")).unwrap();
        fs::write(dir.join("level.dat"), contents).unwrap();
        fs::write(dir.join("region").join("r.0.0.mca"), contents).unwrap();
    }

    fn paths(fx: &Fixture) -> (PathBuf, PathBuf) {
        (fx.root.clone(), fx.root.join("_shared"))
    }

    fn ids(list: &[&str]) -> BTreeSet<String> {
        list.iter().map(|id| (*id).to_owned()).collect()
    }

    #[cfg(unix)]
    #[test]
    fn enabling_moves_the_world_and_links_every_member() {
        let mut fx = Fixture::new(
            "world-enable",
            &[("a", "1.21"), ("b", "1.21"), ("c", "1.21")],
        );
        make_world(&fx, "a", "Survival", "data");
        make_world(&fx, "c", "Survival", "conflicting");
        let (root, shared) = paths(&fx);
        let outcome = enable_world_sync(
            &mut fx.store,
            &WorldPaths {
                installations_root: &root,
                shared_root: &shared,
            },
            "a",
            "Survival",
            &ids(&["b", "c"]),
            &HashSet::new(),
        )
        .unwrap();

        let world = &fx.store.synced_worlds[0];
        assert_eq!(world.members.keys().collect::<Vec<_>>(), ["a", "b"]);
        assert!(world.members.values().all(|m| *m == WorldLinkMode::Symlink));
        assert_eq!(
            outcome.warnings.len(),
            1,
            "c has a conflicting world: {outcome:?}"
        );
        // Data lives once, in the shared folder, and both members see the same files.
        assert!(shared.join(&world.id).join("level.dat").is_file());
        assert!(fs::read_link(fx.path("a", "saves").join("Survival")).is_ok());
        fs::write(
            fx.path("b", "saves").join("Survival").join("level.dat"),
            "edited in b",
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(fx.path("a", "saves").join("Survival").join("level.dat")).unwrap(),
            "edited in b"
        );
        // The conflicting world in c was left alone.
        assert_eq!(
            fs::read_to_string(fx.path("c", "saves").join("Survival").join("level.dat")).unwrap(),
            "conflicting"
        );
    }

    #[cfg(unix)]
    #[test]
    fn membership_changes_and_disabling_keep_real_copies() {
        let mut fx = Fixture::new(
            "world-members",
            &[("a", "1.21"), ("b", "1.21"), ("c", "1.21")],
        );
        make_world(&fx, "a", "W", "v1");
        let (root, shared) = paths(&fx);
        let wp = WorldPaths {
            installations_root: &root,
            shared_root: &shared,
        };
        enable_world_sync(&mut fx.store, &wp, "a", "W", &ids(&["b"]), &HashSet::new()).unwrap();
        let id = fx.store.synced_worlds[0].id.clone();

        set_world_members(&mut fx.store, &wp, &id, &ids(&["a", "c"]), &HashSet::new()).unwrap();
        // b was detached with its own copy; c now shares.
        let b = fx.path("b", "saves").join("W");
        assert!(b.is_dir() && fs::read_link(&b).is_err());
        assert_eq!(fs::read_to_string(b.join("level.dat")).unwrap(), "v1");
        assert!(fs::read_link(fx.path("c", "saves").join("W")).is_ok());

        disable_world_sync(&mut fx.store, &wp, &id, &HashSet::new()).unwrap();
        assert!(fx.store.synced_worlds.is_empty());
        assert!(!shared.join(&id).exists());
        for instance in ["a", "c"] {
            let dir = fx.path(instance, "saves").join("W");
            assert!(fs::read_link(&dir).is_err());
            assert_eq!(fs::read_to_string(dir.join("level.dat")).unwrap(), "v1");
        }
    }

    #[test]
    fn running_instances_block_world_changes() {
        let mut fx = Fixture::new("world-running", &[("a", "1.21"), ("b", "1.21")]);
        make_world(&fx, "a", "W", "v1");
        let (root, shared) = paths(&fx);
        let running = HashSet::from(["b".to_owned()]);
        let err = enable_world_sync(
            &mut fx.store,
            &WorldPaths {
                installations_root: &root,
                shared_root: &shared,
            },
            "a",
            "W",
            &ids(&["b"]),
            &running,
        )
        .unwrap_err();
        assert!(err.contains("Close"), "{err}");
        assert!(fx.path("a", "saves").join("W").join("level.dat").is_file());
    }

    #[test]
    fn mirrored_members_follow_the_newest_save() {
        let mut fx = Fixture::new("world-mirror", &[("a", "1.21"), ("b", "1.21")]);
        let shared = fx.root.join("_shared").join("w-1");
        fs::create_dir_all(&shared).unwrap();
        fs::write(shared.join("level.dat"), "old").unwrap();
        make_world(&fx, "a", "W", "old");
        make_world(&fx, "b", "W", "old");
        fx.store.synced_worlds.push(SyncedWorld {
            id: "w-1".to_owned(),
            folder_name: "W".to_owned(),
            members: [
                ("a".to_owned(), WorldLinkMode::Mirror),
                ("b".to_owned(), WorldLinkMode::Mirror),
            ]
            .into(),
        });
        // `a` saved most recently.
        let a_level = fx.path("a", "saves").join("W").join("level.dat");
        fs::write(&a_level, "newest").unwrap();
        fs::File::options()
            .write(true)
            .open(&a_level)
            .unwrap()
            .set_modified(SystemTime::now() + Duration::from_secs(120))
            .unwrap();

        let report = fx.run(SyncOptions::default(), &[]);
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert_eq!(
            fs::read_to_string(shared.join("level.dat")).unwrap(),
            "newest"
        );
        assert_eq!(
            fs::read_to_string(fx.path("b", "saves").join("W").join("level.dat")).unwrap(),
            "newest"
        );
        assert!(!fx.run(SyncOptions::default(), &[]).changed_anything());
    }
}

mod settings_tests {
    use super::*;

    fn set_modified(path: &PathBuf, offset_secs: u64) {
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(SystemTime::now() + Duration::from_secs(offset_secs))
            .unwrap();
    }

    #[test]
    fn options_sync_translates_between_versions_and_settles() {
        let mut fx = Fixture::new(
            "settings",
            &[("new", "1.21.4"), ("old", "1.12.2"), ("solo", "1.21.4")],
        );
        for id in ["new", "old"] {
            fx.store.game_settings_sync.instances.insert(id.to_owned());
        }
        let new_path = fx.path("new", "options.txt");
        let old_path = fx.path("old", "options.txt");
        let solo_path = fx.path("solo", "options.txt");
        fs::write(&new_path, "ao:true\nrenderDistance:16\nfov:0.5\nfullscreen:true\nkey_key.forward:key.keyboard.up\n").unwrap();
        fs::write(
            &old_path,
            "ao:0\nrenderDistance:3\nfov:0.0\nfullscreen:false\nkey_key.forward:17\nmodded:keep\n",
        )
        .unwrap();
        fs::write(&solo_path, "fov:0.0\n").unwrap();
        set_modified(&old_path, 0);
        set_modified(&new_path, 30);

        let report = fx.run(SyncOptions::default(), &[]);
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        let old = fs::read_to_string(&old_path).unwrap();
        assert!(
            old.contains("ao:2\n") && old.contains("renderDistance:16\n"),
            "{old}"
        );
        assert!(
            old.contains("fov:0.5\n") && old.contains("key_key.forward:200\n"),
            "{old}"
        );
        // Machine-specific and unknown keys are untouched, and non-participants are ignored.
        assert!(
            old.contains("fullscreen:false\n") && old.contains("modded:keep\n"),
            "{old}"
        );
        assert_eq!(fs::read_to_string(&solo_path).unwrap(), "fov:0.0\n");
        // The translated copy doesn't echo back, and a second pass changes nothing.
        assert!(!fx.run(SyncOptions::default(), &[]).changed_anything());
        assert!(
            fs::read_to_string(&new_path)
                .unwrap()
                .contains("renderDistance:16\n")
        );
    }

    #[test]
    fn options_sync_skips_running_instances_and_respects_categories() {
        let mut fx = Fixture::new("settings-skip", &[("a", "1.21.4"), ("b", "1.21.4")]);
        for id in ["a", "b"] {
            fx.store.game_settings_sync.instances.insert(id.to_owned());
        }
        fx.store.game_settings_sync.categories.remove("video");
        let a = fx.path("a", "options.txt");
        let b = fx.path("b", "options.txt");
        fs::write(&a, "fov:0.5\nchatScale:0.5\n").unwrap();
        fs::write(&b, "fov:0.0\nchatScale:1.0\n").unwrap();
        set_modified(&b, 0);
        set_modified(&a, 30);
        fx.run(SyncOptions::default(), &["b"]);
        assert_eq!(fs::read_to_string(&b).unwrap(), "fov:0.0\nchatScale:1.0\n");
        fx.run(SyncOptions::default(), &[]);
        assert_eq!(fs::read_to_string(&b).unwrap(), "fov:0.0\nchatScale:0.5\n");
    }
}

mod pack_tests {
    use super::*;
    use crate::{PackStatus, pack_status, scan_packs};
    use instances::PackOverride;
    use std::io::Write;

    fn write_pack(dir: &PathBuf, name: &str, mcmeta: &str) {
        fs::create_dir_all(dir).unwrap();
        let file = fs::File::create(dir.join(name)).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file("pack.mcmeta", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(mcmeta.as_bytes()).unwrap();
        zip.finish().unwrap();
    }

    fn setup(name: &str) -> Fixture {
        let mut fx = Fixture::new(
            name,
            &[("new", "1.21.4"), ("modern", "26.3"), ("old", "1.12.2")],
        );
        for id in ["new", "modern", "old"] {
            fx.store.game_settings_sync.instances.insert(id.to_owned());
        }
        fx.store.game_settings_sync.resource_packs = true;
        fx
    }

    #[test]
    fn packs_are_shared_only_where_metadata_or_override_allows() {
        let mut fx = setup("packs-share");
        let packs = fx.path("modern", "resourcepacks");
        // Works for 1.21.4 (46) and 26.3 (97.1) but not 1.12.2 (3).
        write_pack(
            &packs,
            "Wide.zip",
            r#"{"pack":{"pack_format":34,"min_format":34,"max_format":120}}"#,
        );
        // Only for 26.x.
        write_pack(
            &packs,
            "Modern.zip",
            r#"{"pack":{"min_format":84,"max_format":120}}"#,
        );
        // No metadata at all.
        write_pack(&packs, "Mystery.zip", r#"{"nothing":true}"#);
        fx.store
            .set_pack_override("Modern.zip", "new", Some(PackOverride::Allow));
        fx.store
            .set_pack_override("Wide.zip", "old", Some(PackOverride::Allow));
        fx.store
            .set_pack_override("Wide.zip", "new", Some(PackOverride::Deny));

        let report = fx.run(SyncOptions::default(), &[]);
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        let names = |id: &str| {
            let mut names: Vec<String> = scan_packs(&fx.root.join(id))
                .into_iter()
                .map(|p| p.name)
                .collect();
            names.sort();
            names
        };
        assert_eq!(
            names("new"),
            ["Modern.zip"],
            "Deny beats metadata; Allow beats metadata"
        );
        assert_eq!(
            names("old"),
            ["Wide.zip"],
            "an override vouches for an incompatible pack"
        );
        assert_eq!(names("modern").len(), 3);
        assert!(!fx.run(SyncOptions::default(), &[]).changed_anything());
    }

    #[test]
    fn enabled_state_follows_the_newest_options_for_shared_packs_only() {
        let fx = setup("packs-enabled");
        let wide = r#"{"pack":{"pack_format":34,"max_format":120,"min_format":34}}"#;
        for id in ["new", "modern"] {
            write_pack(&fx.path(id, "resourcepacks"), "Wide.zip", wide);
            write_pack(&fx.path(id, "resourcepacks"), "Local.zip", wide);
        }
        let new_options = fx.path("new", "options.txt");
        let modern_options = fx.path("modern", "options.txt");
        fs::write(
            &new_options,
            "resourcePacks:[\"vanilla\",\"file/Local.zip\"]\n",
        )
        .unwrap();
        fs::write(
            &modern_options,
            "resourcePacks:[\"vanilla\",\"mod_resources\",\"file/Wide.zip\"]\n",
        )
        .unwrap();
        let file = fs::File::options()
            .write(true)
            .open(&modern_options)
            .unwrap();
        file.set_modified(SystemTime::now() + Duration::from_secs(30))
            .unwrap();

        fx.run(SyncOptions::default(), &[]);
        let updated = fs::read_to_string(&new_options).unwrap();
        // Wide.zip gets enabled and Local.zip (disabled in the newest options) gets disabled;
        // built-in and mod entries in the modern file are irrelevant to the old one.
        assert_eq!(updated, "resourcePacks:[\"vanilla\",\"file/Wide.zip\"]\n");
        assert!(!fx.run(SyncOptions::default(), &[]).changed_anything());
    }

    #[test]
    fn status_reports_metadata_and_overrides() {
        let mut store = InstanceStore::default();
        let support = mc_options::packs::PackSupport::parse("65..120");
        assert_eq!(
            pack_status(&store, "a", "26.3", "P.zip", support),
            PackStatus::Compatible
        );
        assert_eq!(
            pack_status(&store, "a", "1.21.4", "P.zip", support),
            PackStatus::Incompatible
        );
        assert_eq!(
            pack_status(&store, "a", "1.21.4", "P.zip", None),
            PackStatus::Unknown
        );
        store.set_pack_override("P.zip", "a", Some(PackOverride::Allow));
        assert_eq!(
            pack_status(&store, "a", "1.21.4", "P.zip", support),
            PackStatus::ForcedAllow
        );
        store.set_pack_override("P.zip", "a", None);
        assert!(store.resource_pack_overrides.is_empty());
    }
}

mod manage_tests {
    use super::*;
    use crate::{ManageRequest, manage};

    fn ids(list: &[&str]) -> BTreeSet<String> {
        list.iter().map(|id| (*id).to_owned()).collect()
    }

    #[test]
    fn deleting_a_server_keeps_the_next_sync_from_putting_it_back() {
        let mut fx = Fixture::new("manage-delete-server", &[("a", "1.21"), ("b", "1.21")]);
        fs::write(fx.path("a", "servers.dat"), servers_file(&[("S", "play.example.com")])).unwrap();
        fs::write(fx.path("b", "servers.dat"), servers_file(&[("S", "play.example.com")])).unwrap();
        fx.store.set_server_sync_scope("play.example.com", ServerSyncScope::AllInstances);
        let root = fx.root.clone();
        manage(
            &mut fx.store,
            &root,
            &HashSet::new(),
            false,
            ManageRequest::DeleteServer { instance_ids: ids(&["a"]), address: "play.example.com".into() },
        )
        .unwrap();
        assert!(server_ips(&fx.path("a", "servers.dat")).is_empty());
        fx.run(SyncOptions { servers: true, ..SyncOptions::default() }, &[]);
        assert!(server_ips(&fx.path("a", "servers.dat")).is_empty());
        assert_eq!(server_ips(&fx.path("b", "servers.dat")), vec!["play.example.com"]);
    }

    #[test]
    fn moving_a_server_edits_both_lists_and_follows_it_in_the_scope() {
        let mut fx = Fixture::new("manage-move-server", &[("a", "1.21"), ("b", "1.21")]);
        fs::write(fx.path("a", "servers.dat"), servers_file(&[("S", "play.example.com")])).unwrap();
        let root = fx.root.clone();
        manage(
            &mut fx.store,
            &root,
            &HashSet::new(),
            false,
            ManageRequest::TransferServer {
                source_id: "a".into(),
                address: "play.example.com".into(),
                dest_id: "b".into(),
                remove_source: true,
            },
        )
        .unwrap();
        assert!(server_ips(&fx.path("a", "servers.dat")).is_empty());
        assert_eq!(server_ips(&fx.path("b", "servers.dat")), vec!["play.example.com"]);
    }

    #[test]
    fn editing_a_server_renames_it_and_moves_its_scope_to_the_new_address() {
        let mut fx = Fixture::new("manage-edit-server", &[("a", "1.21")]);
        fs::write(fx.path("a", "servers.dat"), servers_file(&[("Old", "old.example.com")])).unwrap();
        fx.store.set_server_sync_scope("old.example.com", ServerSyncScope::AllInstances);
        let root = fx.root.clone();
        manage(
            &mut fx.store,
            &root,
            &HashSet::new(),
            false,
            ManageRequest::EditServer {
                instance_ids: ids(&["a"]),
                address: "old.example.com".into(),
                new_name: "New".into(),
                new_address: "new.example.com".into(),
            },
        )
        .unwrap();
        assert_eq!(server_ips(&fx.path("a", "servers.dat")), vec!["new.example.com"]);
        assert_eq!(fx.store.server_sync_scope("new.example.com"), ServerSyncScope::AllInstances);
        assert_eq!(fx.store.server_sync_scope("old.example.com"), ServerSyncScope::FollowDefault);
    }

    #[test]
    fn a_running_instance_is_left_alone() {
        let mut fx = Fixture::new("manage-running", &[("a", "1.21")]);
        fs::write(fx.path("a", "servers.dat"), servers_file(&[("S", "x.example.com")])).unwrap();
        let root = fx.root.clone();
        let running = HashSet::from(["a".to_owned()]);
        let result = manage(
            &mut fx.store,
            &root,
            &running,
            false,
            ManageRequest::DeleteServer { instance_ids: ids(&["a"]), address: "x.example.com".into() },
        );
        assert!(result.is_err());
        assert_eq!(server_ips(&fx.path("a", "servers.dat")), vec!["x.example.com"]);
    }

    #[test]
    fn a_world_is_renamed_copied_and_deleted() {
        let mut fx = Fixture::new("manage-world", &[("a", "1.21"), ("b", "1.21")]);
        let world = fx.path("a", "saves/World");
        fs::create_dir_all(&world).unwrap();
        let level = NbtFile {
            name: String::new(),
            root: vec![("Data".to_owned(), Tag::Compound(vec![("LevelName".to_owned(), Tag::String(b"World".to_vec()))]))],
        };
        fs::write(world.join("level.dat"), nbt::write_gzip(&level).unwrap()).unwrap();
        let root = fx.root.clone();
        manage(
            &mut fx.store,
            &root,
            &HashSet::new(),
            false,
            ManageRequest::RenameWorld { instance_ids: ids(&["a"]), folder: "World".into(), new_name: "Renamed".into() },
        )
        .unwrap();
        let parsed = nbt::parse(&fs::read(world.join("level.dat")).unwrap()).unwrap();
        let Some(Tag::Compound(data)) = nbt::get(&parsed.root, "Data") else { panic!("no Data") };
        assert_eq!(nbt::get(data, "LevelName").and_then(Tag::as_str).as_deref(), Some("Renamed"));

        manage(
            &mut fx.store,
            &root,
            &HashSet::new(),
            false,
            ManageRequest::TransferWorld { source_id: "a".into(), folder: "World".into(), dest_id: "b".into(), remove_source: false },
        )
        .unwrap();
        assert!(fx.path("b", "saves/World/level.dat").is_file());
        manage(
            &mut fx.store,
            &root,
            &HashSet::new(),
            false,
            ManageRequest::DeleteWorld { instance_ids: ids(&["a"]), folder: "World".into() },
        )
        .unwrap();
        assert!(!world.exists());
        assert!(fx.path("b", "saves/World").is_dir());
    }
}
