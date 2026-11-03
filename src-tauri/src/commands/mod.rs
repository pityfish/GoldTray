use std::collections::HashMap;

use rust_decimal::Decimal;
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::models::{AlertInput, AlertRule, GoldQuote};
use crate::services::alert_engine::{self, EngineSettings};
use crate::services::quote_service::AppState;
use crate::services::scheduler;
use crate::tray;

/// 显示项状态（悬浮行情条 / 菜单栏价格文字）
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayOptions {
    pub hud_enabled: bool,
    pub tray_title_enabled: bool,
}

#[tauri::command]
pub fn get_display_options(state: State<'_, AppState>) -> Result<DisplayOptions, String> {
    let db = state.db.lock().unwrap();
    Ok(DisplayOptions {
        hud_enabled: tray::read_display_setting(&db, tray::SETTING_HUD_ENABLED, false),
        tray_title_enabled: tray::read_display_setting(&db, tray::SETTING_TRAY_TITLE, true),
    })
}

/// 设置显示项：持久化 → 同步窗口/托盘 → 广播给前端刷新
#[tauri::command]
pub fn set_display_option(
    app: AppHandle,
    state: State<'_, AppState>,
    kind: String,
    enabled: bool,
) -> Result<(), String> {
    let key = match kind.as_str() {
        "hud" => tray::SETTING_HUD_ENABLED,
        "trayTitle" => tray::SETTING_TRAY_TITLE,
        _ => return Err(format!("未知的显示项：{kind}")),
    };
    {
        let db = state.db.lock().unwrap();
        db.set_setting(key, if enabled { "true" } else { "false" })?;
    }
    tray::apply_display(&app, &kind, enabled);
    let _ = app.emit("display-options-changed", kind);
    Ok(())
}

/// 最近一次有效行情（可能为空）
#[tauri::command]
pub fn get_latest_quote(state: State<'_, AppState>) -> Option<GoldQuote> {
    state.latest_quote.lock().unwrap().clone()
}

/// 立即刷新行情（与后台轮询共用同一请求闸门）
#[tauri::command]
pub async fn refresh_quote(app: tauri::AppHandle) -> Result<GoldQuote, String> {
    scheduler::fetch_and_process(&app).await
}

#[tauri::command]
pub fn list_alerts(state: State<'_, AppState>) -> Result<Vec<AlertRule>, String> {
    state.db.lock().unwrap().list_alerts()
}

#[tauri::command]
pub fn create_alert(state: State<'_, AppState>, input: AlertInput) -> Result<AlertRule, String> {
    validate_input(&input)?;
    let now = chrono::Local::now().to_rfc3339();
    let rule = AlertRule {
        id: uuid::Uuid::new_v4().to_string(),
        name: input.name.trim().to_string(),
        product_code: input.product_code.clone(),
        direction: input.direction,
        price_type: input.price_type,
        target_price: input.target_price,
        trigger_mode: input.trigger_mode,
        cooldown_minutes: input.cooldown_minutes,
        enabled: true,
        armed: true,
        confirm_hits: 0,
        last_triggered_at: None,
        last_triggered_price: None,
        created_at: now.clone(),
        updated_at: now,
    };

    // 若创建时价格已处于目标区间，默认不立即触发，等待离开后重新进入
    let db = state.db.lock().unwrap();
    if let Some(latest) = state.latest_quote.lock().unwrap().clone() {
        let spread = db
            .get_setting("estimated_spread")
            .ok()
            .flatten()
            .and_then(|v| v.trim().parse::<Decimal>().ok());
        let settings = EngineSettings {
            estimated_spread: spread,
            staleness_threshold_secs: 300,
        };
        if alert_engine::is_currently_in_zone(&latest, &rule, &settings) == Some(true) {
            let mut rule = rule;
            rule.armed = false;
            db.insert_alert(&rule)?;
            return Ok(rule);
        }
    }
    db.insert_alert(&rule)?;
    Ok(rule)
}

