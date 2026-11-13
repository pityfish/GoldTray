mod commands;
mod models;
mod providers;
mod services;
mod storage;
mod tray;

use std::path::PathBuf;

use services::quote_service::AppState;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // 已有实例运行时：唤起主窗口
            tray::show_main_window(app);
        }))
        .setup(|app| {
            let data_dir: PathBuf = app
                .path()
                .app_data_dir()
                .expect("failed to resolve app data dir");
            std::fs::create_dir_all(&data_dir).map_err(|e| format!("创建数据目录失败：{e}"))?;
            let db = storage::db::Db::open(&data_dir.join("goldtray.db"))
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
            app.manage(AppState::new(db));

            tray::create_tray(app.handle())?;
            // 按持久化配置恢复显示项（悬浮条位置 + 显示状态）
            tray::startup_display(app.handle());

            // 启动后台行情轮询（首次请求在循环开始立即执行）
            services::scheduler::spawn(app.handle().clone());

            Ok(())
        })
        .on_window_event(|window, event| {
            match event {
                // 点击关闭按钮：隐藏窗口，托盘继续运行
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    let _ = window.hide();
                    api.prevent_close();
                }
                // 悬浮行情条拖动时持久化位置（SQLite WAL 高频小写入可接受）
                tauri::WindowEvent::Moved(position) if window.label() == tray::HUD_WINDOW => {
                    let app = window.app_handle();
                    let state = app.state::<AppState>();
                    let db = state.db.lock().unwrap();
                    let _ = db.set_setting("hud_pos_x", &position.x.to_string());
                    let _ = db.set_setting("hud_pos_y", &position.y.to_string());
                }
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_latest_quote,
            commands::refresh_quote,
            commands::list_alerts,
            commands::create_alert,
            commands::update_alert,
            commands::delete_alert,
            commands::set_alert_enabled,
            commands::list_alert_history,
            commands::clear_alert_history,
            commands::get_settings,
            commands::set_setting,
            commands::get_display_options,
            commands::set_display_option,
            commands::get_history,
            commands::list_stabilization_alerts,
            commands::create_stabilization_alert,
            commands::update_stabilization_alert,
            commands::delete_stabilization_alert,
            commands::set_stabilization_alert_enabled,
            commands::reset_stabilization_alert,
            commands::list_stabilization_events,
            commands::clear_stabilization_events,
            commands::assess_stabilization,
        ])
        .run(tauri::generate_context!())
        .expect("error while running GoldTray");
}
