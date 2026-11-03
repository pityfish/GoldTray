use std::time::Duration;

use rust_decimal::Decimal;
use serde::Deserialize;

use crate::models::historical_quote::{
    HistoricalPricePoint, HistoryPeriod, RawHistoryData, RawHistoryResponse,
};
use crate::models::GoldQuote;

pub const SOURCE_ID: &str = "huilvsudi";
const API_BASE: &str = "https://api2.huilvsudi.com";
const API_URL: &str = "https://api2.huilvsudi.com/app-api/mp/quote/list?tab=gold&bank=CMB";
const BANK_CODE: &str = "CMB";
const PRODUCT_CODE: &str = "GOLD_DEMAND";

/// 汇率速递 · 招商银行黄金活期行情源
pub struct HuilvsudiCmbProvider {
    client: reqwest::Client,
}

#[derive(Deserialize)]
struct ApiResponse {
    code: i64,
    #[serde(default)]
    data: Option<Vec<QuoteItem>>,
    #[serde(default)]
    msg: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct QuoteItem {
    bank: String,
    bank_name: String,
    currency: String,
    currency_name: String,
    selling_price: Option<String>,
    #[serde(default)]
    change: Option<f64>,
}

impl HuilvsudiCmbProvider {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent("GoldTray/0.1")
            .build()
            .expect("failed to build http client");
        Self { client }
    }

    /// 拉取并解析行情。任何失败都返回带中文说明的错误。
    pub async fn fetch_quote(&self) -> Result<GoldQuote, String> {
        let resp = self
            .client
            .get(API_URL)
            .header("Accept", "application/json")
            .header("tenant-id", "1")
            .header("terminal", "20")
            .send()
            .await
            .map_err(|e| format!("行情请求失败：{e}"))?;

        if !resp.status().is_success() {
            return Err(format!("行情接口返回异常状态：{}", resp.status()));
        }

        let body: ApiResponse = resp
            .json()
            .await
            .map_err(|e| format!("行情数据解析失败（JSON 结构可能已变化）：{e}"))?;

        if body.code != 0 {
            return Err(format!(
                "行情接口返回错误码 {}：{}",
                body.code,
                body.msg.unwrap_or_default()
            ));
        }

        let items = body
            .data
            .ok_or_else(|| "行情接口未返回数据记录".to_string())?;

        let item = items
            .iter()
            .find(|i| i.bank == BANK_CODE && i.currency == PRODUCT_CODE)
            .ok_or_else(|| "接口中未找到招商银行黄金活期记录，产品信息可能已变更".to_string())?;

        let selling_price = item
            .selling_price
            .as_deref()
            .ok_or_else(|| "行情记录缺少 sellingPrice 字段".to_string())?;

        let price = selling_price
            .trim()
            .parse::<Decimal>()
            .ok()
            .filter(|p| *p > Decimal::ZERO)
            .ok_or_else(|| format!("sellingPrice 不是合法正数：{selling_price}"))?;

        let raw_change = item
            .change
            .filter(|c| c.is_finite())
            .map(|c| format!("{c:.2}"));

        Ok(GoldQuote {
            source: SOURCE_ID.to_string(),
            bank_code: item.bank.clone(),
            bank_name: item.bank_name.clone(),
            product_code: item.currency.clone(),
            product_name: item.currency_name.clone(),
            customer_buy_price: price.to_string(),
            customer_sell_price: None,
            raw_change,
            currency: "CNY".to_string(),
            unit: "GRAM".to_string(),
            fetched_at: chrono::Local::now().to_rfc3339(),
            quoted_at: None,
            status: "LIVE".to_string(),
            is_estimated: false,
        })
    }

/// 按 series.name 识别序列（实测为中文，兼容英文），禁止依赖数组顺序
fn find_series<'a>(data: &'a RawHistoryData, names: &[&str]) -> Option<&'a crate::models::historical_quote::RawSeries> {
    for name in names {
        if let Some(s) = data.series.iter().find(|s| s.name == *name) {
            return Some(s);
        }
    }
    None
}

