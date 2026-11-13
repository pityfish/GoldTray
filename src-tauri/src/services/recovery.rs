use std::collections::HashSet;

use chrono::{DateTime, Duration, FixedOffset, NaiveDateTime, NaiveTime, TimeZone, Timelike, Utc};

use crate::models::historical_quote::HistoricalPricePoint;

pub fn parse_time(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value).ok().map(|t| t.to_utc())
}

pub fn market_time() -> FixedOffset {
    FixedOffset::east_opt(8 * 3600).unwrap()
}

pub fn gap_detected(
    last: Option<DateTime<Utc>>,
    fetched: DateTime<Utc>,
    interval_secs: u64,
    retrying: bool,
) -> bool {
    retrying
        || last.map_or(true, |last| {
            (fetched - last).num_seconds() > interval_secs as i64 + 30
        })
}

pub struct ClosedBar<'a> {
    pub start: DateTime<FixedOffset>,
    pub end: DateTime<FixedOffset>,
    pub min: f64,
    pub max: f64,
    pub avg: Option<f64>,
    pub point: &'a HistoricalPricePoint,
    pub complete: bool,
    pub min_at: Option<DateTime<FixedOffset>>,
    pub max_at: Option<DateTime<FixedOffset>>,
}

impl ClosedBar<'_> {
    pub fn includes_extreme(
        &self,
        at: Option<DateTime<FixedOffset>>,
        since: DateTime<Utc>,
    ) -> bool {
        (self.complete && self.start >= since) || at.is_some_and(|at| at > since)
    }
}

