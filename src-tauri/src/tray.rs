//! 系统托盘：打开主窗 / 退出。

use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, Runtime,
};

const TRAY_ID: &str = "main-tray";

pub fn tray_id() -> &'static str {
    TRAY_ID
}

pub fn setup_tray<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let icon = app
        .default_window_icon()
        .ok_or_else(|| "托盘：无默认窗口图标".to_string())?
        .clone();

    let open_label = format!("打开 {}", crate::APP_DISPLAY_NAME);
    let open = MenuItem::with_id(app, "open", open_label, true, None::<&str>)
        .map_err(|e| format!("托盘菜单 open: {e}"))?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)
        .map_err(|e| format!("托盘菜单 quit: {e}"))?;
    let menu = Menu::with_items(app, &[&open, &quit]).map_err(|e| format!("托盘菜单: {e}"))?;

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip(crate::APP_DISPLAY_NAME)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_main(app),
            "quit" => {
                if crate::busy_now(app) {
                    show_main(app);
                    let _=app.emit("shell-ask-close",serde_json::json!({"armed":true}));
                } else {request_update_quit(app);}
            },
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                ..
            } = event
            {
                show_main(tray.app_handle());
            }
        })
        .build(app)
        .map_err(|e| format!("托盘构建失败: {e}"))?;

    Ok(())
}

pub fn show_main<R: Runtime>(app: &AppHandle<R>) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
    }
}

pub fn quit_app<R: Runtime>(app: &AppHandle<R>) {
    // 托盘退出：尽量取消挂机并停子进程
    if let Some(state) = app.try_state::<crate::AppState>() {
        state.arm_gen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let _ = crate::clear_arm_file();
        crate::kill_child(&state);
    }
    app.exit(0);
}

pub fn request_update_quit<R:Runtime>(app:&AppHandle<R>) {
    if crate::updates::has_update() {
        show_main(app);
        let _=app.emit("shell-update-exit",());
    } else {quit_app(app);}
}
