use std::time::Duration;

use rust_decimal::Decimal;
use serde::Deserialize;

use crate::models::GoldQuote;

pub const SOURCE_ID: &str = "huilvsudi";
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
}
