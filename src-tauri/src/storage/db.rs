use std::path::Path;

use chrono::Local;
use rusqlite::{params, Connection, OptionalExtension};

use crate::models::{
    AlertDirection, AlertInput, AlertPriceType, AlertRule, AlertTriggerMode, AlertTriggerRecord,
};

pub struct Db(Connection);

fn parse_direction(s: &str) -> AlertDirection {
    if s == "ABOVE" {
        AlertDirection::Above
    } else {
        AlertDirection::Below
    }
}

fn parse_price_type(s: &str) -> AlertPriceType {
    if s == "CUSTOMER_SELL" {
        AlertPriceType::CustomerSell
    } else {
        AlertPriceType::CustomerBuy
    }
}

fn parse_trigger_mode(s: &str) -> AlertTriggerMode {
    if s == "REARMABLE" {
        AlertTriggerMode::Rearmable
    } else {
        AlertTriggerMode::Once
    }
}

fn row_to_alert(row: &rusqlite::Row) -> rusqlite::Result<AlertRule> {
    Ok(AlertRule {
        id: row.get("id")?,
        name: row.get("name")?,
        product_code: row.get("product_code")?,
        direction: parse_direction(&row.get::<_, String>("direction")?),
        price_type: parse_price_type(&row.get::<_, String>("price_type")?),
        target_price: row.get("target_price")?,
        trigger_mode: parse_trigger_mode(&row.get::<_, String>("trigger_mode")?),
        cooldown_minutes: row.get("cooldown_minutes")?,
        enabled: row.get::<_, i64>("enabled")? != 0,
        armed: row.get::<_, i64>("armed")? != 0,
        confirm_hits: row.get("confirm_hits")?,
        last_triggered_at: row.get("last_triggered_at")?,
        last_triggered_price: row.get("last_triggered_price")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

impl Db {
    pub fn open(path: &Path) -> Result<Self, String> {
        let conn = Connection::open(path).map_err(|e| format!("打开数据库失败：{e}"))?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|e| format!("设置 WAL 失败：{e}"))?;
        let db = Self(conn);
        db.migrate()?;
        Ok(db)
    }

    /// 内存数据库（测试用）
    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self, String> {
        let conn = Connection::open_in_memory().map_err(|e| format!("打开内存数据库失败：{e}"))?;
        let db = Self(conn);
        db.migrate()?;
        Ok(db)
    }

    /// 底层连接访问器（供独立 repository 模块使用）
    pub fn conn(&self) -> &Connection {
        &self.0
    }

    fn migrate(&self) -> Result<(), String> {
        self.0
            .pragma_update(None, "foreign_keys", true)
            .map_err(|e| format!("启用外键检查失败：{e}"))?;
        self.0
            .execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS price_alerts (
                    id TEXT PRIMARY KEY,
                    product_code TEXT NOT NULL,
                    name TEXT NOT NULL,
                    direction TEXT NOT NULL,
                    price_type TEXT NOT NULL,
                    target_price TEXT NOT NULL,
                    trigger_mode TEXT NOT NULL,
                    cooldown_minutes INTEGER NOT NULL DEFAULT 30,
                    enabled INTEGER NOT NULL DEFAULT 1,
                    armed INTEGER NOT NULL DEFAULT 1,
                    confirm_hits INTEGER NOT NULL DEFAULT 0,
                    last_triggered_at TEXT,
                    last_triggered_price TEXT,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );

                CREATE TABLE IF NOT EXISTS alert_history (
                    id TEXT PRIMARY KEY,
                    alert_id TEXT NOT NULL,
                    alert_name TEXT NOT NULL,
                    direction TEXT NOT NULL,
                    target_price TEXT NOT NULL,
                    triggered_price TEXT NOT NULL,
                    is_estimated INTEGER NOT NULL DEFAULT 0,
                    triggered_at TEXT NOT NULL,
                    notification_status TEXT NOT NULL DEFAULT 'SENT'
                );

                CREATE TABLE IF NOT EXISTS app_settings (
                    key TEXT PRIMARY KEY,
                    value TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );

                CREATE TABLE IF NOT EXISTS gold_history_bars (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    source TEXT NOT NULL,
                    product_code TEXT NOT NULL,
                    period TEXT NOT NULL,
                    timestamp TEXT NOT NULL,
                    min_price TEXT,
                    max_price TEXT,
                    avg_price TEXT,
                    quality TEXT NOT NULL DEFAULT 'VALID',
                    fetched_at TEXT NOT NULL,
                    UNIQUE(source, product_code, period, timestamp)
                );

                CREATE INDEX IF NOT EXISTS idx_gold_history_bars_time
                ON gold_history_bars(product_code, timestamp);

                CREATE TABLE IF NOT EXISTS stabilization_alerts (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    product_code TEXT NOT NULL,
                    observation_price TEXT NOT NULL,
                    stable_days INTEGER NOT NULL DEFAULT 2,
                    low_tolerance TEXT NOT NULL DEFAULT '2.00',
                    breakout_days INTEGER NOT NULL DEFAULT 3,
                    breakout_confirm_hours INTEGER NOT NULL DEFAULT 2,
                    trigger_mode TEXT NOT NULL DEFAULT 'ONCE',
                    cooldown_minutes INTEGER NOT NULL DEFAULT 60,
                    notify_stabilizing INTEGER NOT NULL DEFAULT 1,
                    notify_confirmed INTEGER NOT NULL DEFAULT 1,
                    enabled INTEGER NOT NULL DEFAULT 1,
                    state TEXT NOT NULL DEFAULT 'WAITING_FOR_DIP',
                    observation_started_at TEXT,
                    observation_low TEXT,
                    last_evaluated_at TEXT,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );

                CREATE TABLE IF NOT EXISTS stabilization_events (
                    id TEXT PRIMARY KEY,
                    alert_id TEXT NOT NULL,
                    alert_name TEXT NOT NULL,
                    previous_state TEXT,
                    current_state TEXT NOT NULL,
                    reference_price TEXT,
                    reason TEXT,
                    event_time TEXT NOT NULL
                );

                CREATE INDEX IF NOT EXISTS idx_stabilization_events_alert
                ON stabilization_events(alert_id, event_time);

                CREATE TABLE IF NOT EXISTS stabilization_notification_times (
                    alert_id TEXT NOT NULL REFERENCES stabilization_alerts(id) ON DELETE CASCADE,
                    state TEXT NOT NULL,
                    sent_at INTEGER NOT NULL,
                    PRIMARY KEY (alert_id, state)
                );
                "#,
            )
            .map_err(|e| format!("初始化数据库表失败：{e}"))?;
        self.0
            .execute(
                "UPDATE alert_history SET notification_status='UNKNOWN'
                 WHERE notification_status='PENDING'",
                [],
            )
            .map_err(|e| format!("恢复未完成的通知记录失败：{e}"))?;
        Ok(())
    }

    // ---------- 预警规则 ----------

    pub fn list_alerts(&self) -> Result<Vec<AlertRule>, String> {
        let mut stmt = self
            .0
            .prepare("SELECT * FROM price_alerts ORDER BY created_at ASC")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], row_to_alert)
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }

    pub fn get_alert(&self, id: &str) -> Result<Option<AlertRule>, String> {
        self.0
            .query_row(
                "SELECT * FROM price_alerts WHERE id = ?1",
                params![id],
                row_to_alert,
            )
            .optional()
            .map_err(|e| e.to_string())
    }

    pub fn insert_alert(&self, rule: &AlertRule) -> Result<(), String> {
        self.0
            .execute(
                r#"INSERT INTO price_alerts
                   (id, product_code, name, direction, price_type, target_price,
                    trigger_mode, cooldown_minutes, enabled, armed, confirm_hits,
                    last_triggered_at, last_triggered_price, created_at, updated_at)
                   VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)"#,
                params![
                    rule.id,
                    rule.product_code,
                    rule.name,
                    rule.direction.as_str(),
                    rule.price_type.as_str(),
                    rule.target_price,
                    rule.trigger_mode.as_str(),
                    rule.cooldown_minutes,
                    rule.enabled as i64,
                    rule.armed as i64,
                    rule.confirm_hits,
                    rule.last_triggered_at,
                    rule.last_triggered_price,
                    rule.created_at,
                    rule.updated_at,
                ],
            )
            .map_err(|e| format!("保存预警规则失败：{e}"))?;
        Ok(())
    }

    pub fn update_alert(&self, id: &str, input: &AlertInput) -> Result<(), String> {
        let now = Local::now().to_rfc3339();
        self.0
            .execute(
                r#"UPDATE price_alerts SET
                     name=?2, product_code=?3, direction=?4, price_type=?5,
                     target_price=?6, trigger_mode=?7, cooldown_minutes=?8,
                     armed=1, confirm_hits=0, updated_at=?9
                   WHERE id=?1"#,
                params![
                    id,
                    input.name,
                    input.product_code,
                    input.direction.as_str(),
                    input.price_type.as_str(),
                    input.target_price,
                    input.trigger_mode.as_str(),
                    input.cooldown_minutes,
                    now,
                ],
            )
            .map_err(|e| format!("更新预警规则失败：{e}"))?;
        Ok(())
    }

    pub fn delete_alert(&self, id: &str) -> Result<(), String> {
        self.0
            .execute("DELETE FROM price_alerts WHERE id=?1", params![id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn set_alert_enabled(&self, id: &str, enabled: bool) -> Result<(), String> {
        let now = Local::now().to_rfc3339();
        // 重新启用时重置为待触发状态，由引擎根据最新行情重新校准
        self.0
            .execute(
                "UPDATE price_alerts SET enabled=?2, armed=?3, confirm_hits=0, updated_at=?4 WHERE id=?1",
                params![id, enabled as i64, enabled as i64, now],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// 更新规则触发状态（由 AlertEngine 调用）
    #[allow(clippy::too_many_arguments)]
    pub fn update_alert_state(
        &self,
        id: &str,
        armed: bool,
        confirm_hits: i64,
        last_triggered_at: Option<&str>,
        last_triggered_price: Option<&str>,
        enabled: bool,
    ) -> Result<(), String> {
        let now = Local::now().to_rfc3339();
        self.0
            .execute(
                "UPDATE price_alerts SET armed=?2, confirm_hits=?3, last_triggered_at=?4, last_triggered_price=?5, enabled=?6, updated_at=?7 WHERE id=?1",
                params![id, armed as i64, confirm_hits, last_triggered_at, last_triggered_price, enabled as i64, now],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    // ---------- 触发记录 ----------

    pub fn insert_alert_history(&self, record: &AlertTriggerRecord) -> Result<(), String> {
        self.0
            .execute(
                r#"INSERT INTO alert_history
                   (id, alert_id, alert_name, direction, target_price,
                    triggered_price, is_estimated, triggered_at, notification_status)
                   VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)"#,
                params![
                    record.id,
                    record.alert_id,
                    record.alert_name,
                    record.direction.as_str(),
                    record.target_price,
                    record.triggered_price,
                    record.is_estimated as i64,
                    record.triggered_at,
                    record.notification_status,
                ],
            )
            .map_err(|e| format!("保存触发记录失败：{e}"))?;
        Ok(())
    }

    pub fn set_alert_notification_status(&self, id: &str, status: &str) -> Result<(), String> {
        self.0
            .execute(
                "UPDATE alert_history SET notification_status=?2 WHERE id=?1",
                params![id, status],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn list_alert_history(&self, limit: i64) -> Result<Vec<AlertTriggerRecord>, String> {
        let mut stmt = self
            .0
            .prepare("SELECT * FROM alert_history ORDER BY triggered_at DESC LIMIT ?1")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![limit], |row| {
                Ok(AlertTriggerRecord {
                    id: row.get("id")?,
                    alert_id: row.get("alert_id")?,
                    alert_name: row.get("alert_name")?,
                    direction: parse_direction(&row.get::<_, String>("direction")?),
                    target_price: row.get("target_price")?,
                    triggered_price: row.get("triggered_price")?,
                    is_estimated: row.get::<_, i64>("is_estimated")? != 0,
                    triggered_at: row.get("triggered_at")?,
                    notification_status: row.get("notification_status")?,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }

    /// 清空全部触发记录
    pub fn clear_alert_history(&self) -> Result<(), String> {
        self.0
            .execute("DELETE FROM alert_history", [])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    // ---------- 设置 ----------

    pub fn get_setting(&self, key: &str) -> Result<Option<String>, String> {
        self.0
            .query_row(
                "SELECT value FROM app_settings WHERE key=?1",
                params![key],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|e| e.to_string())
    }

    pub fn get_settings(&self, keys: &[String]) -> Result<Vec<(String, Option<String>)>, String> {
        keys.iter()
            .map(|k| Ok((k.clone(), self.get_setting(k)?)))
            .collect()
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), String> {
        let now = Local::now().to_rfc3339();
        self.0
            .execute(
                "INSERT INTO app_settings(key,value,updated_at) VALUES(?1,?2,?3)
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at=excluded.updated_at",
                params![key, value, now],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[cfg(test)]
mod recovery_tests {
    use super::Db;

    #[test]
    fn pending_notification_is_marked_unknown_on_reopen() {
        let path =
            std::env::temp_dir().join(format!("goldtray-recovery-{}.db", uuid::Uuid::new_v4()));
        {
            let db = Db::open(&path).unwrap();
            db.conn()
                .execute(
                    "INSERT INTO alert_history
                 (id, alert_id, alert_name, direction, target_price, triggered_price,
                  triggered_at, notification_status)
                 VALUES ('pending', 'rule', 'rule', 'ABOVE', '850', '851', 'now', 'PENDING')",
                    [],
                )
                .unwrap();
        }
        let db = Db::open(&path).unwrap();
        let records = db.list_alert_history(10).unwrap();
        assert_eq!(records[0].notification_status, "UNKNOWN");
        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }
}
