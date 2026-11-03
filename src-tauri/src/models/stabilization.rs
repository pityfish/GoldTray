use serde::{Deserialize, Serialize};

use super::AlertTriggerMode;

/// 企稳状态机状态
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum StabilizationState {
    WaitingForDip,
    Observing,
    Stabilizing,
    Confirmed,
    Invalidated,
    InsufficientData,
}

impl StabilizationState {
    pub fn as_str(&self) -> &'static str {
        match self {
            StabilizationState::WaitingForDip => "WAITING_FOR_DIP",
            StabilizationState::Observing => "OBSERVING",
            StabilizationState::Stabilizing => "STABILIZING",
            StabilizationState::Confirmed => "CONFIRMED",
            StabilizationState::Invalidated => "INVALIDATED",
            StabilizationState::InsufficientData => "INSUFFICIENT_DATA",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "OBSERVING" => StabilizationState::Observing,
            "STABILIZING" => StabilizationState::Stabilizing,
            "CONFIRMED" => StabilizationState::Confirmed,
            "INVALIDATED" => StabilizationState::Invalidated,
            "INSUFFICIENT_DATA" => StabilizationState::InsufficientData,
            _ => StabilizationState::WaitingForDip,
        }
    }
}

/// 智能企稳预警规则（含运行时状态）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StabilizationRule {
    pub id: String,
    pub name: String,
    pub product_code: String,
    /// 观察价格（≤ 该价进入观察区）
    pub observation_price: String,
    /// 初步企稳：连续有效交易日数
    pub stable_days: i64,
    /// 低点容忍偏差（元/克）：低于观察期最低超过该值视为明显创新低
    pub low_tolerance: String,
    /// 突破参考：最近 N 个有效交易日高点
    pub breakout_days: i64,
    /// 反弹确认：连续 N 个有效小时窗口保持在突破价上方
    pub breakout_confirm_hours: i64,
    pub trigger_mode: AlertTriggerMode,
    pub cooldown_minutes: i64,
    /// 通知阶段：初步企稳
    pub notify_stabilizing: bool,
    /// 通知阶段：反弹确认
    pub notify_confirmed: bool,
    pub enabled: bool,
    pub state: StabilizationState,
    pub observation_started_at: Option<String>,
    pub observation_low: Option<String>,
    pub last_evaluated_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// 新增/编辑企稳规则的输入
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StabilizationInput {
    pub name: String,
    pub product_code: String,
    pub observation_price: String,
    pub stable_days: i64,
    pub low_tolerance: String,
    pub breakout_days: i64,
    pub breakout_confirm_hours: i64,
    pub trigger_mode: AlertTriggerMode,
    pub cooldown_minutes: i64,
    pub notify_stabilizing: bool,
    pub notify_confirmed: bool,
}

/// 企稳状态变更事件
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StabilizationEvent {
    pub id: String,
    pub alert_id: String,
    pub alert_name: String,
    pub previous_state: Option<String>,
    pub current_state: String,
    pub reference_price: Option<String>,
    pub reason: Option<String>,
    pub event_time: String,
}

/// 前端展示用的判断依据
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StabilizationAssessment {
    pub state: String,
    pub observation_price: String,
    pub observation_low: Option<String>,
    pub valid_day_count: usize,
    /// 最近 stable_days 个有效交易日未明显创新低
    pub low_stable: bool,
    /// 最近两个有效交易日波动收窄
    pub narrowing: bool,
    /// 反弹突破参考价
    pub breakout_price: Option<String>,
    pub breakout_met: bool,
    /// 数据是否足以支撑判断
    pub data_sufficient: bool,
}
