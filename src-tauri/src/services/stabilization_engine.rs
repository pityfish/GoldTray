use std::collections::{BTreeMap, HashSet};

use chrono::{DateTime, Local, NaiveDate};

use crate::models::historical_quote::HistoricalPricePoint;
use crate::models::stabilization::{
    StabilizationAssessment, StabilizationRule, StabilizationState,
};

/// 单个有效交易日聚合结果
#[derive(Debug, Clone)]
pub struct DayStat {
    pub date: String,
    pub day_min: f64,
    pub day_max: f64,
    /// 日内振幅（最高 - 最低）
    pub range: f64,
}

/// 单个有效小时窗口聚合结果
#[derive(Debug, Clone)]
pub struct HourBucket {
    pub avg: f64,
}

#[derive(Debug)]
pub struct EventDraft {
    pub previous: Option<StabilizationState>,
    pub current: StabilizationState,
    pub reference_price: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NotifyKind {
    Stabilizing,
    Confirmed,
}

#[derive(Debug)]
pub struct EvalOutcome {
    pub state: StabilizationState,
    pub started_at: Option<String>,
    pub low: Option<f64>,
    pub events: Vec<EventDraft>,
    pub notify: Vec<NotifyKind>,
}

impl EvalOutcome {
    /// 以规则当前状态为基线构造（保持状态不变时直接返回）
    fn baseline(rule: &StabilizationRule) -> Self {
        Self {
            state: rule.state.clone(),
            started_at: rule.observation_started_at.clone(),
            low: rule
                .observation_low
                .as_deref()
                .and_then(|v| v.trim().parse().ok()),
            events: Vec::new(),
            notify: Vec::new(),
        }
    }

