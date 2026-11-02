use serde::{Deserialize, Serialize};

/// 内部行情模型。
/// 价格使用十进制字符串，避免浮点精度问题。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GoldQuote {
    pub source: String,
    pub bank_code: String,
    pub bank_name: String,
    pub product_code: String,
    pub product_name: String,
    /// 客户买入参考价（元/克）
    pub customer_buy_price: String,
    /// 客户卖出价。当前数据源不提供，保持 None。
    pub customer_sell_price: Option<String>,
    /// 原始变化值，语义待验证
    pub raw_change: Option<String>,
    pub currency: String,
    pub unit: String,
    /// 本地成功获取响应的时间（RFC3339）
    pub fetched_at: String,
    /// 行情源明确提供的报价时间，当前源没有则为 None
    pub quoted_at: Option<String>,
    /// LIVE / DELAYED / STALE
    pub status: String,
    /// 第三方估算报价必须为 true
    pub is_estimated: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AlertDirection {
    Above,
    Below,
}

impl AlertDirection {
    pub fn as_str(&self) -> &'static str {
        match self {
            AlertDirection::Above => "ABOVE",
            AlertDirection::Below => "BELOW",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AlertPriceType {
    CustomerBuy,
    CustomerSell,
}

impl AlertPriceType {
    pub fn as_str(&self) -> &'static str {
        match self {
            AlertPriceType::CustomerBuy => "CUSTOMER_BUY",
            AlertPriceType::CustomerSell => "CUSTOMER_SELL",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AlertTriggerMode {
    Once,
    Rearmable,
}

impl AlertTriggerMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            AlertTriggerMode::Once => "ONCE",
            AlertTriggerMode::Rearmable => "REARMABLE",
        }
    }
}

/// 预警规则（含运行时触发状态）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlertRule {
    pub id: String,
    pub name: String,
    pub product_code: String,
    pub direction: AlertDirection,
    pub price_type: AlertPriceType,
    pub target_price: String,
    pub trigger_mode: AlertTriggerMode,
    pub cooldown_minutes: i64,
    pub enabled: bool,
    /// 是否处于待触发状态（位于触发区域外）
    pub armed: bool,
    /// 已连续满足触发条件的有效行情次数（默认连续两次确认）
    pub confirm_hits: i64,
    pub last_triggered_at: Option<String>,
    pub last_triggered_price: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// 新增 / 编辑预警规则的输入
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlertInput {
    pub name: String,
    pub product_code: String,
    pub direction: AlertDirection,
    pub price_type: AlertPriceType,
    pub target_price: String,
    pub trigger_mode: AlertTriggerMode,
    pub cooldown_minutes: i64,
}

/// 预警触发记录
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlertTriggerRecord {
    pub id: String,
    pub alert_id: String,
    pub alert_name: String,
    pub direction: AlertDirection,
    pub target_price: String,
    pub triggered_price: String,
    pub is_estimated: bool,
    pub triggered_at: String,
    pub notification_status: String,
}

/// 预警触发事件载荷（推送给前端）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlertTriggeredEvent {
    pub record: AlertTriggerRecord,
    pub current_price: String,
}
