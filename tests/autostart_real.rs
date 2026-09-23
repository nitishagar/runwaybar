//! Autostart install/uninstall against an isolated XDG_CONFIG_HOME.
//! Own file: owns its env.

use std::os::unix::fs::PermissionsExt;

#[test]
fn install_writes_desktop_entry_uninstall_removes() {
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_CONFIG_HOME", tmp.path());
    let path = runwaybar::autostart::install().expect("install");
    assert!(
        path.exists(),
        "autostart file created at {}",
        path.display()
    );
    let body = std::fs::read_to_string(&path).unwrap();
    assert!(body.contains("[Desktop Entry]"), "body: {body}");
    assert!(body.contains("Exec="), "body: {body}");
    assert!(body.contains("serve"), "body: {body}");
    assert!(body.contains("Name=RunwayBar"), "body: {body}");
    // no temp leftovers
    let dir = tmp.path().join("autostart");
    let leftovers: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().contains("tmp"))
        .collect();
    assert!(leftovers.is_empty(), "temp files: {leftovers:?}");
    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o644, "desktop file mode {mode:o}");

    let removed = runwaybar::autostart::uninstall().expect("uninstall");
    assert_eq!(removed, path);
    assert!(!path.exists(), "file removed");
    // idempotent uninstall
    runwaybar::autostart::uninstall().expect("uninstall is idempotent");
}