#[tauri::command]
pub fn update_alert(
    state: State<'_, AppState>,
    id: String,
    input: AlertInput,
) -> Result<AlertRule, String> {
    validate_input(&input)?;
    {
        let db = state.db.lock().unwrap();
        db.update_alert(&id, &input)?;
    }
    // 编辑后重置了触发状态，按当前行情重新校准 armed
    recalibrate(&state, &id)?;
    state
        .db
        .lock()
        .unwrap()
        .get_alert(&id)?
        .ok_or_else(|| "规则不存在".to_string())
}

#[tauri::command]
pub fn delete_alert(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.db.lock().unwrap().delete_alert(&id)
}

#[tauri::command]
pub fn set_alert_enabled(
    state: State<'_, AppState>,
    id: String,
    enabled: bool,
) -> Result<(), String> {
    {
        let db = state.db.lock().unwrap();
        db.set_alert_enabled(&id, enabled)?;
    }
    if enabled {
        recalibrate(&state, &id)?;
    }
    Ok(())
}

#[tauri::command]
pub fn list_alert_history(
    state: State<'_, AppState>,
    limit: Option<i64>,
) -> Result<Vec<crate::models::AlertTriggerRecord>, String> {
    state
        .db
        .lock()
        .unwrap()
        .list_alert_history(limit.unwrap_or(100).clamp(1, 500))
}

/// 删除单条触发记录
#[tauri::command]
pub fn delete_alert_history(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.db.lock().unwrap().delete_alert_history(&id)
}

/// 清空全部触发记录
#[tauri::command]
pub fn clear_alert_history(state: State<'_, AppState>) -> Result<(), String> {
    state.db.lock().unwrap().clear_alert_history()
}

#[tauri::command]
pub fn get_settings(
    state: State<'_, AppState>,
    keys: Vec<String>,
) -> Result<HashMap<String, Option<String>>, String> {
    Ok(state
        .db
        .lock()
        .unwrap()
        .get_settings(&keys)?
        .into_iter()
        .collect())
}

#[tauri::command]
pub fn set_setting(state: State<'_, AppState>, key: String, value: String) -> Result<(), String> {
    // 新增/启用规则后立即按新设置重新校准阈值相关行为，这里只持久化
    state.db.lock().unwrap().set_setting(&key, &value)
}

fn validate_input(input: &AlertInput) -> Result<(), String> {
    if input.name.trim().is_empty() {
        return Err("规则名称不能为空".to_string());
    }
    let target = input
        .target_price
        .trim()
        .parse::<Decimal>()
        .map_err(|_| format!("目标价格不是合法数字：{}", input.target_price))?;
    if target <= Decimal::ZERO {
        return Err("目标价格必须为正数".to_string());
    }
    if !(0..=1440).contains(&input.cooldown_minutes) {
        return Err("冷却时间必须在 0～1440 分钟之间".to_string());
    }
    Ok(())
}

/// 按最新行情重新校准规则的 armed 状态
fn recalibrate(state: &State<'_, AppState>, id: &str) -> Result<(), String> {
    let db = state.db.lock().unwrap();
    let Some(rule) = db.get_alert(id)? else {
        return Ok(());
    };
    let Some(latest) = state.latest_quote.lock().unwrap().clone() else {
        return Ok(());
    };
    let spread = db
        .get_setting("estimated_spread")
        .ok()
        .flatten()
        .and_then(|v| v.trim().parse::<Decimal>().ok());
    let settings = EngineSettings {
        estimated_spread: spread,
        staleness_threshold_secs: 300,
    };
    let in_zone = alert_engine::is_currently_in_zone(&latest, &rule, &settings);
    let armed = in_zone != Some(true);
    if armed != rule.armed {
        db.update_alert_state(
            id,
            armed,
            0,
            rule.last_triggered_at.as_deref(),
            rule.last_triggered_price.as_deref(),
            rule.enabled,
        )?;
    }
    Ok(())
}

// ============ 历史行情 ============

