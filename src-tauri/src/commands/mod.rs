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
