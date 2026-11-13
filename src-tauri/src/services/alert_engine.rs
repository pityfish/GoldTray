use chrono::{DateTime, Local, Utc};
use rust_decimal::Decimal;

use super::recovery::{parse_time, ClosedBar};
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
    pub armed_changed: bool,
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

fn record_trigger(
    db: &Db,
    rule: &mut AlertRule,
    price: Decimal,
    triggered_at: String,
    historical_from: Option<String>,
    outcome: &mut EngineOutcome,
) -> Result<(), String> {
    let price_text = if historical_from.is_some() {
        format!("{price:.2}")
    } else {
        price.to_string()
    };
    let is_estimated = rule.price_type == AlertPriceType::CustomerSell;
    let record = AlertTriggerRecord {
        id: uuid::Uuid::new_v4().to_string(),
        alert_id: rule.id.clone(),
        alert_name: rule.name.clone(),
        direction: rule.direction.clone(),
        target_price: rule.target_price.clone(),
        triggered_price: price_text.clone(),
        is_estimated,
        triggered_at,
        historical_from,
        notification_status: "PENDING".to_string(),
    };
    rule.enabled = rule.trigger_mode != crate::models::AlertTriggerMode::Once;
    rule.armed = false;
    rule.confirm_hits = 0;
    rule.last_triggered_at = Some(record.triggered_at.clone());
    rule.last_triggered_price = Some(price_text.clone());
    db.insert_alert_history(&record)?;
    outcome.triggered.push((record, price_text, is_estimated));
    Ok(())
}

fn save_rule(db: &Db, rule: &AlertRule) -> Result<(), String> {
    db.update_alert_state(
        &rule.id,
        rule.armed,
        rule.confirm_hits,
        rule.last_triggered_at.as_deref(),
        rule.last_triggered_price.as_deref(),
        rule.enabled,
    )
}

fn cooldown_elapsed(rule: &AlertRule, at: DateTime<Utc>) -> bool {
    rule.last_triggered_at
        .as_deref()
        .and_then(parse_time)
        .map_or(true, |last| {
            (at - last).num_minutes() >= rule.cooldown_minutes
        })
}

pub const LAST_MONITORED_QUOTE: &str = "alert_last_monitored_quote";

pub enum AlertEvaluation<'a, 'bar> {
    Live,
    AwaitingHistory,
    Replay(&'a [ClosedBar<'bar>]),
}

/// 对一次新行情执行预警规则评估。
/// 行情失效（超过过期阈值）时直接跳过，不触发新提醒。
#[cfg(test)]
pub fn evaluate(
    db: &Db,
    quote: &crate::models::GoldQuote,
    settings: &EngineSettings,
) -> Result<EngineOutcome, String> {
    evaluate_with_recovery(db, quote, settings, AlertEvaluation::Live)
}

