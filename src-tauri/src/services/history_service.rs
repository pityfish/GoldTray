use std::sync::OnceLock;

use chrono::{DateTime, Utc};
use tauri::{AppHandle, Manager};
use tokio::sync::Mutex;

use crate::models::historical_quote::{
    HistoricalPricePoint, HistoryPeriod, HistoryPayload, HistoryStats,
};
use crate::services::quote_service::AppState;
use crate::storage::history_repository;

/// 历史请求合并闸门：同一时刻仅一个历史请求进行
static HISTORY_GATE: OnceLock<Mutex<()>> = OnceLock::new();

fn is_fresh(fetched_at: &str, period: HistoryPeriod) -> bool {
    DateTime::parse_from_rfc3339(fetched_at)
        .map(|t| (Utc::now() - t.with_timezone(&Utc)).num_seconds() < period.max_age_secs())
        .unwrap_or(false)
}

fn make_payload(
    period: HistoryPeriod,
    points: Vec<HistoricalPricePoint>,
    fetched_at: Option<String>,
    stats: Option<HistoryStats>,
    from_cache: bool,
    stale: bool,
) -> HistoryPayload {
    HistoryPayload {
        period: period.as_api().to_string(),
        points,
        stats: stats.unwrap_or_else(|| {
            history_repository::build_stats(None, &[])
        }),
        fetched_at: fetched_at.unwrap_or_default(),
        from_cache,
        stale,
    }
}

/// 获取历史行情：新鲜缓存优先 → 合并并发请求 → 拉取接口 → 失败降级旧缓存。
/// 历史行情异常不影响实时行情与预警（调用方独立处理）。
pub async fn get_history(
    app: &AppHandle,
    period: HistoryPeriod,
    force: bool,
) -> Result<HistoryPayload, String> {
    let state = app.state::<AppState>();

    // 1. 新鲜缓存直接返回
    if !force {
        let cached = {
            let db = state.db.lock().unwrap();
            history_repository::load_bars(db.conn(), period)?
        };
        if let (points, Some(fa), stats) = cached {
            if !points.is_empty() && is_fresh(&fa, period) {
                return Ok(make_payload(period, points, Some(fa), stats, true, false));
            }
        }
    }

    // 2. 闸门内合并并发请求（double check）
    let gate = HISTORY_GATE.get_or_init(|| Mutex::new(()));
    let _guard = gate.lock().await;
    if !force {
        let cached = {
            let db = state.db.lock().unwrap();
            history_repository::load_bars(db.conn(), period)?
        };
        if let (points, Some(fa), stats) = cached {
            if !points.is_empty() && is_fresh(&fa, period) {
                return Ok(make_payload(period, points, Some(fa), stats, true, false));
            }
        }
    }

    // 3. 拉取并缓存
    match state.provider.fetch_history(period).await {
        Ok((points, raw_stats)) => {
            let stats = history_repository::build_stats(Some(&raw_stats), &points);
            {
                let db = state.db.lock().unwrap();
                history_repository::save_bars(db.conn(), period, &points, &stats)?;
            }
            Ok(make_payload(
                period,
                points,
                Some(chrono::Local::now().to_rfc3339()),
                Some(stats),
                false,
                false,
            ))
        }
        Err(e) => {
            // 网络异常：有旧缓存则降级返回旧数据并标记 stale
            let cached = {
                let db = state.db.lock().unwrap();
                history_repository::load_bars(db.conn(), period)?
            };
            if !cached.0.is_empty() {
                Ok(make_payload(period, cached.0, cached.1, cached.2, true, true))
            } else {
                Err(e)
            }
        }
    }
}
