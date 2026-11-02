use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

use crate::services::quote_service::AppState;
use crate::services::scheduler;
use crate::storage::db::Db;

pub const TRAY_ID: &str = "goldtray-tray";
pub const HUD_WINDOW: &str = "hud";

/// 显示项持久化 key：悬浮行情条（默认关闭）
pub const SETTING_HUD_ENABLED: &str = "hud_enabled";
/// 显示项持久化 key：菜单栏价格文字（默认开启）
pub const SETTING_TRAY_TITLE: &str = "tray_title_enabled";

/// 读取显示项布尔设置
pub fn read_display_setting(db: &Db, key: &str, default: bool) -> bool {
    db.get_setting(key)
        .ok()
        .flatten()
        .map(|v| v == "true")
        .unwrap_or(default)
}

pub fn create_tray(app: &AppHandle) -> Result<(), tauri::Error> {
    let state = app.state::<AppState>();
    let (hud_enabled, title_enabled) = {
        let db = state.db.lock().unwrap();
        (
            read_display_setting(&db, SETTING_HUD_ENABLED, false),
            read_display_setting(&db, SETTING_TRAY_TITLE, true),
        )
    };

    let open_item = MenuItem::with_id(app, "open", "打开行情", true, None::<&str>)?;
    let alert_item = MenuItem::with_id(app, "alerts", "预警管理", true, None::<&str>)?;
    let refresh_item = MenuItem::with_id(app, "refresh", "立即刷新", true, None::<&str>)?;
    let hud_item =
        CheckMenuItem::with_id(app, "hud", "悬浮行情条", true, hud_enabled, None::<&str>)?;
    let title_item = CheckMenuItem::with_id(
        app,
        "title",
        "菜单栏价格文字",
        true,
        title_enabled,
        None::<&str>,
    )?;
    let sep = PredefinedMenuItem::separator(app)?;
    let quit_item = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &open_item,
            &alert_item,
            &refresh_item,
            &hud_item,
            &title_item,
            &sep,
            &quit_item,
        ],
    )?;
    let _ = state.hud_check.set(hud_item);
    let _ = state.title_check.set(title_item);

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(
            app.default_window_icon()
                .expect("application icon is required")
                .clone(),
        )
        .tooltip("GoldTray 正在启动…")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" | "alerts" => show_main_window(app),
            "refresh" => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = scheduler::fetch_and_process(&app).await;
                });
            }
            "hud" | "title" => {
                // 托盘勾选切换：取反当前持久化状态
                let enabled = {
                    let state = app.state::<AppState>();
                    let db = state.db.lock().unwrap();
                    match event.id().as_ref() {
                        "hud" => !read_display_setting(&db, SETTING_HUD_ENABLED, false),
                        _ => !read_display_setting(&db, SETTING_TRAY_TITLE, true),
                    }
                };
                let kind = if event.id().as_ref() == "hud" { "hud" } else { "trayTitle" };
                apply_display(app, kind, enabled);
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            // 左键单击：打开或聚焦主窗口（macOS 菜单栏点击习惯）
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

pub fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        if window.is_visible().unwrap_or(false) {
            let _ = window.set_focus();
        } else {
            let _ = window.show();
            let _ = window.set_focus();
        }
    }
}

/// 应用显示项变更：同步窗口显隐 / 托盘标题 / 菜单勾选状态。
/// kind: "hud" | "trayTitle"
pub fn apply_display(app: &AppHandle, kind: &str, enabled: bool) {
    let state = app.state::<AppState>();
    match kind {
        "hud" => {
            if let Some(check) = state.hud_check.get() {
                let _ = check.set_checked(enabled);
            }
            if let Some(hud) = app.get_webview_window(HUD_WINDOW) {
                if enabled {
                    let _ = hud.show();
                } else {
                    let _ = hud.hide();
                }
            }
        }
        "trayTitle" => {
            if let Some(check) = state.title_check.get() {
                let _ = check.set_checked(enabled);
            }
            // 立即按新状态刷新托盘标题（取最近一次有效行情）
            let latest = state.latest_quote.lock().unwrap().clone();
            update_tray(app, latest.as_ref());
        }
        _ => {}
    }
}

/// 启动时按持久化配置恢复显示项：悬浮条开启则恢复位置并显示
pub fn startup_display(app: &AppHandle) {
    let hud_enabled = {
        let state = app.state::<AppState>();
        let db = state.db.lock().unwrap();
        read_display_setting(&db, SETTING_HUD_ENABLED, false)
    };
    if hud_enabled {
        restore_hud_position(app);
        if let Some(hud) = app.get_webview_window(HUD_WINDOW) {
            let _ = hud.show();
        }
    }
}

