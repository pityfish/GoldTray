use std::collections::{BTreeMap, HashSet};

use chrono::{
    DateTime, Duration, FixedOffset, Local, NaiveDate, NaiveDateTime, TimeZone, Timelike,
};

use super::recovery::{market_time, parse_time, ClosedBar};
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
struct HourBucket {
    start: NaiveDateTime,
    avg: f64,
}

#[derive(Debug)]
pub struct EventDraft {
    pub previous: Option<StabilizationState>,
    pub current: StabilizationState,
    pub reference_price: Option<String>,
    pub reason: Option<String>,
    pub event_time: String,
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
pub fn aggregate_valid_days(points: &[HistoricalPricePoint], today: &str) -> Vec<DayStat> {
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

fn market_hour(now: &DateTime<impl TimeZone>) -> NaiveDateTime {
    let market_now = now.with_timezone(&market_time());
    market_now
        .date_naive()
        .and_hms_opt(market_now.hour(), 0, 0)
        .unwrap()
}

fn bar_time(point: &HistoricalPricePoint) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(&point.timestamp, "%Y-%m-%d %H:%M").ok()
}

/// 缓存在小时结束前取得的最后一个报价可能仍在变化。
pub fn completed_hour_points(
    points: &[HistoricalPricePoint],
    fetched_at: &str,
) -> Vec<HistoricalPricePoint> {
    let Some(cutoff) = parse_time(fetched_at).map(|time| market_hour(&time)) else {
        return Vec::new();
    };
    points
        .iter()
        .filter(|point| bar_time(point).is_some_and(|time| time < cutoff))
        .cloned()
        .collect()
}

/// 每个小时必须包含六个不同的 10 分钟数据点，且有价格变化。
fn aggregate_valid_hours(
    points_48h: &[HistoricalPricePoint],
    cutoff: NaiveDateTime,
) -> Vec<HourBucket> {
    let mut groups: BTreeMap<NaiveDateTime, [Option<f64>; 6]> = BTreeMap::new();
    for p in points_48h {
        if p.quality != "VALID" {
            continue;
        }
        let Some(time) = bar_time(p) else { continue };
        if time >= cutoff || time.minute() % 10 != 0 {
            continue;
        }
        let Some(avg) = p.avg_price.filter(|avg| avg.is_finite() && *avg > 0.0) else {
            continue;
        };
        let start = time.date().and_hms_opt(time.hour(), 0, 0).unwrap();
        groups.entry(start).or_default()[(time.minute() / 10) as usize] = Some(avg);
    }
    groups
        .into_iter()
        .filter_map(|(start, slots)| {
            let first = slots[0]?;
            if slots.iter().any(Option::is_none) || slots.iter().all(|price| *price == Some(first))
            {
                return None;
            }
            let avg = slots.into_iter().flatten().sum::<f64>() / slots.len() as f64;
            Some(HourBucket { start, avg })
        })
        .collect()
}

fn days_since(date_str: &str, now: &DateTime<impl TimeZone>) -> Option<i64> {
    let d = NaiveDate::parse_from_str(date_str, "%Y-%m-%d").ok()?;
    Some((now.date_naive() - d).num_days())
}

fn days_after_observation<'a>(rule: &StabilizationRule, days: &'a [DayStat]) -> &'a [DayStat] {
    let Some(start) = rule.observation_started_at.as_deref().and_then(parse_time) else {
        return &[];
    };
    let date = start.with_timezone(&market_time()).date_naive().to_string();
    &days[days.partition_point(|day| day.date <= date)..]
}

/// 优先处理已经完成的交易日，再检查当前报价。低点变化超过容差时从新低当天重新观察。
fn observation_reset(
    prior_low: Option<f64>,
    tolerance: f64,
    latest: f64,
    after_start: &[DayStat],
    now: &DateTime<impl TimeZone>,
) -> Option<(f64, String)> {
    let mut baseline = prior_low?;
    let mut reset = None;
    for day in after_start {
        if day.day_min < baseline - tolerance {
            baseline = day.day_min;
            reset = Some((baseline, format!("{}T00:00:00+08:00", day.date)));
        }
    }
    if latest < baseline - tolerance {
        return Some((latest, now.to_rfc3339()));
    }
    reset.map(|(low, started_at)| (low.min(latest), started_at))
}

