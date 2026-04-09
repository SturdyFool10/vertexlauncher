use super::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_windows_cli_paths() {
        assert_eq!(
            normalize_windows_cli_path(r"\\?\C:\Users\clove\AppData\Local\vertexlauncher"),
            r"C:\Users\clove\AppData\Local\vertexlauncher"
        );
        assert_eq!(
            normalize_windows_cli_path(r"\\?\UNC\server\share\vertexlauncher"),
            r"\\server\share\vertexlauncher"
        );
        assert_eq!(
            normalize_windows_cli_path(r"C:\Users\clove\AppData\Local\vertexlauncher"),
            r"C:\Users\clove\AppData\Local\vertexlauncher"
        );
    }

    #[test]
    fn parses_loader_matrix_entries_from_array() {
        let payload = serde_json::json!([
            {
                "loader": { "version": "0.16.5" },
                "intermediary": { "version": "1.21.1" }
            },
            {
                "loader": { "version": "0.16.4" },
                "intermediary": { "version": "1.21.1" }
            }
        ]);

        let catalog = parse_loader_version_matrix(&payload);
        let versions = catalog
            .versions_by_game_version
            .get("1.21.1")
            .expect("expected versions for 1.21.1");
        assert!(versions.iter().any(|entry| entry == "0.16.5"));
        assert!(versions.iter().any(|entry| entry == "0.16.4"));
    }

    #[test]
    fn parses_loader_matrix_entries_from_loader_wrapped_object() {
        let payload = serde_json::json!({
            "loader": [
                {
                    "loader": { "version": "0.1.2" },
                    "intermediary": { "version": "1.20.6" }
                }
            ]
        });

        let catalog = parse_loader_version_matrix(&payload);
        let versions = catalog
            .versions_by_game_version
            .get("1.20.6")
            .expect("expected versions for 1.20.6");
        assert_eq!(versions, &vec!["0.1.2".to_owned()]);
    }

    #[test]
    fn parses_global_loader_versions_when_matrix_has_no_game_mapping() {
        let payload = serde_json::json!([
            {
                "loader": { "version": "0.16.9" }
            },
            {
                "loader": { "version": "0.16.10" }
            }
        ]);

        let versions = parse_global_loader_versions(&payload);
        assert_eq!(versions, vec!["0.16.10".to_owned(), "0.16.9".to_owned()]);
    }

    #[test]
    fn sorts_loader_versions_descending() {
        let versions = sort_loader_versions_desc(vec![
            "21.0.1-beta".to_owned(),
            "21.0.10".to_owned(),
            "21.0.2".to_owned(),
        ]);

        assert_eq!(
            versions,
            vec![
                "21.0.10".to_owned(),
                "21.0.2".to_owned(),
                "21.0.1-beta".to_owned(),
            ]
        );
    }

    #[test]
    fn url_encoding_covers_spaces_and_symbols() {
        assert_eq!(
            url_encode_component("1.14 Pre-Release 5"),
            "1.14%20Pre-Release%205"
        );
        assert_eq!(url_encode_component("a/b"), "a%2Fb");
    }

    #[test]
    fn eta_tracks_progress_fraction_deltas() {
        let mut state = ProgressEtaState::default();

        assert_eq!(
            state.observe(ProgressEtaPoint {
                fraction: 0.25,
                at_millis: 1_000,
            }),
            None
        );
        assert_eq!(
            state.observe(ProgressEtaPoint {
                fraction: 0.50,
                at_millis: 6_000,
            }),
            Some(10)
        );
    }

    #[test]
    fn eta_resets_when_progress_fraction_regresses() {
        let mut state = ProgressEtaState::default();
        let _ = state.observe(ProgressEtaPoint {
            fraction: 0.75,
            at_millis: 2_000,
        });
        let _ = state.observe(ProgressEtaPoint {
            fraction: 0.90,
            at_millis: 5_000,
        });

        assert_eq!(
            state.observe(ProgressEtaPoint {
                fraction: 0.40,
                at_millis: 6_000,
            }),
            None
        );
    }

    #[test]
    fn eta_reuses_last_estimate_when_fraction_does_not_move() {
        let mut state = ProgressEtaState::default();
        let _ = state.observe(ProgressEtaPoint {
            fraction: 0.10,
            at_millis: 1_000,
        });
        assert_eq!(
            state.observe(ProgressEtaPoint {
                fraction: 0.30,
                at_millis: 3_000,
            }),
            Some(7)
        );
        assert_eq!(
            state.observe(ProgressEtaPoint {
                fraction: 0.30,
                at_millis: 3_500,
            }),
            Some(7)
        );
    }

    #[test]
    fn quick_play_requested_singleplayer_is_appended_when_profile_has_no_flags() {
        let mut substitutions = HashMap::new();
        substitutions.insert("quickPlayPath".to_owned(), "/tmp/qp".to_owned());
        substitutions.insert("quickPlaySingleplayer".to_owned(), "MyWorld".to_owned());
        let context = LaunchContext {
            substitutions,
            features: HashMap::new(),
        };
        let args = vec!["--demo".to_owned()];
        let normalized = normalize_quick_play_arguments(args, &context);
        assert_eq!(
            normalized,
            vec![
                "--demo".to_owned(),
                "--quickPlayPath".to_owned(),
                "/tmp/qp".to_owned(),
                "--quickPlaySingleplayer".to_owned(),
                "MyWorld".to_owned(),
            ]
        );
    }

    #[test]
    fn quick_play_keeps_single_mode_and_filters_duplicates() {
        let mut substitutions = HashMap::new();
        substitutions.insert("quickPlayPath".to_owned(), "/tmp/qp".to_owned());
        substitutions.insert("quickPlayMultiplayer".to_owned(), "example.org".to_owned());
        let context = LaunchContext {
            substitutions,
            features: HashMap::new(),
        };
        let args = vec![
            "--quickPlayPath".to_owned(),
            "/tmp/qp".to_owned(),
            "--quickPlaySingleplayer".to_owned(),
            "WorldA".to_owned(),
            "--quickPlayMultiplayer".to_owned(),
            "example.org".to_owned(),
        ];
        let normalized = normalize_quick_play_arguments(args, &context);
        assert_eq!(
            normalized,
            vec![
                "--quickPlayPath".to_owned(),
                "/tmp/qp".to_owned(),
                "--quickPlaySingleplayer".to_owned(),
                "WorldA".to_owned(),
            ]
        );
    }
}