/// 48H 返回的实际范围可能不足 48 小时。仅在完整时段连续覆盖缺口时跳过 14d。
pub fn short_history_covers(
    bars: &[ClosedBar<'_>],
    since: Option<DateTime<Utc>>,
    until: DateTime<Utc>,
) -> bool {
    let Some(since) = since else { return false };
    let step = Duration::minutes(10);
    let mut covered_until = since;
    for bar in bars.iter().filter(|bar| bar.complete && bar.end > since) {
        if bar.start > covered_until {
            return false;
        }
        covered_until = bar.end.to_utc();
    }
    covered_until > since && until - covered_until < step
}

/// 重叠时优先使用 48H；跨缺口边界的时段仅采用带有可验证时间的极值。
pub fn closed_bars<'a>(
    bars14: &'a [HistoricalPricePoint],
    bars48: &'a [HistoricalPricePoint],
    until: DateTime<Utc>,
) -> Vec<ClosedBar<'a>> {
    let time = |point: &HistoricalPricePoint| {
        let time = NaiveDateTime::parse_from_str(&point.timestamp, "%Y-%m-%d %H:%M").ok()?;
        market_time().from_local_datetime(&time).single()
    };
    let fine_start = bars48.iter().filter_map(time).min();
    let mut bars = Vec::new();
    let mut fine_slots = HashSet::new();
    for (points, minutes) in [(bars48, 10), (bars14, 60)] {
        for point in points {
            let Some(start) = time(point) else { continue };
            let end = start + Duration::minutes(minutes);
            if point.quality != "VALID" || start >= until {
                continue;
            }
            let (Some(min), Some(max)) = (point.min_price, point.max_price) else {
                continue;
            };
            if !min.is_finite() || !max.is_finite() || min <= 0.0 || max < min {
                continue;
            }
            let avg = point.avg_price.filter(|a| (min..=max).contains(a));
            let extreme_at = |value: Option<&str>| {
                let time = NaiveTime::parse_from_str(value?, "%H:%M").ok()?;
                let at = market_time()
                    .from_local_datetime(&start.date_naive().and_time(time))
                    .single()?;
                (at >= start && at < end && at + Duration::minutes(1) <= until).then_some(at)
            };
            let min_at = extreme_at(point.min_time.as_deref());
            let max_at = extreme_at(point.max_time.as_deref());
            let complete = end <= until;
            let bar = ClosedBar {
                start,
                end: end.min(until.with_timezone(&market_time())),
                min,
                max,
                avg,
                point,
                complete,
                min_at,
                max_at,
            };
            if minutes == 60 && fine_start.is_some_and(|fine| end > fine) {
                // 小时聚合只能补有时间证据的触达，不能证明整个缺口处于安全侧。
                let step = Duration::minutes(10);
                for slot in (0..6).map(|i| start + step * i) {
                    if fine_slots.contains(&slot) {
                        continue;
                    }
                    let in_slot = |at: &DateTime<FixedOffset>| *at >= slot && *at < slot + step;
                    let min_at = min_at.filter(in_slot);
                    let max_at = max_at.filter(in_slot);
                    if min_at.is_some() || max_at.is_some() {
                        bars.push(ClosedBar {
                            start: slot,
                            end: (slot + step).min(bar.end),
                            complete: false,
                            avg: None,
                            min_at,
                            max_at,
                            ..bar
                        });
                    }
                }
            } else if complete || min_at.is_some() || max_at.is_some() {
                if minutes == 10 && start.minute() % 10 == 0 {
                    fine_slots.insert(start);
                }
                bars.push(bar);
            }
        }
    }
    bars.sort_unstable_by_key(|bar| bar.start);
    bars.dedup_by_key(|bar| bar.start);
    bars
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_restart_reconnect_and_wake_without_replaying_normal_polls() {
        let last = parse_time("2026-11-13T09:00:00+08:00").unwrap();
        let next = last + Duration::seconds(60);
        assert!(!gap_detected(Some(last), next, 60, false));
        assert!(gap_detected(None, next, 60, false));
        assert!(gap_detected(Some(last), next, 60, true));
        assert!(gap_detected(
            Some(last),
            next + Duration::minutes(10),
            60,
            false,
        ));
    }

    #[test]
    fn short_history_requires_contiguous_coverage_of_the_gap() {
        let point = |minute| HistoricalPricePoint {
            timestamp: format!("2026-11-13 09:{minute:02}"),
            min_price: Some(834.0),
            max_price: Some(840.0),
            avg_price: Some(837.0),
            min_time: None,
            max_time: None,
            quality: "VALID",
        };
        let points = [point(0), point(10), point(20)];
        let since = parse_time("2026-11-13T09:04:00+08:00");
        let until = parse_time("2026-11-13T09:32:00+08:00").unwrap();
        let bars = closed_bars(&[], &points, until);
        assert!(short_history_covers(&bars, since, until));
        assert!(!short_history_covers(&bars[1..], since, until));
        assert!(!short_history_covers(&bars[..2], since, until));
        let with_hole = [points[0].clone(), points[2].clone()];
        let bars = closed_bars(&[], &with_hole, until);
        assert!(!short_history_covers(&bars, since, until));
    }

    #[test]
    fn hourly_extreme_fills_only_a_missing_ten_minute_slot() {
        let hour = HistoricalPricePoint {
            timestamp: "2026-11-13 09:00".into(),
            min_price: Some(830.0),
            max_price: Some(845.0),
            avg_price: Some(837.0),
            min_time: Some("09:14".into()),
            max_time: Some("09:24".into()),
            quality: "VALID",
        };
        let fine = |minute| HistoricalPricePoint {
            timestamp: format!("2026-11-13 09:{minute:02}"),
            min_price: Some(835.0),
            max_price: Some(840.0),
            avg_price: Some(837.0),
            min_time: None,
            max_time: None,
            quality: "VALID",
        };
        let hours = [hour];
        let fine_points = [fine(0), fine(20)];
        let until = parse_time("2026-11-13T10:00:00+08:00").unwrap();
        let since = parse_time("2026-11-13T09:12:00+08:00").unwrap();
        let bars = closed_bars(&hours, &fine_points, until);
        assert_eq!(bars.len(), 3);
        let fallback = &bars[1];
        assert_eq!(fallback.start.minute(), 10);
        assert_eq!(fallback.min, 830.0);
        assert!(fallback.includes_extreme(fallback.min_at, since));
        assert!(!fallback.includes_extreme(fallback.max_at, since));
        assert!(!fallback.complete);
        assert!(fallback.avg.is_none());
        assert_eq!(bars[2].max, 840.0);

        let fine_points = [fine(0), fine(10), fine(20)];
        let bars = closed_bars(&hours, &fine_points, until);
        assert_eq!(bars.len(), 3);
        assert_eq!(bars[1].min, 835.0);
        assert!(bars[1].complete);
    }
}