    fn keep(rule: &StabilizationRule, low: Option<f64>) -> Self {
        let mut out = Self::baseline(rule);
        if let Some(v) = low {
            out.low = Some(v);
        }
        out
    }
}

/// 单日最少有效数据点数：低于该值视为非有效交易日（缺失/休市时段）
const MIN_DAY_POINTS: usize = 6;

/// 聚合有效交易日：排除未完成交易日（今天）、数据点过少、全天重复报价（休市）的日期。
/// 返回按日期升序排列的有效交易日列表。
pub fn aggregate_valid_days(
    points: &[HistoricalPricePoint],
    today: &str,
) -> Vec<DayStat> {
    let mut groups: BTreeMap<String, Vec<&HistoricalPricePoint>> = BTreeMap::new();
    for p in points {
        if p.quality != "VALID" || p.timestamp.len() < 10 {
            continue;
        }
        groups
            .entry(p.timestamp[..10].to_string())
            .or_default()
            .push(p);
    }

    let mut out = Vec::new();
    for (date, pts) in groups {
        if date == today {
            continue; // 未完成交易日不参与"连续交易日"计数
        }
        if pts.len() < MIN_DAY_POINTS {
            continue; // 数据点过少
        }
        let avgs: Vec<f64> = pts.iter().filter_map(|p| p.avg_price).collect();
        let distinct: HashSet<u64> = avgs.iter().map(|v| v.to_bits()).collect();
        if distinct.len() < 2 {
            continue; // 长时间重复报价（休市）不得视为有效交易日
        }
        let day_min = pts
            .iter()
            .filter_map(|p| p.min_price)
            .fold(f64::INFINITY, f64::min);
        let day_max = pts
            .iter()
            .filter_map(|p| p.max_price)
            .fold(f64::NEG_INFINITY, f64::max);
        if !day_min.is_finite() || !day_max.is_finite() || day_min <= 0.0 {
            continue;
        }
        out.push(DayStat {
            date,
            day_min,
            day_max,
            range: day_max - day_min,
        });
    }
    out
}

/// 聚合有效小时窗口（48H 数据为 10 分钟粒度，按小时分桶）。
/// 桶内价格完全重复（休市/无更新）的窗口视为无效。
pub fn aggregate_valid_hours(points_48h: &[HistoricalPricePoint]) -> Vec<HourBucket> {
    let mut groups: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for p in points_48h {
        if p.quality != "VALID" || p.timestamp.len() < 13 {
            continue;
        }
        if let Some(avg) = p.avg_price {
            groups
                .entry(p.timestamp[..13].to_string())
                .or_default()
                .push(avg);
        }
    }
    let mut out = Vec::new();
    for (_hour, vals) in groups {
        let distinct: HashSet<u64> = vals.iter().map(|v| v.to_bits()).collect();
        if distinct.len() < 2 || vals.is_empty() {
            continue;
        }
        let avg = vals.iter().sum::<f64>() / vals.len() as f64;
        out.push(HourBucket { avg });
    }
    out
}

fn days_since(date_str: &str, now: &DateTime<Local>) -> Option<i64> {
    let d = NaiveDate::parse_from_str(date_str, "%Y-%m-%d").ok()?;
    Some((now.date_naive() - d).num_days())
}

/// 对一条规则执行一次状态机评估。
/// 仅在状态变更时生成事件；价格参考为实时客户买入参考价。
pub fn evaluate(
    rule: &StabilizationRule,
    latest: f64,
    bars14d: &[HistoricalPricePoint],
    bars48h: &[HistoricalPricePoint],
    now: &DateTime<Local>,
) -> EvalOutcome {
    let Ok(observation) = rule.observation_price.trim().parse::<f64>() else {
        return EvalOutcome::keep(rule, None);
    };
    let Ok(tolerance) = rule.low_tolerance.trim().parse::<f64>() else {
        return EvalOutcome::keep(rule, None);
    };
    let today = now.format("%Y-%m-%d").to_string();
    let days = aggregate_valid_days(bars14d, &today);
    let hours = aggregate_valid_hours(bars48h);
    let prev = rule.state.clone();
    let now_s = now.to_rfc3339();
    let push_event = |out: &mut EvalOutcome,
                          current: StabilizationState,
                          reason: Option<String>,
                          reference: Option<String>| {
        out.events.push(EventDraft {
            previous: Some(prev.clone()),
            current,
            reason,
            reference_price: reference,
        });
    };

    let mut out = EvalOutcome::baseline(rule);

    match rule.state {
        // 等待触达 / 失效后重入 / 数据恢复
        StabilizationState::WaitingForDip
        | StabilizationState::Invalidated
        | StabilizationState::InsufficientData => {
            if latest <= observation {
                out.state = StabilizationState::Observing;
                out.started_at = Some(now_s.clone());
                out.low = Some(latest);
                let reason = if prev == StabilizationState::Invalidated {
                    Some("重新进入观察区，观察重新开始".to_string())
                } else {
                    Some(format!("价格 {latest:.2} 达到观察价 {observation:.2}"))
                };
                push_event(&mut out, StabilizationState::Observing, reason, Some(format!("{latest:.2}")));
            } else if prev != StabilizationState::WaitingForDip {
                out.state = StabilizationState::WaitingForDip;
                out.started_at = None;
                out.low = None;
                push_event(
                    &mut out,
                    StabilizationState::WaitingForDip,
                    Some("价格回到观察区上方，等待再次触达".to_string()),
                    None,
                );
            } else {
                return EvalOutcome::keep(rule, None);
            }
        }

        // 观察中：跟踪低点，评估初步企稳
        StabilizationState::Observing => {
            let low = out.low.map(|v| v.min(latest)).unwrap_or(latest);
            out.low = Some(low);

            // 数据异常降级：历史数据长期缺失（最近有效日距今超过 3 天）
            let last_day_age = days.last().and_then(|d| days_since(&d.date, now));
            if bars14d.is_empty() || matches!(last_day_age, Some(age) if age > 3) {
                out.state = StabilizationState::InsufficientData;
                out.started_at = rule.observation_started_at.clone();
                push_event(
                    &mut out,
                    StabilizationState::InsufficientData,
                    Some("历史行情数据缺失，无法确认有效交易日".to_string()),
                    None,
                );
                return out;
            }

            // 观察开始之后出现的完整有效交易日
            let started_date = rule
                .observation_started_at
                .as_deref()
                .and_then(|s| s.get(..10).map(|s| s.to_string()));
            let after_start: Vec<&DayStat> = days
                .iter()
                .filter(|d| started_date.as_deref().map(|sd| d.date.as_str() > sd).unwrap_or(false))
                .collect();

            let stable_days = rule.stable_days.max(1) as usize;
            // 条件A：最近 stable_days 个有效交易日未明显创新低
            let low_stable = after_start.len() >= stable_days
                && after_start
                    .iter()
                    .rev()
                    .take(stable_days)
                    .all(|d| d.day_min >= low - tolerance);
            // 条件B：下跌波动收窄（最近两个有效交易日振幅递减）
            let narrowing = after_start.len() >= 2 && {
                let last = after_start[after_start.len() - 1].range;
                let prev_day = after_start[after_start.len() - 2].range;
                last < prev_day
            };

            if low_stable && narrowing {
                out.state = StabilizationState::Stabilizing;
                out.started_at = rule.observation_started_at.clone();
                push_event(
                    &mut out,
                    StabilizationState::Stabilizing,
                    Some(format!(
                        "连续 {stable_days} 个有效交易日未明显创新低，且波动收窄"
                    )),
                    Some(format!("{low:.2}")),
                );
                if rule.notify_stabilizing {
                    out.notify.push(NotifyKind::Stabilizing);
                }
            } else if latest > observation * 1.02 {
                // 价格明显离开观察区（未企稳即反弹超过 2%）→ 结束本轮观察
                out.state = StabilizationState::WaitingForDip;
                out.started_at = None;
                out.low = None;
                push_event(
                    &mut out,
                    StabilizationState::WaitingForDip,
                    Some("价格明显离开观察区，本轮观察结束".to_string()),
                    None,
                );
            } else {
                return EvalOutcome::keep(rule, Some(low));
            }
        }

        // 初步企稳：跟踪失效与反弹突破
        StabilizationState::Stabilizing => {
            // 失效判断必须基于更新前的历史最低（否则新低被吸收后条件永不满足）
            let prev_low = out.low;
            let low = prev_low.map(|v| v.min(latest)).unwrap_or(latest);
            out.low = Some(low);

            if let Some(pl) = prev_low {
                if latest < pl - tolerance {
                    // 再次明显创新低 → 信号失效，重新观察
                    push_event(
                        &mut out,
                        StabilizationState::Invalidated,
                        Some(format!("再次明显创新低（{latest:.2}），撤销企稳信号")),
                        Some(format!("{latest:.2}")),
                    );
                    out.state = StabilizationState::Observing;
                    out.started_at = Some(now_s.clone());
                    out.low = Some(latest);
                    push_event(
                        &mut out,
                        StabilizationState::Observing,
                        Some("失效后重新开始观察".to_string()),
                        Some(format!("{latest:.2}")),
                    );
                    return out;
                }
            }

            // 突破参考：最近 breakout_days 个有效交易日的高点
            let breakout_days = rule.breakout_days.max(1) as usize;
            let breakout = days
                .iter()
                .rev()
                .take(breakout_days)
                .map(|d| d.day_max)
                .fold(f64::NEG_INFINITY, f64::max);
            if !breakout.is_finite() {
                return EvalOutcome::keep(rule, Some(low));
            }

            if latest >= breakout {
                // 反弹确认：最近 confirm_hours 个有效小时窗口全部保持在突破价上方
                let confirm_hours = rule.breakout_confirm_hours.max(1) as usize;
                let recent: Vec<&HourBucket> = hours.iter().rev().take(confirm_hours).collect();
                let confirmed = recent.len() == confirm_hours
                    && recent.iter().all(|b| b.avg >= breakout);
                if confirmed {
                    out.state = StabilizationState::Confirmed;
                    out.started_at = rule.observation_started_at.clone();
                    push_event(
                        &mut out,
                        StabilizationState::Confirmed,
                        Some(format!(
                            "突破近 {breakout_days} 个有效交易日高点 {breakout:.2}，并连续 {confirm_hours} 个有效小时窗口保持"
                        )),
                        Some(format!("{latest:.2}")),
                    );
                    if rule.notify_confirmed {
                        out.notify.push(NotifyKind::Confirmed);
                    }
                } else {
                    // 盘中瞬时冲高不确认
                    return EvalOutcome::keep(rule, Some(low));
                }
            } else {
                return EvalOutcome::keep(rule, Some(low));
            }
        }

        // 反弹确认后：跟踪再次破位
        StabilizationState::Confirmed => {
            // 失效判断基于更新前的历史最低
            let prev_low = out.low;
            let low = prev_low.map(|v| v.min(latest)).unwrap_or(latest);
            out.low = Some(low);
            if let Some(pl) = prev_low {
                if latest < pl - tolerance {
                    push_event(
                        &mut out,
                        StabilizationState::Invalidated,
                        Some(format!("反弹确认后再次明显创新低（{latest:.2}）")),
                        Some(format!("{latest:.2}")),
                    );
                    out.state = StabilizationState::Observing;
                    out.started_at = Some(now_s.clone());
                    out.low = Some(latest);
                    push_event(
                        &mut out,
                        StabilizationState::Observing,
                        Some("失效后重新开始观察".to_string()),
                        Some(format!("{latest:.2}")),
                    );
                }
            }
            return out;
        }
    }
    out
}

/// 生成前端展示的判断依据
pub fn assess(
    rule: &StabilizationRule,
    latest: f64,
    bars14d: &[HistoricalPricePoint],
    _bars48h: &[HistoricalPricePoint],
    now: &DateTime<Local>,
) -> StabilizationAssessment {
    let tolerance = rule.low_tolerance.trim().parse::<f64>().unwrap_or(0.0);
    let low = rule
        .observation_low
        .as_deref()
        .and_then(|v| v.trim().parse::<f64>().ok());
    let today = now.format("%Y-%m-%d").to_string();
    let days = aggregate_valid_days(bars14d, &today);

    let started_date = rule
        .observation_started_at
        .as_deref()
        .and_then(|s| s.get(..10).map(|s| s.to_string()));
    let after_start: Vec<&DayStat> = days
        .iter()
        .filter(|d| started_date.as_deref().map(|sd| d.date.as_str() > sd).unwrap_or(false))
        .collect();
    let stable_days = rule.stable_days.max(1) as usize;
    let low_stable = low.is_some()
        && after_start.len() >= stable_days
        && after_start
            .iter()
            .rev()
            .take(stable_days)
            .all(|d| d.day_min >= low.unwrap() - tolerance);
    let narrowing =
        after_start.len() >= 2 && after_start[after_start.len() - 1].range < after_start[after_start.len() - 2].range;
    let breakout = days
        .iter()
        .rev()
        .take(rule.breakout_days.max(1) as usize)
        .map(|d| d.day_max)
        .fold(f64::NEG_INFINITY, f64::max);
    let breakout_finite = breakout.is_finite();
    let breakout_met = breakout_finite && latest >= breakout;

    StabilizationAssessment {
        state: rule.state.as_str().to_string(),
        observation_price: rule.observation_price.clone(),
        observation_low: low.map(|v| format!("{v:.2}")),
        valid_day_count: after_start.len(),
        low_stable,
        narrowing,
        breakout_price: if breakout_finite {
            Some(format!("{breakout:.2}"))
        } else {
            None
        },
        breakout_met,
        data_sufficient: !days.is_empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(
        state: StabilizationState,
        observation: &str,
        low: Option<&str>,
        started: Option<&str>,
    ) -> StabilizationRule {
        StabilizationRule {
            id: "t1".to_string(),
            name: "测试规则".to_string(),
            product_code: "GOLD_DEMAND".to_string(),
            observation_price: observation.to_string(),
            stable_days: 2,
            low_tolerance: "2.00".to_string(),
            breakout_days: 3,
            breakout_confirm_hours: 2,
            trigger_mode: crate::models::AlertTriggerMode::Rearmable,
            cooldown_minutes: 60,
            notify_stabilizing: true,
            notify_confirmed: true,
            enabled: true,
            state,
            observation_started_at: started.map(|s| s.to_string()),
            observation_low: low.map(|s| s.to_string()),
            last_evaluated_at: None,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    /// 生成小时粒度数据：每天 24 点，价格为 [min, max] 区间内平滑波动
    /// 生成截至昨天的小时数据（不含今天，避免被"未完成交易日"排除）
    fn make_past_bars(
        days: usize,
        day_ranges: &[(f64, f64)],
    ) -> Vec<HistoricalPricePoint> {
        let today = Local::now().date_naive();
        let mut pts = Vec::new();
        for d in 0..days {
            let (lo, hi) = day_ranges[d % day_ranges.len()];
            for h in 0..24 {
                let ts = (today - chrono::Duration::days((days - d) as i64))
                    .format("%Y-%m-%d")
                    .to_string();
                let frac = h as f64 / 23.0;
                let price = lo + (hi - lo) * frac;
                pts.push(HistoricalPricePoint {
                    timestamp: format!("{ts} {h:02}:00"),
                    min_price: Some(price - 0.3),
                    max_price: Some(price + 0.3),
                    avg_price: Some(price),
                    quality: "VALID",
                });
            }
        }
        pts
    }

    fn now() -> DateTime<Local> {
        Local::now()
    }

    #[test]
    fn entering_zone_starts_observing() {
        let r = rule(StabilizationState::WaitingForDip, "835.00", None, None);
        let out = evaluate(&r, 834.80, &[], &[], &now());
        assert_eq!(out.state, StabilizationState::Observing);
        assert_eq!(out.events.len(), 1);
        assert!(out.low.unwrap() <= 835.0);
    }

    #[test]
    fn no_stabilize_while_still_falling() {
        // 观察后连续下跌：每日最低不断走低 → 不应企稳
        let started = (Local::now().date_naive() - chrono::Duration::days(3))
            .format("%Y-%m-%dT00:00:00+08:00")
            .to_string();
        let r = rule(
            StabilizationState::Observing,
            "840.00",
            Some("835.00"),
            Some(&started),
        );
        // 最近 3 天：day_min 递减
        let bars = make_past_bars(3,
            &[(838.0, 839.0), (836.0, 837.5), (833.5, 836.0)],
        );
        let out = evaluate(&r, 834.0, &bars, &[], &now());
        assert_eq!(out.state, StabilizationState::Observing);
        assert!(out.notify.is_empty());
    }

    #[test]
    fn stabilizing_when_low_stable_and_narrowing() {
        // 观察开始后 2 个完整有效日：低点未明显创新低（>= low-2），振幅递减
        let started = (Local::now().date_naive() - chrono::Duration::days(3))
            .format("%Y-%m-%dT00:00:00+08:00")
            .to_string();
        let r = rule(
            StabilizationState::Observing,
            "836.00",
            Some("833.00"),
            Some(&started),
        );
        // 日1：振幅 2；日2：振幅 1（收窄），最低点 832.2 >= 833-2=831
        let bars = make_past_bars(2,
            &[(832.2, 834.2), (832.5, 833.5)],
        );
        let out = evaluate(&r, 833.10, &bars, &[], &now());
        assert_eq!(out.state, StabilizationState::Stabilizing);
        assert!(out.notify.contains(&NotifyKind::Stabilizing));
        assert_eq!(out.events.len(), 1);
    }

    #[test]
    fn confirmed_after_breakout_with_hour_confirmation() {
        // 已处于 STABILIZING，突破近 3 日高点且最近 2 个有效小时窗口保持
        let started = (Local::now().date_naive() - chrono::Duration::days(5))
            .format("%Y-%m-%dT00:00:00+08:00")
            .to_string();
        let r = rule(
            StabilizationState::Stabilizing,
            "836.00",
            Some("833.00"),
            Some(&started),
        );
        // 近 3 个完整有效日高点最多 834.5
        let bars = make_past_bars(5,
            &[(833.0, 834.0), (833.2, 834.2), (833.0, 834.5), (833.1, 834.3), (833.2, 834.4)],
        );
        // 48H 数据：最近两个小时桶均在突破价上方，每桶 2 个变化价格点（否则被判为休市）
        let today = Local::now().format("%Y-%m-%d").to_string();
        let mut bars48 = Vec::new();
        for (h, prices) in [(10, [835.1, 835.3]), (11, [835.5, 835.7])] {
            for (m, price) in [(0, prices[0]), (10, prices[1])] {
                bars48.push(HistoricalPricePoint {
                    timestamp: format!("{today} {h:02}:{m:02}"),
                    min_price: Some(price - 0.2),
                    max_price: Some(price + 0.2),
                    avg_price: Some(price),
                    quality: "VALID",
                });
            }
        }
        let out = evaluate(&r, 835.8, &bars, &bars48, &now());
        assert_eq!(out.state, StabilizationState::Confirmed);
        assert!(out.notify.contains(&NotifyKind::Confirmed));
    }

    #[test]
    fn transient_spike_does_not_confirm() {
        // 仅最近 1 个小时窗口满足突破 → 盘中瞬时冲高不确认
        let started = (Local::now().date_naive() - chrono::Duration::days(5))
            .format("%Y-%m-%dT00:00:00+08:00")
            .to_string();
        let r = rule(
            StabilizationState::Stabilizing,
            "836.00",
            Some("833.00"),
            Some(&started),
        );
        let bars = make_past_bars(5, &[(833.0, 834.0), (833.2, 834.2), (833.0, 834.5), (833.1, 834.3), (833.2, 834.4)]);
        // 只有一个达标小时桶
        let bars48 = vec![HistoricalPricePoint {
            timestamp: format!("{} 11:00", Local::now().format("%Y-%m-%d")),
            min_price: Some(835.0),
            max_price: Some(835.8),
            avg_price: Some(835.4),
            quality: "VALID",
        }];
        let out = evaluate(&r, 835.6, &bars, &bars48, &now());
        assert_eq!(out.state, StabilizationState::Stabilizing);
        assert!(out.notify.is_empty());
    }

    #[test]
    fn invalidated_by_new_low_then_reobserve() {
        let started = (Local::now().date_naive() - chrono::Duration::days(2))
            .format("%Y-%m-%dT00:00:00+08:00")
            .to_string();
        let r = rule(
            StabilizationState::Stabilizing,
            "836.00",
            Some("833.00"),
            Some(&started),
        );
        // 831 < 833 - 2 → 明显创新低 → 失效并重新观察
        let out = evaluate(&r, 830.8, &[], &[], &now());
        assert_eq!(out.state, StabilizationState::Observing);
        assert_eq!(out.events.len(), 2); // Invalidated + Observing
        assert_eq!(
            out.events[0].current,
            StabilizationState::Invalidated
        );
        assert_eq!(out.low, Some(830.8));
    }

    #[test]
    fn repeated_price_days_are_not_valid_trading_days() {
        // 全天重复报价（休市）的天不进入有效交易日聚合
        let pts = vec![
            HistoricalPricePoint {
                timestamp: "2026-11-01 08:00".to_string(),
                min_price: Some(836.0),
                max_price: Some(836.0),
                avg_price: Some(836.0),
                quality: "VALID",
            },
            HistoricalPricePoint {
                timestamp: "2026-11-01 09:00".to_string(),
                min_price: Some(836.0),
                max_price: Some(836.0),
                avg_price: Some(836.0),
                quality: "VALID",
            },
        ];
        let days = aggregate_valid_days(&pts, "2026-11-02");
        assert!(days.is_empty());
    }

    #[test]
    fn insufficient_data_keeps_observing_without_confirm() {
        // 观察开始后没有任何完整有效交易日 → 不得企稳
        let started = (Local::now().date_naive() - chrono::Duration::days(0))
            .format("%Y-%m-%dT00:00:00+08:00")
            .to_string();
        let r = rule(
            StabilizationState::Observing,
            "836.00",
            Some("833.00"),
            Some(&started),
        );
        // 仅今天的数据（today 被排除）→ after_start 为空
        let today = Local::now().format("%Y-%m-%d").to_string();
        let bars = vec![HistoricalPricePoint {
            timestamp: format!("{today} 10:00"),
            min_price: Some(833.0),
            max_price: Some(834.0),
            avg_price: Some(833.5),
            quality: "VALID",
        }];
        let out = evaluate(&r, 833.5, &bars, &[], &now());
        assert_eq!(out.state, StabilizationState::Observing);
        assert!(out.notify.is_empty());
    }
}

// ============ 运行时集成（评估循环 + 系统通知） ============

use crate::models::stabilization::StabilizationEvent;
use crate::services::quote_service::AppState;
use crate::storage::stabilization_repository::StabilizationRepo;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

/// 对所有启用的企稳规则执行一次评估（在每次有效行情更新后调用）。
/// 状态变更写入事件表；按冷却时间控制系统通知；广播给前端刷新。
pub async fn evaluate_all(app: &AppHandle) {
    let state = app.state::<AppState>();
    let Some(quote) = state.latest_quote.lock().unwrap().clone() else {
        return;
    };
    let Ok(latest) = quote.customer_buy_price.trim().parse::<f64>() else {
        return;
    };

    // 读取评估所需数据（短锁复制）
    let (bars14, bars48, rules) = {
        let db = state.db.lock().unwrap();
        let bars14 = crate::storage::history_repository::load_bars(
            db.conn(),
            crate::models::historical_quote::HistoryPeriod::D14,
        )
        .map(|(p, _, _)| p)
        .unwrap_or_default();
        let bars48 = crate::storage::history_repository::load_bars(
            db.conn(),
            crate::models::historical_quote::HistoryPeriod::H48,
        )
        .map(|(p, _, _)| p)
        .unwrap_or_default();
        let rules = StabilizationRepo(db.conn()).list(true).unwrap_or_default();
        (bars14, bars48, rules)
    };

    let now = Local::now();
    let mut notifications: Vec<(String, String)> = Vec::new();
    let mut anything_changed = false;

    for rule in &rules {
        let outcome = evaluate(rule, latest, &bars14, &bars48, &now);

        // 持久化状态与事件
        let notify_kinds = outcome.notify.clone();
        if !outcome.events.is_empty() {
            anything_changed = true;
        }
        {
            let db = state.db.lock().unwrap();
            let repo = StabilizationRepo(db.conn());
            if let Err(e) = repo.update_state(
                &rule.id,
                &outcome.state,
                outcome.started_at.as_deref(),
                outcome.low,
                &now.to_rfc3339(),
            ) {
                eprintln!("[goldtray] 企稳状态保存失败：{e}");
            }
            for ev in &outcome.events {
                let record = StabilizationEvent {
                    id: uuid::Uuid::new_v4().to_string(),
                    alert_id: rule.id.clone(),
                    alert_name: rule.name.clone(),
                    previous_state: ev.previous.as_ref().map(|s| s.as_str().to_string()),
                    current_state: ev.current.as_str().to_string(),
                    reference_price: ev.reference_price.clone(),
                    reason: ev.reason.clone(),
                    event_time: now.to_rfc3339(),
                };
                if let Err(e) = repo.insert_event(&record) {
                    eprintln!("[goldtray] 企稳事件保存失败：{e}");
                }
            }
            // ONCE 模式：确认后自动停用
            if outcome.state == StabilizationState::Confirmed
                && rule.trigger_mode == crate::models::AlertTriggerMode::Once
            {
                let _ = repo.set_disabled_if_once(&rule.id);
            }
            // 通知冷却检查：同一状态在冷却期内不重复通知
            for kind in &notify_kinds {
                let state_str = match kind {
                    NotifyKind::Stabilizing => "STABILIZING",
                    NotifyKind::Confirmed => "CONFIRMED",
                };
                let in_cooldown = repo
                    .last_event_time_of_state(&rule.id, state_str)
                    .ok()
                    .flatten()
                    .and_then(|t| DateTime::parse_from_rfc3339(&t).ok())
                    .map(|t| {
                        (now.with_timezone(&t.timezone()) - t).num_minutes()
                            < rule.cooldown_minutes
                    })
                    .unwrap_or(false);
                if !in_cooldown {
                    notifications.push(build_notification(rule, *kind));
                }
            }
        }
    }

    if anything_changed {
        let _ = app.emit("stabilization-changed", ());
    }

    for (title, body) in notifications {
        if let Err(e) = app.notification().builder().title(title).body(body).show() {
            eprintln!("[goldtray] 企稳通知发送失败：{e}");
        }
    }
}

/// 生成企稳系统通知。仅提示参考信号，不含交易指令。
fn build_notification(rule: &StabilizationRule, kind: NotifyKind) -> (String, String) {
    match kind {
        NotifyKind::Stabilizing => (
            "GoldTray · 初步企稳".to_string(),
            format!(
                "招商银行黄金活期\n\n价格进入{}元观察区后，\n近期出现初步止跌信号。\n\n建议继续观察，尚未完成反弹确认。",
                rule.observation_price
            ),
        ),
        NotifyKind::Confirmed => (
            "GoldTray · 反弹确认".to_string(),
            format!(
                "招商银行黄金活期\n\n当前行情满足「{}」设置的短期\n反弹确认条件。\n\n可人工评估是否分批加仓。\n请核对招行App实际成交价格。",
                rule.name
            ),
        ),
    }
}
