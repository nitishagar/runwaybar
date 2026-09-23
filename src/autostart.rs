//! User-level autostart via the XDG autostart spec (`~/.config/autostart/*.desktop`).

use std::fs;
use std::io::Write;
use std::path::PathBuf;

pub const DESKTOP_ID: &str = "in.applair.RunwayBar.desktop";

pub fn autostart_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("autostart").join(DESKTOP_ID))
}

pub fn install() -> Result<PathBuf, String> {
    let path = autostart_path().ok_or("no user config dir")?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let body = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=RunwayBar\n\
         Comment=Your AI runway, at a glance\n\
         Exec={} serve\n\
         Icon=runwaybar\n\
         Terminal=false\n\
         X-GNOME-Autostart-enabled=true\n\
         X-GNOME-UsesNotifications=true\n\
         Categories=Utility;\n",
        exe.display()
    );
    let tmp = path.with_extension("desktop.tmp");
    {
        let mut f = fs::File::create(&tmp).map_err(|e| e.to_string())?;
        f.write_all(body.as_bytes()).map_err(|e| e.to_string())?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o644)).map_err(|e| e.to_string())?;
    }
    fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    Ok(path)
}

pub fn uninstall() -> Result<PathBuf, String> {
    let path = autostart_path().ok_or("no user config dir")?;
    match fs::remove_file(&path) {
        Ok(()) => Ok(path),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(path),
        Err(e) => Err(e.to_string()),
    }
}
