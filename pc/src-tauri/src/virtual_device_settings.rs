/*
 * [Input] Per-computer virtual device preference (`virtual-device.json` under app data):
 *         enabled flag and the web page origins allowed to connect.
 * [Output] Starts/stops the loopback virtual device listener on the shared USB manager and
 *          serves load/save commands for the settings page. Off unless the user turns it on.
 * [Pos] Settings boundary for usb_serial::virtual_device.
 * [Sync] If fields change, update `VirtualDeviceSettings.jsx` and `pc/.folder.md`.
 */

use crate::usb_serial::{virtual_device, UsbSerialManager};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const FILE_NAME: &str = "virtual-device.json";
const MAX_SETTINGS_BYTES: u64 = 64 * 1024;
const MAX_ORIGINS: usize = 32;
static STORAGE_DIR: OnceLock<PathBuf> = OnceLock::new();

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub enabled: bool,
    pub allowed_origins: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self { enabled: false, allowed_origins: virtual_device::DEFAULT_ALLOWED_ORIGINS.map(String::from).to_vec() }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsStatus {
    enabled: bool,
    allowed_origins: Vec<String>,
    default_allowed_origins: Vec<String>,
    address: String,
    #[serde(flatten)]
    link: virtual_device::Status,
}

fn validated(settings: Settings) -> Result<Settings, String> {
    let mut allowed_origins = Vec::new();
    for origin in settings.allowed_origins.iter().filter(|origin| !origin.trim().is_empty()) {
        let origin = virtual_device::normalize_origin_rule(origin)?;
        if !allowed_origins.contains(&origin) { allowed_origins.push(origin); }
    }
    if allowed_origins.len() > MAX_ORIGINS { return Err(format!("允许的来源最多 {MAX_ORIGINS} 个")); }
    Ok(Settings { enabled: settings.enabled, allowed_origins })
}

fn config(settings: &Settings) -> Option<virtual_device::Config> {
    settings.enabled.then(|| virtual_device::Config {
        port: virtual_device::DEFAULT_PORT,
        allowed_origins: settings.allowed_origins.clone(),
    })
}

fn load_path(path: &Path) -> Result<Settings, String> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Settings::default()),
        Err(_) => return Err("无法读取虚拟设备设置".into()),
        Ok(meta) if !meta.is_file() => return Err("虚拟设备设置不是普通文件".into()),
        _ => {}
    }
    let mut raw = String::new();
    fs::File::open(path).and_then(|file| file.take(MAX_SETTINGS_BYTES + 1).read_to_string(&mut raw))
        .map_err(|_| "无法读取虚拟设备设置")?;
    if raw.len() as u64 > MAX_SETTINGS_BYTES { return Err("虚拟设备设置文件过大".into()); }
    validated(serde_json::from_str(&raw).map_err(|_| "虚拟设备设置损坏，请重新保存")?)
}

fn write_path(path: &Path, settings: &Settings) -> Result<(), String> {
    let parent = path.parent().ok_or("虚拟设备设置目录无效")?;
    fs::create_dir_all(parent).map_err(|_| "无法创建虚拟设备设置目录")?;
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|_| "无法创建虚拟设备设置文件")?;
    let raw = serde_json::to_vec_pretty(settings).map_err(|_| "虚拟设备设置序列化失败")?;
    file.write_all(&raw).and_then(|_| file.as_file().sync_all()).map_err(|_| "无法写入虚拟设备设置")?;
    file.persist(path).map_err(|_| "无法保存虚拟设备设置")?;
    Ok(())
}

fn settings_path() -> Result<PathBuf, String> {
    STORAGE_DIR.get().map(|dir| dir.join(FILE_NAME)).ok_or_else(|| "虚拟设备设置尚未初始化".into())
}

fn status(settings: Settings, usb: &UsbSerialManager) -> SettingsStatus {
    SettingsStatus {
        enabled: settings.enabled,
        allowed_origins: settings.allowed_origins,
        default_allowed_origins: Settings::default().allowed_origins,
        address: format!("ws://127.0.0.1:{}{}", virtual_device::DEFAULT_PORT, virtual_device::PATH),
        link: usb.virtual_device_status(),
    }
}

/// App startup: remember the storage directory and start listening only if the user enabled it.
pub fn configure(dir: PathBuf, usb: &UsbSerialManager) {
    let _ = STORAGE_DIR.set(dir);
    match settings_path().and_then(|path| load_path(&path)) {
        Ok(settings) => if let Err(error) = usb.apply_virtual_device_config(config(&settings)) {
            eprintln!("[virtual-device] {error}");
        },
        Err(error) => eprintln!("[virtual-device] settings unavailable, staying off: {error}"),
    }
}

#[tauri::command]
pub fn load_virtual_device_settings(usb_manager: tauri::State<'_, UsbSerialManager>) -> Result<SettingsStatus, String> {
    Ok(status(load_path(&settings_path()?)?, &usb_manager))
}

#[tauri::command]
pub fn save_virtual_device_settings(
    input: Settings,
    usb_manager: tauri::State<'_, UsbSerialManager>,
) -> Result<SettingsStatus, String> {
    let settings = validated(input)?;
    write_path(&settings_path()?, &settings)?;
    usb_manager.apply_virtual_device_config(config(&settings))?;
    Ok(status(settings, &usb_manager))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virtual_device_is_off_until_the_user_enables_it() {
        let dir = tempfile::tempdir().unwrap();
        let settings = load_path(&dir.path().join(FILE_NAME)).unwrap();
        assert!(!settings.enabled);
        assert_eq!(config(&settings).map(|c| c.port), None);
        assert_eq!(settings.allowed_origins, ["https://thorb.com", "http://localhost:*", "http://127.0.0.1:*"]);
    }

    #[test]
    fn saved_settings_round_trip_with_canonical_origins() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        let input = Settings { enabled: true, allowed_origins: vec![" HTTPS://Thorb.com/ ".into(), "".into(), "https://thorb.com".into(), "https://staging.example".into()] };
        write_path(&path, &validated(input).unwrap()).unwrap();
        let loaded = load_path(&path).unwrap();
        assert!(loaded.enabled);
        assert_eq!(loaded.allowed_origins, ["https://thorb.com", "https://staging.example"]);
        assert_eq!(config(&loaded).unwrap().port, virtual_device::DEFAULT_PORT);
    }

    #[test]
    fn invalid_origins_are_not_saved() {
        let input = Settings { enabled: true, allowed_origins: vec!["https://thorb.com/virtual".into()] };
        assert!(validated(input).is_err());
    }
}