struct BreakoutCheck {
    price: Option<f64>,
    price_met: bool,
    matching_hours: usize,
    required_hours: usize,
    data_sufficient: bool,
}

impl BreakoutCheck {
    fn confirmed(&self) -> bool {
        self.price_met && self.matching_hours == self.required_hours
    }
}

fn check_breakout(
    rule: &StabilizationRule,
    latest: f64,
    days: &[DayStat],
    hours: &[HourBucket],
    cutoff: NaiveDateTime,
) -> BreakoutCheck {
    let required_days = rule.breakout_days.max(1) as usize;
    let required_hours = rule.breakout_confirm_hours.max(1) as usize;
    let price = (days.len() >= required_days)
        .then(|| {
            days.iter()
                .rev()
                .take(required_days)
                .map(|day| day.day_max)
                .fold(f64::NEG_INFINITY, f64::max)
        })
        .filter(|price| price.is_finite());
    let consecutive = hours
        .iter()
        .rev()
        .take(required_hours)
        .enumerate()
        .take_while(|(index, hour)| hour.start == cutoff - Duration::hours(*index as i64 + 1))
        .map(|(_, hour)| hour);
    let matching_hours = price.map_or(0, |price| {
        consecutive
            .clone()
            .take_while(|hour| hour.avg >= price)
            .count()
    });
    BreakoutCheck {
        price,
        price_met: price.is_some_and(|price| latest >= price),
        matching_hours,
        required_hours,
        data_sufficient: price.is_some() && consecutive.count() == required_hours,
    }
}