/// 获取走势图数据（Rust 侧缓存优先，网络异常降级旧缓存）
#[tauri::command]
pub async fn get_history(
    app: tauri::AppHandle,
    period: String,
    force: Option<bool>,
) -> Result<crate::models::historical_quote::HistoryPayload, String> {
    let period = crate::models::historical_quote::HistoryPeriod::from_api(&period)
        .ok_or_else(|| format!("未知的历史周期：{period}"))?;
    crate::services::history_service::get_history(&app, period, force.unwrap_or(false)).await
}

// ============ 智能企稳预警 ============

fn validate_stabilization_input(
    input: &crate::models::stabilization::StabilizationInput,
) -> Result<(), String> {
    use crate::models::stabilization::StabilizationInput;
    let StabilizationInput {
        name,
        observation_price,
        stable_days,
        low_tolerance,
        breakout_days,
        breakout_confirm_hours,
        cooldown_minutes,
        ..
    } = input;
    if name.trim().is_empty() {
        return Err("规则名称不能为空".to_string());
    }
    let price = observation_price
        .trim()
        .parse::<Decimal>()
        .map_err(|_| format!("观察价格不是合法数字：{observation_price}"))?;
    if price <= Decimal::ZERO {
        return Err("观察价格必须为正数".to_string());
    }
    let tol = low_tolerance
        .trim()
        .parse::<f64>()
        .map_err(|_| format!("低点偏差不是合法数字：{low_tolerance}"))?;
    if !(0.0..=100.0).contains(&tol) {
        return Err("低点偏差必须在 0～100 之间".to_string());
    }
    if !(1..=10).contains(stable_days) {
        return Err("观察交易日必须在 1～10 天之间".to_string());
    }
    if !(1..=10).contains(breakout_days) {
        return Err("突破参考周期必须在 1～10 天之间".to_string());
    }
    if !(1..=24).contains(breakout_confirm_hours) {
        return Err("确认持续时间必须在 1～24 小时之间".to_string());
    }
    if !(0..=1440).contains(cooldown_minutes) {
        return Err("冷却时间必须在 0～1440 分钟之间".to_string());
    }
    Ok(())
}

#[tauri::command]
pub fn list_stabilization_alerts(
    state: State<'_, AppState>,
) -> Result<Vec<crate::models::stabilization::StabilizationRule>, String> {
    crate::storage::stabilization_repository::StabilizationRepo(state.db.lock().unwrap().conn())
        .list(false)
}

#[tauri::command]
pub fn create_stabilization_alert(
    state: State<'_, AppState>,
    input: crate::models::stabilization::StabilizationInput,
) -> Result<crate::models::stabilization::StabilizationRule, String> {
    use crate::models::stabilization::{StabilizationRule, StabilizationState};
    validate_stabilization_input(&input)?;
    let now = chrono::Local::now().to_rfc3339();
    let rule = StabilizationRule {
        id: uuid::Uuid::new_v4().to_string(),
        name: input.name.trim().to_string(),
        product_code: input.product_code.clone(),
        observation_price: input.observation_price.trim().to_string(),
        stable_days: input.stable_days,
        low_tolerance: input.low_tolerance.trim().to_string(),
        breakout_days: input.breakout_days,
        breakout_confirm_hours: input.breakout_confirm_hours,
        trigger_mode: input.trigger_mode,
        cooldown_minutes: input.cooldown_minutes,
        notify_stabilizing: input.notify_stabilizing,
        notify_confirmed: input.notify_confirmed,
        enabled: true,
        state: StabilizationState::WaitingForDip,
        observation_started_at: None,
        observation_low: None,
        last_evaluated_at: None,
        created_at: now.clone(),
        updated_at: now,
    };
    crate::storage::stabilization_repository::StabilizationRepo(state.db.lock().unwrap().conn())
        .insert(&rule)?;
    Ok(rule)
}

