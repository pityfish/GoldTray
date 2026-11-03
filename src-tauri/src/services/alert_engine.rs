use chrono::{DateTime, Local, Utc};
use rust_decimal::Decimal;

use crate::models::{AlertDirection, AlertPriceType, AlertRule, AlertTriggerRecord};
use crate::storage::db::Db;

/// 引擎运行参数（来自用户设置）
#[derive(Debug, Clone)]
pub struct EngineSettings {
    /// 客户卖出价固定价差估算（元/克）。None 表示未启用估算。
    pub estimated_spread: Option<Decimal>,
    /// 行情过期阈值（秒），超过则暂停触发新提醒
    pub staleness_threshold_secs: i64,
}

/// 默认确认机制：连续两次有效行情满足条件才触发
const REQUIRED_CONFIRMATIONS: i64 = 2;

pub struct EngineOutcome {
    /// (触发记录, 触发时参考价, 是否估算)
    pub triggered: Vec<(AlertTriggerRecord, String, bool)>,
}

/// 判断规则创建/启用时价格是否已满足触发条件。
/// 若已满足，armed=false（不立即触发），等待离开并重新进入区间。
pub fn is_currently_in_zone(
    quote: &crate::models::GoldQuote,
    rule: &AlertRule,
    settings: &EngineSettings,
) -> Option<bool> {
    let price = monitor_price(quote, rule, settings)?;
    let target = rule.target_price.trim().parse::<Decimal>().ok()?;
    Some(match rule.direction {
        AlertDirection::Above => price >= target,
        AlertDirection::Below => price <= target,
    })
}

/// 计算规则实际监控价格。返回 (价格, 是否估算)。
/// 卖出价规则在未设置价差估算时返回 None（无法准确监控）。
fn monitor_price(
    quote: &crate::models::GoldQuote,
    rule: &AlertRule,
    settings: &EngineSettings,
) -> Option<Decimal> {
    match rule.price_type {
        AlertPriceType::CustomerBuy => quote.customer_buy_price.trim().parse::<Decimal>().ok(),
        AlertPriceType::CustomerSell => quote.estimated_sell_price(settings.estimated_spread?),
    }
}