pub fn evaluate_with_recovery(
    db: &Db,
    quote: &crate::models::GoldQuote,
    settings: &EngineSettings,
    evaluation: AlertEvaluation<'_, '_>,
) -> Result<EngineOutcome, String> {
    let mut outcome = EngineOutcome {
        triggered: Vec::new(),
        armed_changed: false,
    };

    // 保留断网前的规则状态和游标，等历史补算完成后再处理实时报价。
    let recovery = match evaluation {
        AlertEvaluation::AwaitingHistory => return Ok(outcome),
        AlertEvaluation::Live => None,
        AlertEvaluation::Replay(bars) => Some(bars),
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
    let last_monitored = db
        .get_setting(LAST_MONITORED_QUOTE)?
        .as_deref()
        .and_then(parse_time);
    let rules = db.list_alerts()?;

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

        let originally_armed = rule.armed;
        let direction = rule.direction.clone();
        let in_zone = |price: Decimal| match direction {
            AlertDirection::Above => price >= target,
            AlertDirection::Below => price <= target,
        };
        if let (Some(bars), Some(updated)) = (recovery, parse_time(&rule.updated_at)) {
            let since = [
                Some(updated),
                last_monitored,
                rule.last_triggered_at.as_deref().and_then(parse_time),
            ]
            .into_iter()
            .flatten()
            .max()
            .unwrap_or(updated);
            let spread = if rule.price_type == AlertPriceType::CustomerSell {
                settings.estimated_spread.unwrap_or_default()
            } else {
                Decimal::ZERO
            };
            rule.confirm_hits = 0;
            for bar in bars.iter().filter(|bar| bar.end > since) {
                let (Some(min), Some(max)) = (
                    Decimal::from_f64_retain(bar.min)
                        .and_then(|p| p.round_dp(2).checked_sub(spread)),
                    Decimal::from_f64_retain(bar.max)
                        .and_then(|p| p.round_dp(2).checked_sub(spread)),
                ) else {
                    continue;
                };
                let (extreme, at) = match rule.direction {
                    AlertDirection::Above => (max, bar.max_at),
                    AlertDirection::Below => (min, bar.min_at),
                };
                let trigger_time = at.filter(|at| *at > since).unwrap_or(bar.end);
                // 同一时段内没有先后顺序。只有整个时段在安全侧才能确认重新激活。
                if !rule.armed {
                    rule.armed = bar.complete && bar.start >= since && !in_zone(extreme);
                } else if bar.includes_extreme(at, since)
                    && in_zone(extreme)
                    && cooldown_elapsed(&rule, trigger_time.to_utc())
                {
                    record_trigger(
                        db,
                        &mut rule,
                        extreme,
                        bar.end.to_rfc3339(),
                        Some(bar.start.to_rfc3339()),
                        &mut outcome,
                    )?;
                    if !rule.enabled {
                        break;
                    }
                }
            }
        }

        if rule.enabled {
            if !rule.armed {
                if !in_zone(price) {
                    rule.armed = true;
                    rule.confirm_hits = 0;
                }
            } else if in_zone(price) {
                rule.confirm_hits = (rule.confirm_hits + 1).min(REQUIRED_CONFIRMATIONS);
                if rule.confirm_hits >= REQUIRED_CONFIRMATIONS && cooldown_elapsed(&rule, fetched) {
                    record_trigger(
                        db,
                        &mut rule,
                        price,
                        fetched.with_timezone(&Local).to_rfc3339(),
                        None,
                        &mut outcome,
                    )?;
                }
            } else {
                rule.confirm_hits = 0;
            }
        }
        outcome.armed_changed |= originally_armed != rule.armed;
        save_rule(db, &rule)?;
    }
    db.set_setting(LAST_MONITORED_QUOTE, &quote.fetched_at)?;

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
    if let Some(from) = &record.historical_from {
        let start = parse_time(from)
            .map(|t| {
                t.with_timezone(&super::recovery::market_time())
                    .format("%m-%d %H:%M")
                    .to_string()
            })
            .unwrap_or_else(|| from.clone());
        return (
            "GoldTray · 离线价格预警补记".to_string(),
            format!("招商银行黄金活期 {price_kind}\n北京时间 {start} 起的历史时段内，参考价{dir_text} {} 元/克，时段极值 {} 元/克。\n当前监控参考价：{current_price} 元/克。", record.target_price, record.triggered_price),
        );
    }
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
    use crate::models::historical_quote::HistoricalPricePoint;
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
    fn delayed_history_takes_priority_over_live_once_alert() {
        let db = Db::open_in_memory().unwrap();
        let id = insert_rule(
            &db,
            "一次提醒",
            AlertDirection::Below,
            "835",
            AlertTriggerMode::Once,
        );
        db.conn()
            .execute(
                "UPDATE price_alerts SET updated_at='2026-11-12T22:00:00+08:00' WHERE id=?1",
                [&id],
            )
            .unwrap();
        let cursor = "2026-11-12T23:52:30+08:00";
        db.set_setting(LAST_MONITORED_QUOTE, cursor).unwrap();
        let mut quote = sample_quote("832", 0);
        let sample_time = "2026-11-13T00:08:00+08:00";
        quote.fetched_at = sample_time.into();
        let mut settings = fresh_settings();
        settings.staleness_threshold_secs = i64::MAX;

        // 两次实时报价已进入预警区间，但历史尚未恢复。
        for _ in 0..2 {
            let out =
                evaluate_with_recovery(&db, &quote, &settings, AlertEvaluation::AwaitingHistory)
                    .unwrap();
            assert!(out.triggered.is_empty());
        }
        let waiting = db.get_alert(&id).unwrap().unwrap();
        assert!(waiting.enabled && waiting.armed);
        assert_eq!(waiting.confirm_hits, 0);
        assert!(db.list_alert_history(10).unwrap().is_empty());
        assert_eq!(
            db.get_setting(LAST_MONITORED_QUOTE).unwrap().as_deref(),
            Some(cursor)
        );

        let before_gap = HistoricalPricePoint {
            timestamp: "2026-11-12 23:50".into(),
            min_price: Some(831.0),
            max_price: Some(842.0),
            avg_price: Some(839.0),
            quality: "VALID",
            min_time: Some("23:51".into()),
            max_time: Some("23:55".into()),
        };

        // 另一条规则补算后未找到离线触达，继续按两次实时报价确认。
        let live_id = insert_rule(
            &db,
            "仅实时触达",
            AlertDirection::Below,
            "833",
            AlertTriggerMode::Once,
        );
        db.conn()
            .execute(
                "UPDATE price_alerts SET updated_at='2026-11-12T22:00:00+08:00' WHERE id=?1",
                [&live_id],
            )
            .unwrap();
        let during_gap = HistoricalPricePoint {
            timestamp: "2026-11-13 00:00".into(),
            min_price: Some(834.0),
            min_time: Some("00:04".into()),
            max_time: None,
            ..before_gap.clone()
        };
        let points = [before_gap, during_gap];
        let bars =
            super::super::recovery::closed_bars(&[], &points, parse_time(sample_time).unwrap());
        let out =
            evaluate_with_recovery(&db, &quote, &settings, AlertEvaluation::Replay(&bars)).unwrap();
        let [record] = db.list_alert_history(10).unwrap().try_into().unwrap();
        let record: AlertTriggerRecord = record;
        assert_eq!(out.triggered.len(), 1);
        assert_eq!(
            record.historical_from.as_deref(),
            Some("2026-11-13T00:00:00+08:00")
        );
        assert_eq!(record.triggered_price, "834.00");
        assert!(!db.get_alert(&id).unwrap().unwrap().enabled);
        assert_eq!(db.get_alert(&live_id).unwrap().unwrap().confirm_hits, 1);
        assert_eq!(
            db.get_setting(LAST_MONITORED_QUOTE).unwrap().as_deref(),
            Some(sample_time)
        );
        quote.fetched_at = "2026-11-13T00:09:00+08:00".into();
        let out = evaluate(&db, &quote, &settings).unwrap();
        assert_eq!(out.triggered.len(), 1);
        assert_eq!(out.triggered[0].0.alert_id, live_id);
        assert!(out.triggered[0].0.historical_from.is_none());
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