/// 启动时恢复悬浮行情条上次保存的屏幕位置
pub fn restore_hud_position(app: &AppHandle) {
    let Some(hud) = app.get_webview_window(HUD_WINDOW) else {
        return;
    };
    let state = app.state::<AppState>();
    let (x, y) = {
        let db = state.db.lock().unwrap();
        match (
            db.get_setting("hud_pos_x").ok().flatten(),
            db.get_setting("hud_pos_y").ok().flatten(),
        ) {
            (Some(x), Some(y)) => match (x.trim().parse::<i32>(), y.trim().parse::<i32>()) {
                (Ok(x), Ok(y)) => (x, y),
                _ => return,
            },
            _ => return,
        }
    };
    let _ = hud.set_position(tauri::PhysicalPosition::new(x, y));
}

/// 行情变化时同步更新托盘。
/// macOS：通过原生接口设置菜单栏文字标题；
/// Windows：set_title 不生效，依赖 tooltip 展示价格。
pub fn update_tray(app: &AppHandle, quote: Option<&crate::models::GoldQuote>) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };

    // 「菜单栏价格文字」关闭时：macOS 不显示标题（Windows 本就不显示），tooltip 保留
    let title_enabled = {
        let state = app.state::<AppState>();
        let db = state.db.lock().unwrap();
        read_display_setting(&db, SETTING_TRAY_TITLE, true)
    };
    let apply_title = |title: Option<String>| {
        let _ = if title_enabled {
            tray.set_title(title.as_deref())
        } else {
            tray.set_title::<&str>(None)
        };
    };

    let Some(quote) = quote else {
        // 获取失败：保留最后一次有效报价并标记异常
        let state = app.state::<AppState>();
        let last = state.latest_quote.lock().unwrap().clone();
        match last {
            Some(last) => {
                apply_title(Some(format!("Au ¥{} ⚠", format_price(&last.customer_buy_price))));
                let _ = tray.set_tooltip(Some("行情获取失败，显示的为最近一次有效报价"));
            }
            None => {
                apply_title(Some("Au --.-- ⚠".to_string()));
                let _ = tray.set_tooltip(Some("暂无有效行情，等待重试…"));
            }
        }
        return;
    };

    let price = format_price(&quote.customer_buy_price);
    let change_text = quote
        .raw_change
        .as_deref()
        .and_then(|c| c.trim().parse::<f64>().ok())
        .filter(|c| *c != 0.0 && c.is_finite())
        .map(|c| {
            if c > 0.0 {
                format!(" ↑{c:.2}")
            } else {
                format!(" ↓{:.2}", c.abs())
            }
        })
        .unwrap_or_default();

    let mut title = format!("Au ¥{price}{change_text}");

    // 行情过期标记 + 估算卖出价（设置过价差时显示）
    let (staleness, estimated_sell_line) = {
        let state = app.state::<AppState>();
        let db = state.db.lock().unwrap();
        let staleness = db
            .get_setting("staleness_threshold_secs")
            .ok()
            .flatten()
            .and_then(|v| v.trim().parse::<i64>().ok())
            .unwrap_or(300);
        // 估算卖出价 = 买入参考价 − 价差
        let sell_line = db
            .get_setting("estimated_spread")
            .ok()
            .flatten()
            .and_then(|v| v.trim().parse::<f64>().ok())
            .filter(|s| *s > 0.0)
            .zip(price.parse::<f64>().ok())
            .map(|(s, buy)| format!("\n估算卖出价：¥{:.2} 元/克", buy - s))
            .unwrap_or_default();
        (staleness, sell_line)
    };
    if let Ok(fetched) = chrono::DateTime::parse_from_rfc3339(&quote.fetched_at) {
        let age = chrono::Utc::now().signed_duration_since(fetched.with_timezone(&chrono::Utc));
        if age.num_seconds() > staleness {
            title.push_str(" · 已过期");
        }
    }

    apply_title(Some(title));
    let change_line = if change_text.is_empty() {
        "持平".to_string()
    } else {
        change_text.trim_start().to_string()
    };
    let _ = tray.set_tooltip(Some(&format!(
        "{} {}\n参考买入价：¥{} 元/克（较昨日{}）{}\n获取时间：{}",
        quote.bank_name,
        quote.product_name,
        price,
        change_line,
        estimated_sell_line,
        quote.fetched_at.get(11..19).unwrap_or(&quote.fetched_at)
    )));
}

fn format_price(price: &str) -> String {
    price
        .trim()
        .parse::<f64>()
        .map(|p| format!("{p:.2}"))
        .unwrap_or_else(|_| price.trim().to_string())
}
