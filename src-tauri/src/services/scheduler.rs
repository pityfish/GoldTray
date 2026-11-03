use std::time::Duration;

use rust_decimal::Decimal;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::models::{AlertTriggeredEvent, GoldQuote};
use crate::services::alert_engine::{self, EngineSettings};
use crate::services::history_service;
use crate::services::quote_service::AppState;
use crate::tray;

/// 读取引擎相关设置
fn read_engine_settings(state: &AppState) -> (EngineSettings, bool) {
    let db = state.db.lock().unwrap();
    let spread = db
        .get_setting("estimated_spread")
        .ok()
        .flatten()
        .and_then(|v| v.trim().parse::<Decimal>().ok());
    let staleness = db
        .get_setting("staleness_threshold_secs")
        .ok()
        .flatten()
        .and_then(|v| v.trim().parse::<i64>().ok())
        .unwrap_or(300);
    let notify_enabled = db
        .get_setting("enable_notification")
        .ok()
        .flatten()
        .map(|v| v == "true")
        .unwrap_or(true);
    let settings = EngineSettings {
        estimated_spread: spread,
        staleness_threshold_secs: staleness,
    };
    (settings, notify_enabled)
}

/// 读取刷新间隔（秒），限制在 [30, 3600]，默认 60
pub fn read_refresh_interval_secs(state: &AppState) -> u64 {
    let db = state.db.lock().unwrap();
    let v = db
        .get_setting("refresh_interval_secs")
        .ok()
        .flatten()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(60);
    v.clamp(30, 3600)
}

/// 拉取一次行情并完成：缓存更新 → 托盘刷新 → 前端事件 → 预警评估 → 系统通知。
/// 通过 fetch_gate 保证同一时刻只有一个请求进行。
pub async fn fetch_and_process(app: &AppHandle) -> Result<GoldQuote, String> {
    let state = app.state::<AppState>();
    let _gate = state.fetch_gate.lock().await;

    let quote = match state.provider.fetch_quote().await {
        Ok(q) => q,
        Err(e) => {
            state.set_error(&e);
            let _ = app.emit("quote-error", e.clone());
            tray::update_tray(app, None);
            return Err(e);
        }
    };

    state.set_quote(quote.clone());
    let _ = app.emit("quote-updated", &quote);
    tray::update_tray(app, Some(&quote));

    // 预警评估（引擎内部含行情失效保护）；作用域块确保锁在 await 前释放
    let (engine_settings, notify_enabled) = read_engine_settings(&state);
    let outcome = {
        let db = state.db.lock().unwrap();
        alert_engine::evaluate(&db, &quote, &engine_settings)
    };
    match outcome {
        Ok(outcome) => {
            for (mut record, price, _) in outcome.triggered {
                let status = if notify_enabled {
                    let (title, body) = alert_engine::notification_body(&record, &price);
                    match app.notification().builder().title(title).body(body).show() {
                        Ok(()) => "SENT",
                        Err(e) => {
                            eprintln!("[goldtray] 发送通知失败：{e}");
                            "FAILED"
                        }
                    }
                } else {
                    "SKIPPED"
                };
                record.notification_status = status.to_string();
                {
                    let db = state.db.lock().unwrap();
                    if let Err(e) = db.set_alert_notification_status(&record.id, status) {
                        eprintln!("[goldtray] 保存通知状态失败：{e}");
                    }
                }
                let _ = app.emit(
                    "alert-triggered",
                    AlertTriggeredEvent {
                        record,
                        current_price: quote.customer_buy_price.clone(),
                    },
                );
            }
        }
        Err(e) => eprintln!("[goldtray] 预警评估失败：{e}"),
    }

    // 智能企稳引擎评估（历史数据缺失时引擎内部自行降级，不影响实时预警）
    if let Err(e) = crate::services::stabilization_engine::evaluate_all(app, notify_enabled).await {
        eprintln!("[goldtray] 企稳评估失败：{e}");
    }

    Ok(quote)
}

/// 历史行情后台刷新：48H 每 15 分钟、14d 每 30 分钟（企稳引擎数据源）。
/// 其余长周期由前端请求时按需缓存。
pub fn spawn_history_refresher(app: AppHandle) {
    use crate::models::historical_quote::HistoryPeriod;
    tauri::async_runtime::spawn(async move {
        let mut tick: u64 = 0;
        loop {
            // 立即首次刷新
            if let Err(e) = history_service::get_history(&app, HistoryPeriod::H48, true).await {
                eprintln!("[goldtray] 48H 历史刷新失败：{e}");
            }
            if tick % 2 == 0 {
                if let Err(e) = history_service::get_history(&app, HistoryPeriod::D14, true).await {
                    eprintln!("[goldtray] 14d 历史刷新失败：{e}");
                }
            }
            tick += 1;
            tokio::time::sleep(Duration::from_secs(15 * 60)).await;
        }
    });
}

/// 后台轮询任务：启动后立即拉取，成功后按设定周期轮询，失败时退避重试。
pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut fail_streak: u32 = 0;
        loop {
            let ok = fetch_and_process(&app).await.is_ok();
            let interval = {
                let state = app.state::<AppState>();
                read_refresh_interval_secs(&state)
            };
            let wait = if ok {
                fail_streak = 0;
                interval
            } else {
                fail_streak += 1;
                // 指数退避：15s → 30s → 60s …，上限为正常轮询间隔
                let shift = fail_streak.min(6) - 1;
                (15u64.saturating_mul(1u64 << shift)).min(interval)
            };
            tokio::time::sleep(Duration::from_secs(wait)).await;
        }
    });
}