/// 归一化时间戳为 "YYYY-MM-DD HH:MM"；解析失败返回 None
fn normalize_timestamp(raw: &str) -> Option<String> {
    let s = raw.trim();
    // "2026-11-02 17:00" 或 "2026-11-02 17:00:00" → 取前 16 位
    if s.len() >= 16
        && s.as_bytes()[4] == b'-'
        && s.as_bytes()[7] == b'-'
        && s.as_bytes()[10] == b' '
    {
        let core = &s[..16];
        // 校验数字与冒号格式
        if core
            .bytes()
            .enumerate()
            .all(|(i, b)| matches!(i, 4 | 7 | 10 | 13) || b.is_ascii_digit())
        {
            return Some(core.to_string());
        }
    }
    None
}

/// 拉取并标准化历史行情。
/// 以 categories 为主时间轴按索引关联各序列，高低价关系异常的点标记 SUSPECT。
/// 返回 (标准化数据点, 接口原始 stats)。
pub async fn fetch_history(
    &self,
    period: HistoryPeriod,
) -> Result<
    (
        Vec<HistoricalPricePoint>,
        crate::models::historical_quote::RawHistoryStats,
    ),
    String,
> {
    let url = format!("{API_BASE}/app-api/mp/quote-detail/chart-data");
    let resp = self
        .client
        .get(&url)
        .query(&[
            ("bank", BANK_CODE),
            ("currency", PRODUCT_CODE),
            ("priceType", "SELL"),
            ("period", period.as_api()),
        ])
        .header("Accept", "application/json")
        .header("tenant-id", "1")
        .header("terminal", "20")
        .send()
        .await
        .map_err(|e| format!("历史行情请求失败：{e}"))?;

    if !resp.status().is_success() {
        return Err(format!("历史行情接口返回异常状态：{}", resp.status()));
    }

    let body: RawHistoryResponse = resp
        .json()
        .await
        .map_err(|e| format!("历史行情解析失败（结构可能已变化）：{e}"))?;
    if body.code != 0 {
        return Err(format!(
            "历史行情接口返回错误码 {}：{}",
            body.code,
            body.msg.unwrap_or_default()
        ));
    }
    let data = body.data.ok_or_else(|| "历史行情接口未返回数据".to_string())?;
    if data.categories.is_empty() {
        return Err("历史行情数据为空".to_string());
    }

    let min_s = Self::find_series(&data, &["最小值", "min", "lowest"]);
    let max_s = Self::find_series(&data, &["最大值", "max", "highest"]);
    let avg_s = Self::find_series(&data, &["平均值", "avg", "average"]);

    let mut points: Vec<HistoricalPricePoint> = Vec::with_capacity(data.categories.len());
    let mut last_ts = String::new();
    for (i, cat) in data.categories.iter().enumerate() {
        let Some(ts) = Self::normalize_timestamp(cat) else {
            continue; // 无法解析的时间戳直接丢弃
        };
        if ts == last_ts {
            continue; // 归一化后重复的时间戳去重（保留首个）
        }
        last_ts = ts.clone();

        let min = min_s.and_then(|s| s.data.get(i)).copied().flatten();
        let max = max_s.and_then(|s| s.data.get(i)).copied().flatten();
        let avg = avg_s.and_then(|s| s.data.get(i)).copied().flatten();

        let quality = match (min, max, avg) {
            (None, None, None) => "MISSING",
            (Some(mi), Some(ma), _) if ma < mi => "SUSPECT", // 高低价关系异常
            (Some(mi), _, Some(a)) if a < mi => "SUSPECT",
            (Some(mi), _, Some(a)) if mi <= 0.0 || a <= 0.0 => "SUSPECT",
            _ => "VALID",
        };
        points.push(HistoricalPricePoint {
            timestamp: ts,
            min_price: min,
            max_price: max,
            avg_price: avg,
            quality,
        });
    }

    if points.is_empty() {
        return Err("历史行情数据解析后为空".to_string());
    }
    Ok((points, data.stats))
}
}