#[tauri::command]
pub fn update_stabilization_alert(
    state: State<'_, AppState>,
    id: String,
    input: crate::models::stabilization::StabilizationInput,
) -> Result<crate::models::stabilization::StabilizationRule, String> {
    use crate::models::stabilization::{StabilizationEvent, StabilizationState};
    use crate::storage::stabilization_repository::StabilizationRepo;
    validate_stabilization_input(&input)?;
    let now = chrono::Local::now().to_rfc3339();
    let db = state.db.lock().unwrap();
    let repo = StabilizationRepo(db.conn());
    let old = repo.get(&id)?.ok_or_else(|| "规则不存在".to_string())?;
    repo.update_config(&id, &input)?;
    // 修改配置视为重置：记录状态变更事件（保留历史，不删除）
    repo.insert_event(&StabilizationEvent {
        id: uuid::Uuid::new_v4().to_string(),
        alert_id: id.clone(),
        alert_name: old.name.clone(),
        previous_state: Some(old.state.as_str().to_string()),
        current_state: StabilizationState::WaitingForDip.as_str().to_string(),
        reference_price: None,
        reason: Some("用户修改规则配置，观察重新开始".to_string()),
        event_time: now,
    })?;
    repo.get(&id)?.ok_or_else(|| "规则不存在".to_string())
}

#[tauri::command]
pub fn delete_stabilization_alert(state: State<'_, AppState>, id: String) -> Result<(), String> {
    crate::storage::stabilization_repository::StabilizationRepo(state.db.lock().unwrap().conn())
        .delete(&id)
}

#[tauri::command]
pub fn set_stabilization_alert_enabled(
    state: State<'_, AppState>,
    id: String,
    enabled: bool,
) -> Result<(), String> {
    crate::storage::stabilization_repository::StabilizationRepo(state.db.lock().unwrap().conn())
        .set_enabled(&id, enabled)
}

#[tauri::command]
pub fn reset_stabilization_alert(state: State<'_, AppState>, id: String) -> Result<(), String> {
    use crate::models::stabilization::{StabilizationEvent, StabilizationState};
    use crate::storage::stabilization_repository::StabilizationRepo;
    let now = chrono::Local::now().to_rfc3339();
    let db = state.db.lock().unwrap();
    let repo = StabilizationRepo(db.conn());
    let old = repo.get(&id)?.ok_or_else(|| "规则不存在".to_string())?;
    repo.reset_state(&id)?;
    repo.insert_event(&StabilizationEvent {
        id: uuid::Uuid::new_v4().to_string(),
        alert_id: id,
        alert_name: old.name,
        previous_state: Some(old.state.as_str().to_string()),
        current_state: StabilizationState::WaitingForDip.as_str().to_string(),
        reference_price: None,
        reason: Some("用户手动重置".to_string()),
        event_time: now,
    })?;
    Ok(())
}

#[tauri::command]
pub fn list_stabilization_events(
    state: State<'_, AppState>,
    alert_id: Option<String>,
    limit: Option<i64>,
) -> Result<Vec<crate::models::stabilization::StabilizationEvent>, String> {
    crate::storage::stabilization_repository::StabilizationRepo(state.db.lock().unwrap().conn())
        .list_events(alert_id.as_deref(), limit.unwrap_or(100))
}

/// 获取指定规则的企稳判断依据（供「查看判断依据」弹窗）
#[tauri::command]
pub fn assess_stabilization(
    state: State<'_, AppState>,
    id: String,
) -> Result<crate::models::stabilization::StabilizationAssessment, String> {
    use crate::models::historical_quote::HistoryPeriod;
    use crate::storage::{history_repository, stabilization_repository::StabilizationRepo};
    let db = state.db.lock().unwrap();
    let repo = StabilizationRepo(db.conn());
    let rule = repo.get(&id)?.ok_or_else(|| "规则不存在".to_string())?;
    let latest = state
        .latest_quote
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|q| q.customer_buy_price.trim().parse::<f64>().ok())
        .ok_or_else(|| "暂无有效行情，无法评估".to_string())?;
    let (bars14, _, _) = history_repository::load_bars(db.conn(), HistoryPeriod::D14)?;
    let (bars48, _, _) = history_repository::load_bars(db.conn(), HistoryPeriod::H48)?;
    Ok(crate::services::stabilization_engine::assess(
        &rule,
        latest,
        &bars14,
        &bars48,
        &chrono::Local::now(),
    ))
}
