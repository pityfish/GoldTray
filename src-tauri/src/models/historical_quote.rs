use serde::{Deserialize, Serialize};

/// 历史行情周期
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HistoryPeriod {
    H48,
    D14,
    M1,
    M3,
    M6,
    M12,
}

impl HistoryPeriod {
    pub fn as_api(&self) -> &'static str {
        match self {
            HistoryPeriod::H48 => "48H",
            HistoryPeriod::D14 => "14d",
            HistoryPeriod::M1 => "1M",
            HistoryPeriod::M3 => "3M",
            HistoryPeriod::M6 => "6M",
            HistoryPeriod::M12 => "12M",
        }
    }

    pub fn from_api(s: &str) -> Option<Self> {
        Some(match s {
            "48H" => HistoryPeriod::H48,
            "14d" => HistoryPeriod::D14,
            "1M" => HistoryPeriod::M1,
            "3M" => HistoryPeriod::M3,
            "6M" => HistoryPeriod::M6,
            "12M" => HistoryPeriod::M12,
            _ => return None,
        })
    }

    /// 缓存有效期（秒）：48H 15分钟、14d 30分钟、更长周期 60 分钟
    pub fn max_age_secs(&self) -> i64 {
        match self {
            HistoryPeriod::H48 => 15 * 60,
            HistoryPeriod::D14 => 30 * 60,
            _ => 60 * 60,
        }
    }
}

/// 接口原始响应结构（仅解析内部使用）
#[derive(Deserialize)]
pub struct RawHistoryResponse {
    pub code: i64,
    pub data: Option<RawHistoryData>,
    #[serde(default)]
    pub msg: Option<String>,
}

#[derive(Deserialize)]
pub struct RawHistoryData {
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub series: Vec<RawSeries>,
    #[serde(default)]
    pub stats: RawHistoryStats,
}

#[derive(Deserialize)]
pub struct RawSeries {
    pub name: String,
    /// 实测可为 null，以 categories 为主时间轴（保留原始字段供排查）
    #[serde(default)]
    pub time: Option<Vec<String>>,
    #[serde(default)]
    pub data: Vec<Option<f64>>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct RawHistoryStats {
    pub start_date: Option<String>,
    pub total_days: Option<i64>,
    pub max_price: Option<String>,
    pub max_price_time: Option<String>,
    pub min_price: Option<String>,
    pub min_price_time: Option<String>,
    pub avg_price: Option<String>,
    pub change_percent: Option<String>,
}

/// 标准化后的单点行情
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoricalPricePoint {
    /// 统一格式 "YYYY-MM-DD HH:MM"（Asia/Shanghai）
    pub timestamp: String,
    pub min_price: Option<f64>,
    pub max_price: Option<f64>,
    pub avg_price: Option<f64>,
    #[serde(skip)]
    pub min_time: Option<String>,
    #[serde(skip)]
    pub max_time: Option<String>,
    /// VALID / CARRIED / MISSING / SUSPECT
    pub quality: &'static str,
}

/// 区间统计（优先接口 stats，缺失时本地计算补齐）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryStats {
    pub max_price: Option<String>,
    pub max_price_time: Option<String>,
    pub min_price: Option<String>,
    pub min_price_time: Option<String>,
    pub avg_price: Option<String>,
    pub change_percent: Option<String>,
}

/// 提供给前端的走势图数据
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryPayload {
    pub period: String,
    pub points: Vec<HistoricalPricePoint>,
    pub stats: HistoryStats,
    /// 本份缓存数据的获取时间
    pub fetched_at: String,
    /// 是否来自本地缓存
    pub from_cache: bool,
    /// 缓存已过期（网络异常时的旧数据）
    pub stale: bool,
}
