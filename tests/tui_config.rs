use std::path::PathBuf;
use std::sync::Mutex;

use pairproof::tui::config::{
    delete_api_key, env_override_active, mask_api_key, read_api_key_input, resolve_api_key,
    save_api_key, stored_api_key, ApiKeyConfig, ConfigError,
};

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn unique_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pairproof-config-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn key_path(name: &str) -> PathBuf {
    unique_dir(name).join("pairproof").join("api_key")
}

fn cleanup(path: &std::path::Path) {
    if let Some(root) = path.ancestors().nth(2) {
        let _ = std::fs::remove_dir_all(root);
    }
}

fn clear_env() {
    std::env::remove_var("MODEL_API_KEY");
}

#[test]
fn stored_api_key_file_is_private() {
    let _guard = ENV_LOCK.lock().unwrap();
    clear_env();
    let path = key_path("private");
    save_api_key(&path, "  sk-test-key-12345678  ").unwrap();
    assert_eq!(
        stored_api_key(&path).unwrap(),
        Some("sk-test-key-12345678".to_string())
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "sk-test-key-12345678"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let file_mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(file_mode, 0o600);
        let parent_mode = std::fs::metadata(path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(parent_mode, 0o700);
    }
    cleanup(&path);
}

#[test]
fn environment_key_overrides_stored_key() {
    let _guard = ENV_LOCK.lock().unwrap();
    clear_env();
    let path = key_path("env-override");
    save_api_key(&path, "stored-key-12345678").unwrap();
    std::env::set_var("MODEL_API_KEY", "  env-key-87654321  ");
    assert!(env_override_active());
    assert_eq!(resolve_api_key(&path).unwrap(), "env-key-87654321");
    assert_eq!(
        stored_api_key(&path).unwrap(),
        Some("stored-key-12345678".to_string())
    );
    std::env::remove_var("MODEL_API_KEY");
    assert!(!env_override_active());
    assert_eq!(resolve_api_key(&path).unwrap(), "stored-key-12345678");
    cleanup(&path);
}

#[test]
fn api_key_never_appears_in_debug_or_rendered_output() {
    let _guard = ENV_LOCK.lock().unwrap();
    clear_env();
    let secret = "sk-super-secret-value-9999";
    let path = key_path("no-leak");
    let config = ApiKeyConfig::new(path.clone());
    let debug = format!("{config:?}");
    assert!(!debug.contains(secret));
    let masked = mask_api_key(secret);
    assert!(!masked.contains(secret));
    assert!(!masked.contains("sk-super-secret"));
    let errors = [
        format!("{:?}", ConfigError::MissingKey),
        format!("{:?}", ConfigError::EmptyKey),
        format!("{:?}", ConfigError::Symlink("x".to_string())),
        format!("{:?}", ConfigError::Io("x".to_string())),
    ];
    for text in errors {
        assert!(!text.contains(secret));
    }
    save_api_key(&path, secret).unwrap();
    // Debug of the resolved value is never produced by the API; mask is safe.
    assert_eq!(mask_api_key(secret), "\u{2022}\u{2022}\u{2022}\u{2022}9999");
    cleanup(&path);
}

#[test]
fn mask_api_key_edge_cases() {
    assert_eq!(mask_api_key(""), "********");
    assert_eq!(mask_api_key("short"), "********");
    assert_eq!(mask_api_key("1234567"), "********");
    assert_eq!(
        mask_api_key("12345678"),
        "\u{2022}\u{2022}\u{2022}\u{2022}5678"
    );
    assert_eq!(
        mask_api_key("  abcdefgh  "),
        "\u{2022}\u{2022}\u{2022}\u{2022}efgh"
    );
    // Multibyte characters must not panic on slicing.
    let masked = mask_api_key("\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}");
    assert!(masked.ends_with("\u{e9}\u{e9}\u{e9}\u{e9}"));
    assert!(!mask_api_key("a-very-long-secret-key").contains("a-very-long-secret-key"));
}

#[test]
fn save_delete_and_replacement() {
    let _guard = ENV_LOCK.lock().unwrap();
    clear_env();
    let path = key_path("lifecycle");
    assert!(matches!(
        resolve_api_key(&path),
        Err(ConfigError::MissingKey)
    ));
    assert_eq!(stored_api_key(&path).unwrap(), None);
    delete_api_key(&path).unwrap();
    save_api_key(&path, "first-key-11111111").unwrap();
    assert_eq!(resolve_api_key(&path).unwrap(), "first-key-11111111");
    save_api_key(&path, "second-key-22222222").unwrap();
    assert_eq!(resolve_api_key(&path).unwrap(), "second-key-22222222");
    delete_api_key(&path).unwrap();
    assert_eq!(stored_api_key(&path).unwrap(), None);
    delete_api_key(&path).unwrap();
    assert!(matches!(
        save_api_key(&path, "   "),
        Err(ConfigError::EmptyKey)
    ));
    cleanup(&path);
}

#[test]
fn blank_and_missing_stored_key() {
    let _guard = ENV_LOCK.lock().unwrap();
    clear_env();
    let path = key_path("blank");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "   \n").unwrap();
    assert_eq!(stored_api_key(&path).unwrap(), None);
    assert!(matches!(
        resolve_api_key(&path),
        Err(ConfigError::MissingKey)
    ));
    // Blank env does not override a valid stored key.
    save_api_key(&path, "stored-key-12345678").unwrap();
    std::env::set_var("MODEL_API_KEY", "   ");
    assert!(!env_override_active());
    assert_eq!(resolve_api_key(&path).unwrap(), "stored-key-12345678");
    std::env::remove_var("MODEL_API_KEY");
    cleanup(&path);
}

