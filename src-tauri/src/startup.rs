use serde::Serialize;
use winreg::{
    enums::{HKEY_CURRENT_USER, KEY_SET_VALUE, REG_BINARY},
    RegKey, RegValue,
};
const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const APPROVED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";

#[derive(Serialize)]
pub struct StartupStatus {
    supported: bool,
    enabled: bool,
    hidden: bool,
}

#[tauri::command]
pub fn get_startup() -> Result<StartupStatus, String> {
    let supported = !cfg!(debug_assertions);
    let user = RegKey::predef(HKEY_CURRENT_USER);
    let registered = user
        .open_subkey(RUN)
        .and_then(|key| key.get_value::<String, _>(crate::APP_DISPLAY_NAME))
        .ok();
    let allowed = user
        .open_subkey(APPROVED)
        .and_then(|key| key.get_raw_value(crate::APP_DISPLAY_NAME))
        .ok()
        .map(|value| value.bytes.iter().skip(4).all(|byte| *byte == 0))
        .unwrap_or(true);
    Ok(StartupStatus {
        supported,
        enabled: supported
            && registered.as_deref()
                == Some(command(&std::env::current_exe().map_err(|e| e.to_string())?).as_str())
            && allowed,
        hidden: crate::load_shell_prefs().autostart_hidden,
    })
}

#[tauri::command]
pub fn set_startup(enabled: bool) -> Result<StartupStatus, String> {
    if cfg!(debug_assertions) {
        return Err("开发版不注册开机启动，请在安装版中设置".into());
    }
    let user = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = user.create_subkey(RUN).map_err(|e| e.to_string())?;
    if enabled {
        key.set_value(
            crate::APP_DISPLAY_NAME,
            &command(&std::env::current_exe().map_err(|e| e.to_string())?),
        )
        .map_err(|e| e.to_string())?;
        if let Ok(approved) = user.open_subkey_with_flags(APPROVED, KEY_SET_VALUE) {
            approved
                .set_raw_value(
                    crate::APP_DISPLAY_NAME,
                    &RegValue {
                        vtype: REG_BINARY,
                        bytes: vec![2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                    },
                )
                .map_err(|e| e.to_string())?;
        }
    } else if key.get_raw_value(crate::APP_DISPLAY_NAME).is_ok() {
        key.delete_value(crate::APP_DISPLAY_NAME)
            .map_err(|e| e.to_string())?;
    }
    get_startup()
}

pub fn is_background() -> bool {
    std::env::args().any(|arg| arg == "--autostart") && crate::load_shell_prefs().autostart_hidden
}

pub fn notify(app: &tauri::AppHandle, title: &str, body: &str) {
    use tauri_plugin_notification::NotificationExt;
    if let Err(error) = app.notification().builder().title(title).body(body).show() {
        log::warn!(target:"plans","[notification_failed] {error}");
    }
}

pub fn show_if_foreground(app: &tauri::AppHandle) {
    if !is_background() {
        crate::tray::show_main(app);
    }
}

fn command(path: &std::path::Path) -> String {
    format!("\"{}\" --autostart", path.display())
}

#[cfg(test)]
mod tests {
    #[test]
    fn startup_command_quotes_paths_with_spaces() {
        assert_eq!(
            super::command(std::path::Path::new(
                r"C:\Program Files\XMU Sports\xmu-sports.exe"
            )),
            r#""C:\Program Files\XMU Sports\xmu-sports.exe" --autostart"#
        );
    }
}
