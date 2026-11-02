/** 行情状态 */
export type QuoteStatus = "LIVE" | "DELAYED" | "STALE";

/** 内部行情模型（与 Rust 端 GoldQuote 对应） */
export interface GoldQuote {
  source: string;
  bankCode: string;
  bankName: string;
  productCode: string;
  productName: string;
  /** 客户买入参考价（元/克），十进制字符串 */
  customerBuyPrice: string;
  /** 客户卖出价，当前数据源不提供，保持空 */
  customerSellPrice: string | null;
  rawChange: string | null;
  currency: "CNY";
  unit: "GRAM";
  /** 本地成功获取响应的时间（ISO8601） */
  fetchedAt: string;
  quotedAt: string | null;
  status: QuoteStatus;
  isEstimated: boolean;
}

export type AlertDirection = "ABOVE" | "BELOW";
export type AlertPriceType = "CUSTOMER_BUY" | "CUSTOMER_SELL";
export type AlertTriggerMode = "ONCE" | "REARMABLE";

/** 预警规则（与 Rust 端 AlertRule 对应） */
export interface PriceAlert {
  id: string;
  name: string;
  productCode: string;
  direction: AlertDirection;
  priceType: AlertPriceType;
  targetPrice: string;
  triggerMode: AlertTriggerMode;
  cooldownMinutes: number;
  enabled: boolean;
  /** 是否处于待触发状态（已确认在触发区域外） */
  armed: boolean;
  /** 已连续满足条件的有效行情次数（二次确认用） */
  confirmHits: number;
  lastTriggeredAt: string | null;
  lastTriggeredPrice: string | null;
  createdAt: string;
  updatedAt: string;
}

/** 新增/编辑预警的输入 */
export interface AlertInput {
  name: string;
  productCode: string;
  direction: AlertDirection;
  priceType: AlertPriceType;
  targetPrice: string;
  triggerMode: AlertTriggerMode;
  cooldownMinutes: number;
}

/** 预警触发记录 */
export interface AlertTriggerRecord {
  id: string;
  alertId: string;
  alertName: string;
  direction: AlertDirection;
  targetPrice: string;
  triggeredPrice: string;
  isEstimated: boolean;
  triggeredAt: string;
  notificationStatus: string;
}

/** 预警触发时前端收到的事件载荷 */
export interface AlertTriggeredEvent {
  record: AlertTriggerRecord;
  currentPrice: string;
}

/** 应用设置项 key */
export const SETTING_KEYS = {
  refreshIntervalSecs: "refresh_interval_secs",
  enableNotification: "enable_notification",
  enableChangeDisplay: "enable_change_display",
  estimatedSpread: "estimated_spread",
  stalenessThresholdSecs: "staleness_threshold_secs",
} as const;

/** 应用设置（前端聚合视图） */
export interface AppSettings {
  refreshIntervalSecs: number;
  enableNotification: boolean;
  enableChangeDisplay: boolean;
  /** 卖出价差估算（元/克），空表示未启用估算 */
  estimatedSpread: string | null;
  stalenessThresholdSecs: number;
}