#[test]
fn env_override_wins_when_stored_is_unusable() {
    let _guard = ENV_LOCK.lock().unwrap();
    clear_env();
    let path = key_path("env-wins");
    std::env::set_var("MODEL_API_KEY", "env-only-key-12345");
    // Stored path does not even exist; env still wins.
    assert_eq!(resolve_api_key(&path).unwrap(), "env-only-key-12345");
    std::env::remove_var("MODEL_API_KEY");
    cleanup(&path);
}

#[test]
fn read_api_key_input_trims_first_line() {
    assert_eq!(read_api_key_input(b"  my-key-123  \r\n"), "my-key-123");
    assert_eq!(read_api_key_input(b"only-key"), "only-key");
    assert_eq!(read_api_key_input(b""), "");
    assert_eq!(read_api_key_input(b"  \nignored"), "");
}

#[cfg(unix)]
#[test]
fn symlink_file_is_rejected() {
    let _guard = ENV_LOCK.lock().unwrap();
    clear_env();
    let path = key_path("symlink-file");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let target = path.parent().unwrap().join("real.txt");
    std::fs::write(&target, "real-key-12345678").unwrap();
    std::os::unix::fs::symlink(&target, &path).unwrap();
    assert!(matches!(
        stored_api_key(&path),
        Err(ConfigError::Symlink(_))
    ));
    assert!(matches!(
        save_api_key(&path, "new-key-12345678"),
        Err(ConfigError::Symlink(_))
    ));
    assert!(matches!(
        delete_api_key(&path),
        Err(ConfigError::Symlink(_))
    ));
    // The symlink target is untouched.
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "real-key-12345678"
    );
    cleanup(&path);
}

#[cfg(unix)]
#[test]
fn symlink_ancestor_is_rejected() {
    let _guard = ENV_LOCK.lock().unwrap();
    clear_env();
    let root = unique_dir("symlink-ancestor");
    let _ = std::fs::remove_dir_all(&root);
    let real = root.join("real");
    std::fs::create_dir_all(&real).unwrap();
    let link = root.join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let deep = link.join("sub").join("api_key");
    let shallow = link.join("api_key");
    for path in [&deep, &shallow] {
        assert!(
            matches!(
                save_api_key(path, "some-key-12345678"),
                Err(ConfigError::Symlink(_))
            ),
            "deep symlink should be rejected: {}",
            path.display()
        );
        assert!(matches!(stored_api_key(path), Err(ConfigError::Symlink(_))));
        assert!(matches!(
            resolve_api_key(path),
            Err(ConfigError::Symlink(_))
        ));
        assert!(matches!(delete_api_key(path), Err(ConfigError::Symlink(_))));
    }
    assert!(!real.join("sub").join("api_key").exists());
    assert!(!real.join("api_key").exists());
    assert!(!real.join("sub").exists());
    let legit = real.join("sub").join("api_key");
    save_api_key(&legit, "legit-key-12345678").unwrap();
    assert_eq!(
        stored_api_key(&legit).unwrap(),
        Some("legit-key-12345678".to_string())
    );
    assert_eq!(resolve_api_key(&legit).unwrap(), "legit-key-12345678");
    delete_api_key(&legit).unwrap();
    assert_eq!(stored_api_key(&legit).unwrap(), None);
    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn symlink_parent_is_rejected() {
    let _guard = ENV_LOCK.lock().unwrap();
    clear_env();
    let root = unique_dir("symlink-parent");
    let real_dir = root.join("real");
    std::fs::create_dir_all(&real_dir).unwrap();
    let link_dir = root.join("link");
    std::os::unix::fs::symlink(&real_dir, &link_dir).unwrap();
    let path = link_dir.join("api_key");
    assert!(matches!(
        save_api_key(&path, "some-key-12345678"),
        Err(ConfigError::Symlink(_))
    ));
    assert!(matches!(
        stored_api_key(&path),
        Err(ConfigError::Symlink(_))
    ));
    assert!(matches!(
        delete_api_key(&path),
        Err(ConfigError::Symlink(_))
    ));
    let _ = std::fs::remove_dir_all(&root);
}
