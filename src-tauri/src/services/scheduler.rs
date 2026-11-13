use std::{sync::atomic::Ordering, time::Duration};

use rust_decimal::Decimal;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::models::historical_quote::HistoryPeriod;
use crate::models::{AlertTriggeredEvent, GoldQuote};
use crate::services::alert_engine::{self, AlertEvaluation, EngineSettings};
use crate::services::history_service;
use crate::services::quote_service::AppState;
use crate::services::{recovery, stabilization_engine};
use crate::storage::stabilization_repository::StabilizationRepo;
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
    let previous_quote = state
        .latest_quote
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|quote| recovery::parse_time(&quote.fetched_at));
    let retrying = state.last_error.lock().unwrap().is_some();

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

    let (engine_settings, notify_enabled) = read_engine_settings(&state);
    let (last_monitored, has_alerts, has_stabilization) = {
        let db = state.db.lock().unwrap();
        (
            db.get_setting(alert_engine::LAST_MONITORED_QUOTE)?
                .as_deref()
                .and_then(recovery::parse_time),
            db.list_alerts()?
                .iter()
                .any(|rule| rule.enabled && rule.product_code == quote.product_code),
            StabilizationRepo(db.conn())
                .list(true)?
                .iter()
                .any(|rule| rule.product_code == quote.product_code),
        )
    };
    let fetched = recovery::parse_time(&quote.fetched_at).ok_or("行情时间格式异常")?;
    let interval = read_refresh_interval_secs(&state);
    let quote_gap = recovery::gap_detected(previous_quote, fetched, interval, retrying);
    let recover_alerts =
        has_alerts && (quote_gap || state.alert_recovery_pending.load(Ordering::Relaxed));
    let recover_stabilization = has_stabilization
        && (quote_gap || state.stabilization_recovery_pending.load(Ordering::Relaxed));

    let mut history48 = if recover_alerts {
        get_rule_history(app, HistoryPeriod::H48, true).await
    } else {
        None
    };
    let bars48 = history48.as_ref().map_or(&[][..], |h| h.points.as_slice());
    let short_replay = recover_alerts.then(|| recovery::closed_bars(&[], bars48, fetched));
    let short_covers_gap = short_replay
        .as_deref()
        .is_some_and(|bars| recovery::short_history_covers(bars, last_monitored, fetched));

    // 短缺口先评估价格预警，企稳需要的 14d 请求在预警处理完之后再发。
    let needs_long_for_alerts = recover_alerts && history48.is_some() && !short_covers_gap;
    let mut history14 = if needs_long_for_alerts {
        get_rule_history(app, HistoryPeriod::D14, true).await
    } else {
        None
    };
    let alert_replay = if short_covers_gap {
        short_replay
    } else {
        history14
            .as_ref()
            .filter(|_| needs_long_for_alerts)
            .map(|history| recovery::closed_bars(&history.points, bars48, fetched))
    };
    let alert_pending = recover_alerts && alert_replay.is_none();
    state
        .alert_recovery_pending
        .store(alert_pending, Ordering::Relaxed);
    let outcome = {
        let db = state.db.lock().unwrap();
        let evaluation = match alert_replay.as_deref() {
            Some(bars) => AlertEvaluation::Replay(bars),
            None if alert_pending => AlertEvaluation::AwaitingHistory,
            None => AlertEvaluation::Live,
        };
        alert_engine::evaluate_with_recovery(&db, &quote, &engine_settings, evaluation)
    };
    match outcome {
        Ok(outcome) => {
            if outcome.armed_changed {
                let _ = app.emit("alert-state-changed", ());
            }
            for (mut record, price, _) in outcome.triggered {
                let status = if notify_enabled {
                    let current = if record.historical_from.is_none() {
                        price
                    } else if record.is_estimated {
                        engine_settings
                            .estimated_spread
                            .and_then(|spread| quote.estimated_sell_price(spread))
                            .map(|price| price.to_string())
                            .unwrap_or_else(|| "暂无".to_string())
                    } else {
                        quote.customer_buy_price.clone()
                    };
                    let (title, body) = alert_engine::notification_body(&record, &current);
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
        Err(e) => {
            state
                .alert_recovery_pending
                .store(recover_alerts, Ordering::Relaxed);
            eprintln!("[goldtray] 预警评估失败：{e}");
        }
    }

    if has_stabilization && !needs_long_for_alerts {
        history14 = get_rule_history(app, HistoryPeriod::D14, recover_stabilization).await;
    }
    if has_stabilization && !recover_alerts {
        history48 = get_rule_history(app, HistoryPeriod::H48, recover_stabilization).await;
    }
    let stabilization_pending =
        recover_stabilization && (history14.is_none() || history48.is_none());
    state
        .stabilization_recovery_pending
        .store(stabilization_pending, Ordering::Relaxed);
    if has_stabilization && !stabilization_pending {
        let bars14 = history14.as_ref().map_or(&[][..], |h| h.points.as_slice());
        let bars48 = history48.as_ref().map_or(&[][..], |h| h.points.as_slice());
        let replay = recover_stabilization.then(|| recovery::closed_bars(bars14, bars48, fetched));
        let completed_hours = history48.as_ref().map_or_else(Vec::new, |history| {
            stabilization_engine::completed_hour_points(&history.points, &history.fetched_at)
        });
        if let Err(e) = stabilization_engine::evaluate_all(
            app,
            &quote,
            bars14,
            &completed_hours,
            replay.as_deref(),
            notify_enabled,
        ) {
            state
                .stabilization_recovery_pending
                .store(recover_stabilization, Ordering::Relaxed);
            eprintln!("[goldtray] 企稳评估失败：{e}");
        }
    }

    Ok(quote)
}

async fn get_rule_history(
    app: &AppHandle,
    period: HistoryPeriod,
    force: bool,
) -> Option<crate::models::historical_quote::HistoryPayload> {
    match history_service::get_history(app, period, force).await {
        Ok(history) => (!history.stale).then_some(history),
        Err(e) => {
            eprintln!("[goldtray] {} 历史刷新失败：{e}", period.as_api());
            None
        }
    }
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