/// 对一条规则执行一次状态机评估。
/// 仅在状态变更时生成事件；价格参考为实时客户买入参考价。
pub fn evaluate(
    rule: &StabilizationRule,
    latest: f64,
    bars14d: &[HistoricalPricePoint],
    bars48h: &[HistoricalPricePoint],
    now: &DateTime<impl TimeZone>,
) -> EvalOutcome {
    let Ok(observation) = rule.observation_price.trim().parse::<f64>() else {
        return EvalOutcome::keep(rule, None);
    };
    let Ok(tolerance) = rule.low_tolerance.trim().parse::<f64>() else {
        return EvalOutcome::keep(rule, None);
    };
    let today = now.date_naive().to_string();
    let days = aggregate_valid_days(bars14d, &today);
    let cutoff = market_hour(now);
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
            event_time: now_s.clone(),
        });
    };

    let mut out = EvalOutcome::baseline(rule);

    let state = if rule.state == StabilizationState::InsufficientData
        && rule.observation_started_at.is_some()
    {
        StabilizationState::Observing
    } else {
        rule.state.clone()
    };
    match state {
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
                push_event(
                    &mut out,
                    StabilizationState::Observing,
                    reason,
                    Some(format!("{latest:.2}")),
                );
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
            let prior_low = out.low;
            let after_start = days_after_observation(rule, &days);
            if let Some((low, started_at)) =
                observation_reset(prior_low, tolerance, latest, after_start, now)
            {
                out.state = StabilizationState::Observing;
                out.started_at = Some(started_at);
                out.low = Some(low);
                push_event(
                    &mut out,
                    StabilizationState::Observing,
                    Some(format!(
                        "价格明显创新低（{low:.2}），从新低重新累计有效交易日"
                    )),
                    Some(format!("{low:.2}")),
                );
                return out;
            }
            let low = prior_low.map(|v| v.min(latest)).unwrap_or(latest);
            out.low = Some(low);

            // 数据异常降级：历史数据长期缺失（最近有效日距今超过 3 天）
            let last_day_age = days.last().and_then(|d| days_since(&d.date, now));
            if bars14d.is_empty() || matches!(last_day_age, Some(age) if age > 3) {
                out.state = StabilizationState::InsufficientData;
                out.started_at = rule.observation_started_at.clone();
                if prev != StabilizationState::InsufficientData {
                    push_event(
                        &mut out,
                        StabilizationState::InsufficientData,
                        Some("历史行情数据缺失，无法确认有效交易日".to_string()),
                        None,
                    );
                }
                return out;
            }

            let stable_days = rule.stable_days.max(1) as usize;
            // 条件A：最近 stable_days 个有效交易日未明显创新低
            let low_stable = after_start.len() >= stable_days
                && after_start
                    .iter()
                    .rev()
                    .take(stable_days)
                    .all(|d| d.day_min >= prior_low.unwrap_or(low) - tolerance);
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
            } else if prev == StabilizationState::InsufficientData {
                out.state = StabilizationState::Observing;
                push_event(
                    &mut out,
                    StabilizationState::Observing,
                    Some("历史行情恢复，继续本轮观察".into()),
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

            let hours = aggregate_valid_hours(bars48h, cutoff);
            let check = check_breakout(rule, latest, &days, &hours, cutoff);
            if let Some(breakout) = check.price {
                if check.confirmed() {
                    let breakout_days = rule.breakout_days;
                    let confirm_hours = check.required_hours;
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

fn evaluate_with_recovery(
    rule: &StabilizationRule,
    latest: f64,
    bars14: &[HistoricalPricePoint],
    bars48: &[HistoricalPricePoint],
    now: &DateTime<FixedOffset>,
    recovery: Option<&[ClosedBar<'_>]>,
) -> EvalOutcome {
    let mut current = rule.clone();
    let mut events = Vec::new();
    let mut notify = Vec::new();
    let since = [
        rule.last_evaluated_at.as_deref().and_then(parse_time),
        parse_time(&rule.updated_at),
    ]
    .into_iter()
    .flatten()
    .max();
    if let (Some(bars), Some(since)) = (recovery, since) {
        for (i, bar) in bars
            .iter()
            .enumerate()
            .filter(|(_, bar)| bar.includes_extreme(bar.min_at, since))
        {
            // 每个时段只使用当时可见的完整交易日及细粒度小时窗口。
            let prior14: Vec<_> = bars14
                .iter()
                .filter(|p| {
                    p.timestamp
                        .get(..10)
                        .is_some_and(|d| d < &bar.point.timestamp[..10])
                })
                .cloned()
                .collect();
            let prior48: Vec<_> = bars[..=i]
                .iter()
                .filter(|p| p.complete && (p.end - p.start).num_minutes() == 10)
                .map(|p| p.point.clone())
                .collect();
            let mut step = evaluate(&current, bar.min, &prior14, &prior48, &bar.end);
            // 回放窗口起点之前的数据可能已不在接口保留范围内，不能据此补记数据中断。
            if step.state == StabilizationState::InsufficientData
                && current.state == StabilizationState::Observing
            {
                step = EvalOutcome::keep(&current, step.low);
            }
            // 低点用于触达和失效判断。反弹使用均价，仍须满足有效小时条件。
            if bar.complete
                && step.events.is_empty()
                && current.state == StabilizationState::Stabilizing
            {
                if let Some(avg) = bar.avg {
                    let rebound = evaluate(&current, avg, &prior14, &prior48, &bar.end);
                    if rebound.state == StabilizationState::Confirmed {
                        step.state = rebound.state;
                        step.events = rebound.events;
                        step.notify = rebound.notify;
                    }
                }
            }
            current.state = step.state.clone();
            current.observation_started_at = step.started_at.clone();
            current.observation_low = step.low.map(|v| format!("{v:.2}"));
            events.extend(step.events.drain(..).map(|mut event| {
                event.reason = event.reason.map(|reason| format!("离线行情补记：{reason}"));
                event
            }));
            for kind in step.notify.drain(..) {
                if !notify.contains(&kind) {
                    notify.push(kind);
                }
            }
            if current.state == StabilizationState::Confirmed
                && rule.trigger_mode == crate::models::AlertTriggerMode::Once
            {
                step.events = events;
                step.notify = notify;
                return step;
            }
        }
    }
    let mut outcome = evaluate(&current, latest, bars14, bars48, now);
    events.append(&mut outcome.events);
    for kind in outcome.notify {
        if !notify.contains(&kind) {
            notify.push(kind);
        }
    }
    outcome.events = events;
    outcome.notify = notify;
    outcome
}

/// 生成前端展示的判断依据
pub fn assess(
    rule: &StabilizationRule,
    latest: f64,
    bars14d: &[HistoricalPricePoint],
    bars48h: &[HistoricalPricePoint],
    now: &DateTime<impl TimeZone>,
) -> StabilizationAssessment {
    let tolerance = rule.low_tolerance.trim().parse::<f64>().unwrap_or(0.0);
    let low = rule
        .observation_low
        .as_deref()
        .and_then(|v| v.trim().parse::<f64>().ok());
    let today = now.date_naive().to_string();
    let days = aggregate_valid_days(bars14d, &today);

    let after_start = days_after_observation(rule, &days);
    let stable_days = rule.stable_days.max(1) as usize;
    let low_stable = low.is_some()
        && observation_reset(low, tolerance, latest, after_start, now).is_none()
        && after_start.len() >= stable_days
        && after_start
            .iter()
            .rev()
            .take(stable_days)
            .all(|d| d.day_min >= low.unwrap() - tolerance);
    let narrowing = after_start.len() >= 2
        && after_start[after_start.len() - 1].range < after_start[after_start.len() - 2].range;
    let cutoff = market_hour(now);
    let hours = aggregate_valid_hours(bars48h, cutoff);
    let breakout = check_breakout(rule, latest, &days, &hours, cutoff);

    StabilizationAssessment {
        state: rule.state.as_str().to_string(),
        observation_price: rule.observation_price.clone(),
        observation_low: low.map(|v| format!("{v:.2}")),
        valid_day_count: after_start.len(),
        low_stable,
        narrowing,
        breakout_price: breakout.price.map(|price| format!("{price:.2}")),
        breakout_met: breakout.price_met,
        breakout_matching_hours: breakout.matching_hours,
        breakout_required_hours: breakout.required_hours,
        data_sufficient: breakout.data_sufficient,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn breakout_requires_configured_days_and_matching_hour_windows() {
        let mut rule = rule(StabilizationState::Stabilizing, "850", Some("840"), None);
        rule.breakout_days = 3;
        rule.breakout_confirm_hours = 2;
        let days = vec![
            DayStat {
                date: "2026-10-30".into(),
                day_min: 839.0,
                day_max: 842.0,
                range: 3.0,
            },
            DayStat {
                date: "2026-10-31".into(),
                day_min: 839.0,
                day_max: 843.0,
                range: 4.0,
            },
        ];
        let cutoff = NaiveDate::from_ymd_opt(2026, 11, 2)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap();
        let hours = [
            HourBucket {
                start: cutoff - Duration::hours(2),
                avg: 845.0,
            },
            HourBucket {
                start: cutoff - Duration::hours(1),
                avg: 846.0,
            },
        ];
        let insufficient = check_breakout(&rule, 847.0, &days, &hours, cutoff);
        assert!(!insufficient.data_sufficient);
        assert_eq!(insufficient.price, None);

        let mut enough_days = days;
        enough_days.push(DayStat {
            date: "2026-11-01".into(),
            day_min: 840.0,
            day_max: 844.0,
            range: 4.0,
        });
        let one_hour = check_breakout(&rule, 847.0, &enough_days, &hours[1..], cutoff);
        assert!(one_hour.price_met);
        assert_eq!(one_hour.matching_hours, 1);
        assert!(!one_hour.confirmed());
        let confirmed = check_breakout(&rule, 847.0, &enough_days, &hours, cutoff);
        assert!(confirmed.confirmed());
    }

    #[test]
    fn cooldown_tracks_sent_notifications_instead_of_state_events() {
        let db = crate::storage::db::Db::open_in_memory().unwrap();
        let repo = StabilizationRepo(db.conn());
        let rule = rule(StabilizationState::Stabilizing, "850", None, None);
        repo.insert(&rule).unwrap();
        let code = rule.state.as_str();
        repo.insert_event(&StabilizationEvent {
            id: "event-1".into(),
            alert_id: rule.id.clone(),
            alert_name: rule.name.clone(),
            previous_state: None,
            current_state: code.into(),
            reference_price: None,
            reason: None,
            event_time: Local::now().to_rfc3339(),
        })
        .unwrap();

        let last_sent = || repo.last_notification_at(&rule.id, code).unwrap();
        assert!(notification_due(last_sent(), 10_000, 60));
        repo.record_notification(&rule.id, code, 10_000).unwrap();
        assert!(!notification_due(last_sent(), 13_599, 60));
        assert!(notification_due(last_sent(), 13_600, 60));
    }

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
    fn make_past_bars(days: usize, day_ranges: &[(f64, f64)]) -> Vec<HistoricalPricePoint> {
        let today = now().date_naive();
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
                    min_time: None,
                    max_time: None,
                    quality: "VALID",
                });
            }
        }
        pts
    }

    fn now() -> DateTime<FixedOffset> {
        Local::now().with_timezone(&market_time())
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
    fn recovery_can_reach_stabilizing_after_dip_and_rebound_while_closed() {
        let market_now = market_time()
            .with_ymd_and_hms(2026, 11, 13, 9, 0, 0)
            .unwrap();
        let mut rule = rule(StabilizationState::WaitingForDip, "835", None, None);
        rule.updated_at = "2026-11-10T07:00:00+08:00".into();
        rule.last_evaluated_at = Some("2026-11-10T08:00:00+08:00".into());
        let mut bars14 = Vec::new();
        for (day, low, high) in [(10, 832.0, 842.0), (11, 833.0, 840.0), (12, 834.0, 837.0)] {
            for hour in 9..15 {
                let avg = if hour == 9 { low + 0.5 } else { high - 0.5 };
                bars14.push(HistoricalPricePoint {
                    timestamp: format!("2026-11-{day} {hour:02}:00"),
                    min_price: Some(if hour == 9 { low } else { avg - 0.25 }),
                    max_price: Some(if hour == 9 { low + 1.0 } else { high }),
                    avg_price: Some(avg),
                    min_time: None,
                    max_time: None,
                    quality: "VALID",
                });
            }
        }
        let replay = super::super::recovery::closed_bars(&bars14, &[], market_now.to_utc());
        let outcome =
            evaluate_with_recovery(&rule, 840.0, &bars14, &[], &market_now, Some(&replay));
        assert_eq!(outcome.state, StabilizationState::Stabilizing);
        assert_eq!(
            outcome.started_at.as_deref(),
            Some("2026-11-10T10:00:00+08:00")
        );
        assert_eq!(outcome.low, Some(832.0));
        assert_eq!(outcome.notify, vec![NotifyKind::Stabilizing]);
        assert_eq!(outcome.events.len(), 2);
        assert!(outcome.events[0]
            .reason
            .as_deref()
            .unwrap()
            .starts_with("离线行情补记："));

        // 手动重置后的规则不回放重置前的低点。
        rule.updated_at = market_now.to_rfc3339();
        let outcome =
            evaluate_with_recovery(&rule, 840.0, &bars14, &[], &market_now, Some(&replay));
        assert_eq!(outcome.state, StabilizationState::WaitingForDip);
        assert!(outcome.events.is_empty());
    }

    #[test]
    fn no_stabilize_while_still_falling() {
        let now = now();
        let started = (now.date_naive() - Duration::days(3))
            .format("%Y-%m-%dT00:00:00+08:00")
            .to_string();
        let r = rule(
            StabilizationState::Observing,
            "840.00",
            Some("835.00"),
            Some(&started),
        );
        // 前两天振幅收窄，今日明显创新低。旧逻辑会立即判为企稳。
        let bars = make_past_bars(2, &[(835.0, 840.0), (834.5, 837.0)]);
        let out = evaluate(&r, 832.0, &bars, &[], &now);
        assert_eq!(out.state, StabilizationState::Observing);
        assert_eq!(out.low, Some(832.0));
        assert_eq!(out.started_at.as_deref(), Some(now.to_rfc3339().as_str()));
        assert!(out.notify.is_empty());
        assert!(!assess(&r, 832.0, &bars, &[], &now).low_stable);

        let mut persisted = r.clone();
        persisted.observation_started_at = out.started_at;
        persisted.observation_low = Some("832".into());
        let out = evaluate(&persisted, 832.1, &bars, &[], &now);
        assert_eq!(out.state, StabilizationState::Observing);
        assert!(out.events.is_empty());

        // 当前报价已经回升时，昨天的明显新低仍会重置观察日期。
        let bars = make_past_bars(2, &[(835.0, 840.0), (832.0, 835.0)]);
        let out = evaluate(&r, 836.0, &bars, &[], &now);
        let yesterday = now.date_naive() - Duration::days(1);
        assert_eq!(out.started_at, Some(format!("{yesterday}T00:00:00+08:00")));
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
        let bars = make_past_bars(2, &[(832.2, 834.2), (832.5, 833.5)]);
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
        let bars = make_past_bars(
            5,
            &[
                (833.0, 834.0),
                (833.2, 834.2),
                (833.0, 834.5),
                (833.1, 834.3),
                (833.2, 834.4),
            ],
        );
        let day = now().date_naive();
        let at = |hour, minute| {
            market_time()
                .from_local_datetime(&day.and_hms_opt(hour, minute, 0).unwrap())
                .unwrap()
        };
        let bars48: Vec<_> = [9, 10, 11]
            .into_iter()
            .flat_map(|h| {
                (0..6).map(move |i| {
                    let price = 835.0 + 0.1 * f64::from(i);
                    HistoricalPricePoint {
                        timestamp: format!("{day} {h:02}:{:02}", 10 * i),
                        min_price: Some(price - 0.2),
                        max_price: Some(price + 0.2),
                        avg_price: Some(price),
                        min_time: None,
                        max_time: None,
                        quality: "VALID",
                    }
                })
            })
            .collect();
        for points in [
            [&bars48[..6], &bars48[12..]].concat(),   // 缺少 10 点整个小时
            [&bars48[6..10], &bars48[11..]].concat(), // 10 点小时缺少一个数据点
            bars48[..12].to_vec(),                    // 最近的 11 点小时没有数据
        ] {
            let out = evaluate(&r, 835.8, &bars, &points, &at(12, 0));
            assert_eq!(out.state, StabilizationState::Stabilizing);
            assert!(!assess(&r, 835.8, &bars, &points, &at(12, 0)).data_sufficient);
        }
        let partial = &bars48[6..];
        let before_close = at(11, 55);
        let out = evaluate(&r, 835.8, &bars, partial, &before_close);
        assert_eq!(out.state, StabilizationState::Stabilizing);
        let cached = completed_hour_points(partial, &before_close.to_rfc3339());
        let out = evaluate(&r, 835.8, &bars, &cached, &at(12, 0));
        assert_eq!(out.state, StabilizationState::Stabilizing);
        let out = evaluate(&r, 835.8, &bars, partial, &at(12, 0));
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
        let bars = make_past_bars(
            5,
            &[
                (833.0, 834.0),
                (833.2, 834.2),
                (833.0, 834.5),
                (833.1, 834.3),
                (833.2, 834.4),
            ],
        );
        // 只有一个达标小时桶
        let bars48 = vec![HistoricalPricePoint {
            timestamp: format!("{} 11:00", Local::now().format("%Y-%m-%d")),
            min_price: Some(835.0),
            max_price: Some(835.8),
            avg_price: Some(835.4),
            min_time: None,
            max_time: None,
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
        assert_eq!(out.events[0].current, StabilizationState::Invalidated);
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
                min_time: None,
                max_time: None,
                quality: "VALID",
            },
            HistoricalPricePoint {
                timestamp: "2026-11-01 09:00".to_string(),
                min_price: Some(836.0),
                max_price: Some(836.0),
                avg_price: Some(836.0),
                min_time: None,
                max_time: None,
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
            min_time: None,
            max_time: None,
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
pub fn evaluate_all(
    app: &AppHandle,
    quote: &crate::models::GoldQuote,
    bars14: &[HistoricalPricePoint],
    bars48: &[HistoricalPricePoint],
    recovery: Option<&[ClosedBar<'_>]>,
    notify_enabled: bool,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let Ok(latest) = quote.customer_buy_price.trim().parse::<f64>() else {
        return Ok(());
    };

    let rules = {
        let db = state.db.lock().unwrap();
        StabilizationRepo(db.conn())
            .list(true)
            .map_err(|e| format!("读取企稳规则失败：{e}"))?
    };

    let now = parse_time(&quote.fetched_at)
        .ok_or("行情时间格式异常")?
        .with_timezone(&market_time());
    let mut notifications = Vec::new();
    let mut anything_changed = false;

    for rule in rules
        .iter()
        .filter(|r| r.product_code == quote.product_code)
    {
        let outcome = evaluate_with_recovery(rule, latest, bars14, bars48, &now, recovery);

        let db = state.db.lock().unwrap();
        let repo = StabilizationRepo(db.conn());
        let persisted = (|| -> Result<bool, String> {
            let tx = db
                .conn()
                .unchecked_transaction()
                .map_err(|e| e.to_string())?;
            if !repo.update_state(
                &rule.id,
                &rule.updated_at,
                &outcome.state,
                outcome.started_at.as_deref(),
                outcome.low,
                &now.to_rfc3339(),
            )? {
                return Ok(false);
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
                    event_time: ev.event_time.clone(),
                };
                repo.insert_event(&record)?;
            }
            // ONCE 模式：确认后自动停用
            if outcome.state == StabilizationState::Confirmed
                && rule.trigger_mode == crate::models::AlertTriggerMode::Once
            {
                repo.set_disabled_if_once(&rule.id)?;
            }
            tx.commit().map_err(|e| e.to_string())?;
            Ok(true)
        })();
        match persisted {
            Ok(true) => anything_changed |= !outcome.events.is_empty(),
            Ok(false) => continue,
            Err(e) => {
                eprintln!("[goldtray] 保存企稳评估失败：{e}");
                continue;
            }
        }
        {
            if notify_enabled {
                for kind in outcome.notify {
                    let state_code = match kind {
                        NotifyKind::Stabilizing => StabilizationState::Stabilizing.as_str(),
                        NotifyKind::Confirmed => StabilizationState::Confirmed.as_str(),
                    };
                    match repo.last_notification_at(&rule.id, state_code) {
                        Ok(last)
                            if notification_due(last, now.timestamp(), rule.cooldown_minutes) =>
                        {
                            notifications.push((
                                &rule.id,
                                state_code,
                                build_notification(
                                    rule,
                                    kind,
                                    outcome.events.iter().any(|event| {
                                        event.current.as_str() == state_code
                                            && event.reason.as_deref().is_some_and(|reason| {
                                                reason.starts_with("离线行情补记：")
                                            })
                                    }),
                                ),
                            ));
                        }
                        Ok(_) => {}
                        Err(e) => eprintln!("[goldtray] 读取企稳通知时间失败：{e}"),
                    }
                }
            }
        }
    }

    if anything_changed {
        let _ = app.emit("stabilization-changed", ());
    }

    for (id, state_code, (title, body)) in notifications {
        let reserved_at = Local::now().timestamp();
        {
            let db = state.db.lock().unwrap();
            StabilizationRepo(db.conn())
                .record_notification(id, state_code, reserved_at)
                .map_err(|e| format!("预留企稳通知失败：{e}"))?;
        }
        if let Err(e) = app.notification().builder().title(title).body(body).show() {
            eprintln!("[goldtray] 企稳通知发送失败：{e}");
            let db = state.db.lock().unwrap();
            if let Err(clear_error) =
                StabilizationRepo(db.conn()).clear_notification(id, state_code, reserved_at)
            {
                eprintln!("[goldtray] 清除企稳通知预留失败：{clear_error}");
            }
        }
    }
    Ok(())
}

fn notification_due(last_sent: Option<i64>, now: i64, cooldown_minutes: i64) -> bool {
    last_sent.map_or(true, |last| {
        now.saturating_sub(last) >= cooldown_minutes.saturating_mul(60)
    })
}

/// 生成企稳系统通知。仅提示参考信号，不含交易指令。
fn build_notification(
    rule: &StabilizationRule,
    kind: NotifyKind,
    historical: bool,
) -> (String, String) {
    if historical {
        let stage = match kind {
            NotifyKind::Stabilizing => "初步企稳",
            NotifyKind::Confirmed => "反弹确认",
        };
        return (
            format!("GoldTray · 离线{stage}补记"),
            format!(
                "「{}」在离线期间达到{stage}条件。请打开 GoldTray 查看最新状态和历史记录。",
                rule.name
            ),
        );
    }
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