fn parse_time(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

/// 对一次新行情执行预警规则评估。
/// 行情失效（超过过期阈值）时直接跳过，不触发新提醒。
pub fn evaluate(
    db: &Db,
    quote: &crate::models::GoldQuote,
    settings: &EngineSettings,
) -> Result<EngineOutcome, String> {
    let mut outcome = EngineOutcome {
        triggered: Vec::new(),
    };

    // 行情失效保护：过期行情不触发新提醒
    let fetched = parse_time(&quote.fetched_at).ok_or_else(|| "行情时间格式异常".to_string())?;
    let age_secs = (Utc::now() - fetched).num_seconds();
    if age_secs > settings.staleness_threshold_secs {
        return Ok(outcome);
    }

    let tx = db
        .conn()
        .unchecked_transaction()
        .map_err(|e| e.to_string())?;
    let rules = db.list_alerts()?;
    let now = Local::now().to_rfc3339();

    for mut rule in rules {
        if !rule.enabled || rule.product_code != quote.product_code {
            continue;
        }

        let Some(price) = monitor_price(quote, &rule, settings) else {
            continue; // 卖出价规则未启用估算，无法监控
        };
        let Ok(target) = rule.target_price.trim().parse::<Decimal>() else {
            continue;
        };

        let condition = match rule.direction {
            AlertDirection::Above => price >= target,
            AlertDirection::Below => price <= target,
        };

        if !rule.armed {
            // 已触发状态：价格回到安全侧后重新激活（可重复规则）
            if !condition {
                rule.armed = true;
                rule.confirm_hits = 0;
                db.update_alert_state(&rule.id, true, 0, None, None, rule.enabled)?;
            }
            continue;
        }

        if condition {
            rule.confirm_hits += 1;
            if rule.confirm_hits < REQUIRED_CONFIRMATIONS {
                db.update_alert_state(
                    &rule.id,
                    rule.armed,
                    rule.confirm_hits,
                    rule.last_triggered_at.as_deref(),
                    rule.last_triggered_price.as_deref(),
                    rule.enabled,
                )?;
                continue;
            }
        } else {
            rule.confirm_hits = 0;
            db.update_alert_state(
                &rule.id,
                rule.armed,
                0,
                rule.last_triggered_at.as_deref(),
                rule.last_triggered_price.as_deref(),
                rule.enabled,
            )?;
            continue;
        }

        // 冷却检查：冷却时间内禁止重复通知
        if let Some(last) = rule.last_triggered_at.as_deref().and_then(parse_time) {
            let elapsed = Utc::now() - last;
            if elapsed.num_minutes() < rule.cooldown_minutes {
                continue;
            }
        }

        let price_text = price.to_string();
        let is_estimated = rule.price_type == AlertPriceType::CustomerSell;
        let record = AlertTriggerRecord {
            id: uuid::Uuid::new_v4().to_string(),
            alert_id: rule.id.clone(),
            alert_name: rule.name.clone(),
            direction: rule.direction.clone(),
            target_price: rule.target_price.clone(),
            triggered_price: price_text.clone(),
            is_estimated,
            triggered_at: now.clone(),
            notification_status: "PENDING".to_string(),
        };

        let disable = rule.trigger_mode == crate::models::AlertTriggerMode::Once;
        db.update_alert_state(
            &rule.id,
            false, // 触发后离开待触发状态，等待价格回到安全侧
            0,
            Some(&record.triggered_at),
            Some(&price_text),
            if disable { false } else { rule.enabled },
        )?;

        db.insert_alert_history(&record)?;
        outcome.triggered.push((record, price_text, is_estimated));
    }

    tx.commit().map_err(|e| e.to_string())?;
    Ok(outcome)
}

/// 生成系统通知内容。不包含任何交易指令。
pub fn notification_body(record: &AlertTriggerRecord, current_price: &str) -> (String, String) {
    let dir_text = match record.direction {
        AlertDirection::Above => "上涨达到",
        AlertDirection::Below => "下跌达到",
    };
    let price_kind = if record.is_estimated {
        "按价差估算，客户卖出价相关监控"
    } else {
        "客户买入参考价"
    };
    let title = "GoldTray · 黄金价格提醒".to_string();
    let body = format!(
        "招商银行黄金活期 {price_kind}\n已{dir_text} {target} 元/克观察区间\n\n当前参考价：{current} 元/克\n\n建议打开银行 App 核对实际价格及市场趋势。",
        target = record.target_price,
        current = current_price
    );
    (title, body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{AlertTriggerMode, GoldQuote};
    use crate::storage::db::Db;

    fn sample_quote(buy_price: &str, age_secs: i64) -> GoldQuote {
        GoldQuote {
            source: "huilvsudi".to_string(),
            bank_code: "CMB".to_string(),
            bank_name: "招商银行".to_string(),
            product_code: "GOLD_DEMAND".to_string(),
            product_name: "黄金活期".to_string(),
            customer_buy_price: buy_price.to_string(),
            customer_sell_price: None,
            raw_change: None,
            currency: "CNY".to_string(),
            unit: "GRAM".to_string(),
            fetched_at: (Utc::now() - chrono::Duration::seconds(age_secs)).to_rfc3339(),
            quoted_at: None,
            status: "LIVE".to_string(),
            is_estimated: false,
        }
    }

    fn insert_rule(
        db: &Db,
        name: &str,
        direction: AlertDirection,
        target: &str,
        mode: AlertTriggerMode,
    ) -> String {
        let now = Local::now().to_rfc3339();
        let rule = AlertRule {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_string(),
            product_code: "GOLD_DEMAND".to_string(),
            direction: direction.clone(),
            price_type: AlertPriceType::CustomerBuy,
            target_price: target.to_string(),
            trigger_mode: mode,
            cooldown_minutes: 30,
            enabled: true,
            armed: true,
            confirm_hits: 0,
            last_triggered_at: None,
            last_triggered_price: None,
            created_at: now.clone(),
            updated_at: now,
        };
        db.insert_alert(&rule).unwrap();
        rule.id
    }

    fn settings(spread: Option<&str>) -> EngineSettings {
        EngineSettings {
            estimated_spread: spread.and_then(|s| s.parse::<Decimal>().ok()),
            staleness_threshold_secs: 300,
        }
    }

    fn fresh_settings() -> EngineSettings {
        settings(None)
    }

    #[test]
    fn crossing_requires_two_confirmations() {
        let db = Db::open_in_memory().unwrap();
        let id = insert_rule(
            &db,
            "加仓",
            AlertDirection::Below,
            "835.00",
            AlertTriggerMode::Rearmable,
        );

        // 价格远高于目标：安全侧，无触发
        let out = evaluate(&db, &sample_quote("838.00", 0), &fresh_settings()).unwrap();
        assert!(out.triggered.is_empty());

        // 第一次满足条件：仅计数，不触发
        let out = evaluate(&db, &sample_quote("834.00", 0), &fresh_settings()).unwrap();
        assert!(out.triggered.is_empty());
        assert_eq!(db.get_alert(&id).unwrap().unwrap().confirm_hits, 1);

        // 第二次满足条件：触发
        let out = evaluate(&db, &sample_quote("833.50", 0), &fresh_settings()).unwrap();
        assert_eq!(out.triggered.len(), 1);
        assert_eq!(out.triggered[0].0.triggered_price, "833.50");

        let rule = db.get_alert(&id).unwrap().unwrap();
        assert!(!rule.armed, "触发后应退出待触发状态");
        assert_eq!(rule.confirm_hits, 0);
        assert!(rule.last_triggered_at.is_some());
        assert_eq!(rule.last_triggered_price.as_deref(), Some("833.50"));
        assert!(rule.enabled, "REARMABLE 触发后仍保持启用");
    }

    #[test]
    fn once_mode_disables_after_trigger() {
        let db = Db::open_in_memory().unwrap();
        let id = insert_rule(
            &db,
            "一次性",
            AlertDirection::Below,
            "835.00",
            AlertTriggerMode::Once,
        );

        evaluate(&db, &sample_quote("834.00", 0), &fresh_settings()).unwrap();
        evaluate(&db, &sample_quote("834.00", 0), &fresh_settings()).unwrap();

        let rule = db.get_alert(&id).unwrap().unwrap();
        assert!(!rule.enabled, "ONCE 规则触发后应自动禁用");
    }

    #[test]
    fn rearm_after_price_leaves_zone() {
        let db = Db::open_in_memory().unwrap();
        let id = insert_rule(
            &db,
            "重复",
            AlertDirection::Below,
            "835.00",
            AlertTriggerMode::Rearmable,
        );

        // 触发
        evaluate(&db, &sample_quote("834.00", 0), &fresh_settings()).unwrap();
        evaluate(&db, &sample_quote("834.00", 0), &fresh_settings()).unwrap();
        assert!(!db.get_alert(&id).unwrap().unwrap().armed);

        // 价格仍在区间内：不重新激活
        evaluate(&db, &sample_quote("834.50", 0), &fresh_settings()).unwrap();
        assert!(!db.get_alert(&id).unwrap().unwrap().armed);

        // 价格回到安全侧：重新激活
        evaluate(&db, &sample_quote("836.00", 0), &fresh_settings()).unwrap();
        assert!(db.get_alert(&id).unwrap().unwrap().armed);
    }

    #[test]
    fn cooldown_blocks_repeat_notification() {
        let db = Db::open_in_memory().unwrap();
        let id = insert_rule(
            &db,
            "冷却",
            AlertDirection::Below,
            "835.00",
            AlertTriggerMode::Rearmable,
        );

        // 第一次触发（需两次有效行情确认）
        evaluate(&db, &sample_quote("834.00", 0), &fresh_settings()).unwrap();
        let out = evaluate(&db, &sample_quote("834.00", 0), &fresh_settings()).unwrap();
        assert_eq!(out.triggered.len(), 1, "首次穿越应触发");

        // 重新激活并模拟刚触发过的状态（冷却期内）
        let now = Local::now().to_rfc3339();
        db.update_alert_state(&id, true, 0, Some(&now), Some("834.00"), true)
            .unwrap();

        // 立即穿越：确认计数满足但处于冷却期，不触发
        let out = evaluate(&db, &sample_quote("833.00", 0), &fresh_settings()).unwrap();
        assert!(out.triggered.is_empty(), "冷却期内不应触发");
        let out = evaluate(&db, &sample_quote("833.00", 0), &fresh_settings()).unwrap();
        assert!(out.triggered.is_empty(), "冷却期内不应触发");
    }

    #[test]
    fn stale_quote_does_not_trigger() {
        let db = Db::open_in_memory().unwrap();
        insert_rule(
            &db,
            "过期保护",
            AlertDirection::Below,
            "900.00",
            AlertTriggerMode::Rearmable,
        );

        // 行情超过 300 秒阈值：即使价格满足条件也不触发
        let out = evaluate(&db, &sample_quote("800.00", 400), &fresh_settings()).unwrap();
        assert!(out.triggered.is_empty(), "过期行情不应触发提醒");
    }

    #[test]
    fn sell_price_uses_estimated_spread() {
        let db = Db::open_in_memory().unwrap();
        let now = Local::now().to_rfc3339();
        let mut rule = AlertRule {
            id: uuid::Uuid::new_v4().to_string(),
            name: "止盈估算".to_string(),
            product_code: "GOLD_DEMAND".to_string(),
            direction: AlertDirection::Above,
            price_type: AlertPriceType::CustomerSell,
            target_price: "925.00".to_string(),
            trigger_mode: AlertTriggerMode::Rearmable,
            cooldown_minutes: 30,
            enabled: true,
            armed: true,
            confirm_hits: 0,
            last_triggered_at: None,
            last_triggered_price: None,
            created_at: now.clone(),
            updated_at: now,
        };
        db.insert_alert(&rule).unwrap();

        // 买入参考价 930 − 价差 5 = 估算卖出价 925。
        let out = evaluate(&db, &sample_quote("930.00", 0), &settings(Some("5.00"))).unwrap();
        // 第一次仅计数
        assert!(out.triggered.is_empty());

        db.conn()
            .execute_batch(
                "CREATE TRIGGER reject_history BEFORE INSERT ON alert_history
                 BEGIN SELECT RAISE(ABORT, 'history write failed'); END;",
            )
            .unwrap();
        assert!(evaluate(&db, &sample_quote("930.00", 0), &settings(Some("5.00"))).is_err());
        let unchanged = db.get_alert(&rule.id).unwrap().unwrap();
        assert!(unchanged.armed);
        assert_eq!(unchanged.confirm_hits, 1);
        assert!(db.list_alert_history(10).unwrap().is_empty());
        db.conn()
            .execute_batch("DROP TRIGGER reject_history")
            .unwrap();

        let out = evaluate(&db, &sample_quote("930.00", 0), &settings(Some("5.00"))).unwrap();
        assert_eq!(out.triggered.len(), 1, "等效监控值应满足后触发");
        assert_eq!(out.triggered[0].0.triggered_price, "925.00");
        assert_eq!(
            db.list_alert_history(10).unwrap()[0].notification_status,
            "PENDING"
        );
        rule.direction = AlertDirection::Below;
        assert_eq!(
            super::is_currently_in_zone(&sample_quote("929.00", 0), &rule, &settings(Some("5.00")),),
            Some(true),
        );
        assert_eq!(out.triggered[0].2, true, "应标记为估算触发");

        // 未设置价差：无法监控，不触发
        db.update_alert_state(&rule.id, true, 0, None, None, true)
            .unwrap();
        rule.armed = true;
        let out = evaluate(&db, &sample_quote("800.00", 0), &fresh_settings()).unwrap();
        assert!(out.triggered.is_empty());
    }

    #[test]
    fn zone_check_matches_direction_semantics() {
        let db = Db::open_in_memory().unwrap();
        let now = Local::now().to_rfc3339();
        let rule = AlertRule {
            id: "t1".to_string(),
            name: "止盈".to_string(),
            product_code: "GOLD_DEMAND".to_string(),
            direction: AlertDirection::Above,
            price_type: AlertPriceType::CustomerBuy,
            target_price: "925.00".to_string(),
            trigger_mode: AlertTriggerMode::Once,
            cooldown_minutes: 30,
            enabled: true,
            armed: true,
            confirm_hits: 0,
            last_triggered_at: None,
            last_triggered_price: None,
            created_at: now.clone(),
            updated_at: now,
        };
        let s = fresh_settings();
        // 等于目标价：ABOVE 语义为"达到或高于"，应视为在区间内
        assert_eq!(
            is_currently_in_zone(&sample_quote("925.00", 0), &rule, &s),
            Some(true)
        );
        assert_eq!(
            is_currently_in_zone(&sample_quote("924.99", 0), &rule, &s),
            Some(false)
        );

        let below_rule = AlertRule {
            direction: AlertDirection::Below,
            ..rule
        };
        assert_eq!(
            is_currently_in_zone(&sample_quote("925.00", 0), &below_rule, &s),
            Some(true)
        );
    }
}
