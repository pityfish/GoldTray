use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

use crate::services::quote_service::AppState;
use crate::services::scheduler;

pub const TRAY_ID: &str = "goldtray-tray";

pub fn create_tray(app: &AppHandle) -> Result<(), tauri::Error> {
    let open_item = MenuItem::with_id(app, "open", "打开行情", true, None::<&str>)?;
    let alert_item = MenuItem::with_id(app, "alerts", "预警管理", true, None::<&str>)?;
    let refresh_item = MenuItem::with_id(app, "refresh", "立即刷新", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let quit_item = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[&open_item, &alert_item, &refresh_item, &sep, &quit_item],
    )?;

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

/// 行情变化时同步更新托盘。
/// macOS：通过原生接口设置菜单栏文字标题；
/// Windows：set_title 不生效，依赖 tooltip 展示价格。
pub fn update_tray(app: &AppHandle, quote: Option<&crate::models::GoldQuote>) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };

    let Some(quote) = quote else {
        // 获取失败：保留最后一次有效报价并标记异常
        let state = app.state::<AppState>();
        let last = state.latest_quote.lock().unwrap().clone();
        match last {
            Some(last) => {
                let title = format!("Au ¥{} ⚠", format_price(&last.customer_buy_price));
                let _ = tray.set_title(Some(&title));
                let _ = tray.set_tooltip(Some("行情获取失败，显示的为最近一次有效报价"));
            }
            None => {
                let _ = tray.set_title(Some("Au --.-- ⚠"));
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

    let _ = tray.set_title(Some(&title));
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
