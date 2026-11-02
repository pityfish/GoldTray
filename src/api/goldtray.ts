import { invoke } from "@tauri-apps/api/core";
import type {
  AlertInput,
  AlertTriggerRecord,
  AppSettings,
  GoldQuote,
  PriceAlert,
} from "../types/gold";
import { SETTING_KEYS } from "../types/gold";

/** 获取最近一次有效行情（可能为空） */
export function getLatestQuote(): Promise<GoldQuote | null> {
  return invoke("get_latest_quote");
}

/** 立即刷新行情 */
export function refreshQuote(): Promise<GoldQuote> {
  return invoke("refresh_quote");
}

export function listAlerts(): Promise<PriceAlert[]> {
  return invoke("list_alerts");
}

export function createAlert(input: AlertInput): Promise<PriceAlert> {
  return invoke("create_alert", { input });
}

export function updateAlert(
  id: string,
  input: AlertInput,
): Promise<PriceAlert> {
  return invoke("update_alert", { id, input });
}

export function deleteAlert(id: string): Promise<void> {
  return invoke("delete_alert", { id });
}

export function setAlertEnabled(id: string, enabled: boolean): Promise<void> {
  return invoke("set_alert_enabled", { id, enabled });
}

export function listAlertHistory(limit = 100): Promise<AlertTriggerRecord[]> {
  return invoke("list_alert_history", { limit });
}

/** 删除单条触发记录 */
export function deleteAlertHistory(id: string): Promise<void> {
  return invoke("delete_alert_history", { id });
}

/** 清空全部触发记录 */
export function clearAlertHistory(): Promise<void> {
  return invoke("clear_alert_history");
}

/** 显示项状态（悬浮行情条 / 菜单栏价格文字） */
export interface DisplayOptions {
  hudEnabled: boolean;
  trayTitleEnabled: boolean;
}

/** 读取显示项开关状态 */
export function getDisplayOptions(): Promise<DisplayOptions> {
  return invoke("get_display_options");
}

/** 设置显示项：kind = "hud" | "trayTitle" */
export function setDisplayOption(
  kind: "hud" | "trayTitle",
  enabled: boolean,
): Promise<void> {
  return invoke("set_display_option", { kind, enabled });
}

export async function loadSettings(): Promise<AppSettings> {
  const pairs: Record<string, string | null> = await invoke("get_settings", {
    keys: Object.values(SETTING_KEYS),
  });
  const rawRefresh = pairs[SETTING_KEYS.refreshIntervalSecs];
  const rawNotify = pairs[SETTING_KEYS.enableNotification];
  const rawChange = pairs[SETTING_KEYS.enableChangeDisplay];
  const rawSpread = pairs[SETTING_KEYS.estimatedSpread];
  const rawStale = pairs[SETTING_KEYS.stalenessThresholdSecs];
  return {
    refreshIntervalSecs: rawRefresh ? Number(rawRefresh) : 60,
    enableNotification: rawNotify === null ? true : rawNotify === "true",
    enableChangeDisplay: rawChange === null ? true : rawChange === "true",
    estimatedSpread: rawSpread && rawSpread !== "" ? rawSpread : null,
    stalenessThresholdSecs: rawStale ? Number(rawStale) : 300,
  };
}

export function saveSetting(key: string, value: string): Promise<void> {
  return invoke("set_setting", { key, value });
}
