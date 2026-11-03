use chrono::Local;
use rusqlite::{params, Connection, OptionalExtension};

use crate::models::stabilization::{
    StabilizationEvent, StabilizationInput, StabilizationRule, StabilizationState,
};
use crate::models::AlertTriggerMode;

fn parse_trigger_mode(s: &str) -> AlertTriggerMode {
    if s == "REARMABLE" {
        AlertTriggerMode::Rearmable
    } else {
        AlertTriggerMode::Once
    }
}

fn row_to_rule(row: &rusqlite::Row) -> rusqlite::Result<StabilizationRule> {
    Ok(StabilizationRule {
        id: row.get("id")?,
        name: row.get("name")?,
        product_code: row.get("product_code")?,
        observation_price: row.get("observation_price")?,
        stable_days: row.get("stable_days")?,
        low_tolerance: row.get("low_tolerance")?,
        breakout_days: row.get("breakout_days")?,
        breakout_confirm_hours: row.get("breakout_confirm_hours")?,
        trigger_mode: parse_trigger_mode(&row.get::<_, String>("trigger_mode")?),
        cooldown_minutes: row.get("cooldown_minutes")?,
        notify_stabilizing: row.get::<_, i64>("notify_stabilizing")? != 0,
        notify_confirmed: row.get::<_, i64>("notify_confirmed")? != 0,
        enabled: row.get::<_, i64>("enabled")? != 0,
        state: StabilizationState::from_str(&row.get::<_, String>("state")?),
        observation_started_at: row.get("observation_started_at")?,
        observation_low: row.get("observation_low")?,
        last_evaluated_at: row.get("last_evaluated_at")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

pub struct StabilizationRepo<'a>(pub &'a Connection);

impl StabilizationRepo<'_> {
    pub fn list(&self, enabled_only: bool) -> Result<Vec<StabilizationRule>, String> {
        let sql = if enabled_only {
            "SELECT * FROM stabilization_alerts WHERE enabled=1 ORDER BY created_at ASC"
        } else {
            "SELECT * FROM stabilization_alerts ORDER BY created_at ASC"
        };
        let mut stmt = self.0.prepare(sql).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], row_to_rule)
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
    }

    pub fn get(&self, id: &str) -> Result<Option<StabilizationRule>, String> {
        self.0
            .query_row(
                "SELECT * FROM stabilization_alerts WHERE id=?1",
                params![id],
                row_to_rule,
            )
            .optional()
            .map_err(|e| e.to_string())
    }

    pub fn insert(&self, rule: &StabilizationRule) -> Result<(), String> {
        self.0
            .execute(
                r#"INSERT INTO stabilization_alerts
                   (id, name, product_code, observation_price, stable_days, low_tolerance,
                    breakout_days, breakout_confirm_hours, trigger_mode, cooldown_minutes,
                    notify_stabilizing, notify_confirmed, enabled, state,
                    observation_started_at, observation_low, last_evaluated_at,
                    created_at, updated_at)
                   VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19)"#,
                params![
                    rule.id,
                    rule.name,
                    rule.product_code,
                    rule.observation_price,
                    rule.stable_days,
                    rule.low_tolerance,
                    rule.breakout_days,
                    rule.breakout_confirm_hours,
                    rule.trigger_mode.as_str(),
                    rule.cooldown_minutes,
                    rule.notify_stabilizing as i64,
                    rule.notify_confirmed as i64,
                    rule.enabled as i64,
                    rule.state.as_str(),
                    rule.observation_started_at,
                    rule.observation_low,
                    rule.last_evaluated_at,
                    rule.created_at,
                    rule.updated_at,
                ],
            )
            .map_err(|e| format!("保存企稳规则失败：{e}"))?;
        Ok(())
    }

    /// 编辑配置字段并重置状态（修改观察价格 → 原状态失效）
    pub fn update_config(&self, id: &str, input: &StabilizationInput) -> Result<(), String> {
        let now = Local::now().to_rfc3339();
        self.0
            .execute(
                r#"UPDATE stabilization_alerts SET
                     name=?2, product_code=?3, observation_price=?4, stable_days=?5,
                     low_tolerance=?6, breakout_days=?7, breakout_confirm_hours=?8,
                     trigger_mode=?9, cooldown_minutes=?10, notify_stabilizing=?11,
                     notify_confirmed=?12, enabled=1, state='WAITING_FOR_DIP',
                     observation_started_at=NULL, observation_low=NULL, updated_at=?13
                   WHERE id=?1"#,
                params![
                    id,
                    input.name,
                    input.product_code,
                    input.observation_price,
                    input.stable_days,
                    input.low_tolerance,
                    input.breakout_days,
                    input.breakout_confirm_hours,
                    input.trigger_mode.as_str(),
                    input.cooldown_minutes,
                    input.notify_stabilizing as i64,
                    input.notify_confirmed as i64,
                    now,
                ],
            )
            .map_err(|e| format!("更新企稳规则失败：{e}"))?;
        Ok(())
    }

    pub fn delete(&self, id: &str) -> Result<(), String> {
        self.0
            .execute("DELETE FROM stabilization_alerts WHERE id=?1", params![id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<(), String> {
        let now = Local::now().to_rfc3339();
        // 重新启用时重置状态，由引擎重新评估
        self.0
            .execute(
                "UPDATE stabilization_alerts SET enabled=?2, state=?3, observation_started_at=NULL,
                 observation_low=NULL, updated_at=?4 WHERE id=?1",
                params![
                    id,
                    enabled as i64,
                    if enabled {
                        StabilizationState::WaitingForDip.as_str()
                    } else {
                        StabilizationState::WaitingForDip.as_str()
                    },
                    now,
                ],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn reset_state(&self, id: &str) -> Result<(), String> {
        let now = Local::now().to_rfc3339();
        self.0
            .execute(
                "UPDATE stabilization_alerts SET state='WAITING_FOR_DIP',
                 observation_started_at=NULL, observation_low=NULL, updated_at=?2 WHERE id=?1",
                params![id, now],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn update_state(
        &self,
        id: &str,
        state: &StabilizationState,
        started_at: Option<&str>,
        low: Option<f64>,
        last_evaluated: &str,
    ) -> Result<(), String> {
        let now = Local::now().to_rfc3339();
        self.0
            .execute(
                "UPDATE stabilization_alerts SET state=?2, observation_started_at=?3,
                 observation_low=?4, last_evaluated_at=?5, updated_at=?6 WHERE id=?1",
                params![
                    id,
                    state.as_str(),
                    started_at,
                    low.map(|v| format!("{v:.2}")),
                    last_evaluated,
                    now,
                ],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// 同步 enabled 状态（ONCE 触发完成后禁用）
    pub fn set_disabled_if_once(&self, id: &str) -> Result<(), String> {
        let now = Local::now().to_rfc3339();
        self.0
            .execute(
                "UPDATE stabilization_alerts SET enabled=0, updated_at=?2
                 WHERE id=?1 AND trigger_mode='ONCE'",
                params![id, now],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn insert_event(&self, event: &StabilizationEvent) -> Result<(), String> {
        self.0
            .execute(
                r#"INSERT INTO stabilization_events
                   (id, alert_id, alert_name, previous_state, current_state,
                    reference_price, reason, event_time)
                   VALUES (?1,?2,?3,?4,?5,?6,?7,?8)"#,
                params![
                    event.id,
                    event.alert_id,
                    event.alert_name,
                    event.previous_state,
                    event.current_state,
                    event.reference_price,
                    event.reason,
                    event.event_time,
                ],
            )
            .map_err(|e| format!("保存企稳事件失败：{e}"))?;
        Ok(())
    }

    pub fn list_events(
        &self,
        alert_id: Option<&str>,
        limit: i64,
    ) -> Result<Vec<StabilizationEvent>, String> {
        let mut stmt = self
            .0
            .prepare(
                "SELECT id, alert_id, alert_name, previous_state, current_state,
                        reference_price, reason, event_time
                 FROM stabilization_events
                 WHERE (?1 IS NULL OR alert_id=?1)
                 ORDER BY event_time DESC LIMIT ?2",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![alert_id, limit.clamp(1, 500)], |row| {
                Ok(StabilizationEvent {
                    id: row.get(0)?,
                    alert_id: row.get(1)?,
                    alert_name: row.get(2)?,
                    previous_state: row.get(3)?,
                    current_state: row.get(4)?,
                    reference_price: row.get(5)?,
                    reason: row.get(6)?,
                    event_time: row.get(7)?,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
    }

    /// 最近一次同状态事件时间（通知冷却去重用）
    pub fn last_event_time_of_state(
        &self,
        alert_id: &str,
        state: &str,
    ) -> Result<Option<String>, String> {
        self.0
            .query_row(
                "SELECT event_time FROM stabilization_events
                 WHERE alert_id=?1 AND current_state=?2
                 ORDER BY event_time DESC LIMIT 1",
                params![alert_id, state],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|e| e.to_string())
    }
}
