use chrono::Local;
use rusqlite::params;

use crate::models::historical_quote::{
    HistoricalPricePoint, HistoryPeriod, HistoryStats, RawHistoryStats,
};

pub const SOURCE: &str = "huilvsudi";
pub const PRODUCT_CODE: &str = "GOLD_DEMAND";

fn stats_key(period: HistoryPeriod) -> String {
    format!("history_stats_{}", period.as_api())
}

fn price_to_str(v: Option<f64>) -> Option<String> {
    v.map(|p| format!("{p:.2}"))
}

fn str_to_price(s: Option<String>) -> Option<f64> {
    s.and_then(|v| v.trim().parse::<f64>().ok())
}

fn get_setting(conn: &rusqlite::Connection, key: &str) -> Option<String> {
    conn.query_row(
        "SELECT value FROM app_settings WHERE key=?1",
        params![key],
        |r| r.get(0),
    )
    .ok()
}

fn set_setting(conn: &rusqlite::Connection, key: &str, value: &str) -> Result<(), String> {
    conn.execute(
        "INSERT INTO app_settings(key,value,updated_at) VALUES(?1,?2,?3)
         ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at=excluded.updated_at",
        params![key, value, Local::now().to_rfc3339()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 覆盖写入某周期数据点及接口 stats（事务内先删后插）
pub fn save_bars(
    conn: &rusqlite::Connection,
    period: HistoryPeriod,
    points: &[HistoricalPricePoint],
    stats: &HistoryStats,
) -> Result<(), String> {
    let fetched_at = Local::now().to_rfc3339();
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("开启事务失败：{e}"))?;
    tx.execute(
        "DELETE FROM gold_history_bars WHERE source=?1 AND product_code=?2 AND period=?3",
        params![SOURCE, PRODUCT_CODE, period.as_api()],
    )
    .map_err(|e| e.to_string())?;
    for p in points {
        tx.execute(
            "INSERT INTO gold_history_bars
             (source, product_code, period, timestamp, min_price, max_price, avg_price, quality, fetched_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                SOURCE,
                PRODUCT_CODE,
                period.as_api(),
                p.timestamp,
                price_to_str(p.min_price),
                price_to_str(p.max_price),
                price_to_str(p.avg_price),
                p.quality,
                fetched_at,
            ],
        )
        .map_err(|e| format!("写入历史行情失败：{e}"))?;
    }
    tx.commit().map_err(|e| format!("提交事务失败：{e}"))?;

    // stats 缓存到 app_settings，加载时反序列化
    let stats_json = serde_json::to_string(stats).unwrap_or_else(|_| "{}".to_string());
    set_setting(conn, &stats_key(period), &stats_json)?;
    Ok(())
}

/// 读取某周期缓存数据、获取时间与缓存 stats
pub fn load_bars(
    conn: &rusqlite::Connection,
    period: HistoryPeriod,
) -> Result<(Vec<HistoricalPricePoint>, Option<String>, Option<HistoryStats>), String> {
    let mut stmt = conn
        .prepare(
            "SELECT timestamp, min_price, max_price, avg_price, quality, fetched_at
             FROM gold_history_bars
             WHERE source=?1 AND product_code=?2 AND period=?3
             ORDER BY timestamp ASC",
        )
        .map_err(|e| e.to_string())?;
    let mut points = Vec::new();
    let mut fetched_at: Option<String> = None;
    let rows = stmt
        .query_map(
            params![SOURCE, PRODUCT_CODE, period.as_api()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                ))
            },
        )
        .map_err(|e| e.to_string())?;
    for row in rows {
        let (ts, min, max, avg, quality, fa) = row.map_err(|e| e.to_string())?;
        fetched_at = Some(fa);
        points.push(HistoricalPricePoint {
            timestamp: ts,
            min_price: str_to_price(min),
            max_price: str_to_price(max),
            avg_price: str_to_price(avg),
            quality: match quality.as_str() {
                "MISSING" => "MISSING",
                "CARRIED" => "CARRIED",
                "SUSPECT" => "SUSPECT",
                _ => "VALID",
            },
        });
    }

    let stats = get_setting(conn, &stats_key(period))
        .and_then(|json| serde_json::from_str::<HistoryStats>(&json).ok());

    Ok((points, fetched_at, stats))
}

/// 优先接口 stats，缺失时基于数据点本地计算补齐（口径明确：区间首尾平均值涨跌幅）
pub fn build_stats(
    stats: Option<&RawHistoryStats>,
    points: &[HistoricalPricePoint],
) -> HistoryStats {
    let mut out = match stats {
        Some(s) => HistoryStats {
            max_price: s.max_price.clone(),
            max_price_time: s.max_price_time.clone(),
            min_price: s.min_price.clone(),
            min_price_time: s.min_price_time.clone(),
            avg_price: s.avg_price.clone(),
            change_percent: s.change_percent.clone(),
        },
        None => HistoryStats {
            max_price: None,
            max_price_time: None,
            min_price: None,
            min_price_time: None,
            avg_price: None,
            change_percent: None,
        },
    };

    // 本地计算兜底：任一统计缺失时基于数据点重算
    let valid: Vec<&HistoricalPricePoint> = points
        .iter()
        .filter(|p| p.quality == "VALID" && p.avg_price.is_some())
        .collect();
    if !valid.is_empty() {
        if out.max_price.is_none() {
            if let Some(p) = valid.iter().max_by(|a, b| {
                a.max_price
                    .partial_cmp(&b.max_price)
                    .unwrap_or(std::cmp::Ordering::Equal)
            }) {
                out.max_price = price_to_str(p.max_price);
                out.max_price_time = Some(p.timestamp.clone());
            }
        }
        if out.min_price.is_none() {
            if let Some(p) = valid.iter().min_by(|a, b| {
                a.min_price
                    .partial_cmp(&b.min_price)
                    .unwrap_or(std::cmp::Ordering::Equal)
            }) {
                out.min_price = price_to_str(p.min_price);
                out.min_price_time = Some(p.timestamp.clone());
            }
        }
        if out.avg_price.is_none() {
            let n = valid.len() as f64;
            let mean = valid.iter().map(|p| p.avg_price.unwrap()).sum::<f64>() / n;
            out.avg_price = Some(format!("{mean:.2}"));
        }
        if out.change_percent.is_none() && valid.len() >= 2 {
            let first = valid[0].avg_price.unwrap();
            let last = valid[valid.len() - 1].avg_price.unwrap();
            if first > 0.0 {
                out.change_percent = Some(format!("{:.2}", (last - first) / first * 100.0));
            }
        }
    }
    out
}
