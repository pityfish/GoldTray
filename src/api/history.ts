import { invoke } from "@tauri-apps/api/core";
import type {
  StabilizationAlert,
  StabilizationAssessment,
  StabilizationEvent,
  StabilizationInput,
} from "../types/stabilization";
import type { HistoryPayload, HistoryPeriodValue } from "../types/history";

/** 获取历史行情（Rust 侧缓存优先，网络异常降级旧缓存） */
export function getHistory(
  period: HistoryPeriodValue,
  force = false,
): Promise<HistoryPayload> {
  return invoke("get_history", { period, force });
}

export function listStabilizationAlerts(): Promise<StabilizationAlert[]> {
  return invoke("list_stabilization_alerts");
}

export function createStabilizationAlert(
  input: StabilizationInput,
): Promise<StabilizationAlert> {
  return invoke("create_stabilization_alert", { input });
}

export function updateStabilizationAlert(
  id: string,
  input: StabilizationInput,
): Promise<StabilizationAlert> {
  return invoke("update_stabilization_alert", { id, input });
}

export function deleteStabilizationAlert(id: string): Promise<void> {
  return invoke("delete_stabilization_alert", { id });
}

export function setStabilizationAlertEnabled(
  id: string,
  enabled: boolean,
): Promise<void> {
  return invoke("set_stabilization_alert_enabled", { id, enabled });
}

/** 手动重置企稳规则状态 */
export function resetStabilizationAlert(id: string): Promise<void> {
  return invoke("reset_stabilization_alert", { id });
}

export function listStabilizationEvents(
  alertId?: string,
  limit = 100,
): Promise<StabilizationEvent[]> {
  return invoke("list_stabilization_events", { alertId: alertId ?? null, limit });
}

export function clearStabilizationEvents(): Promise<void> {
  return invoke("clear_stabilization_events");
}

/** 获取企稳判断依据 */
export function assessStabilization(
  id: string,
): Promise<StabilizationAssessment> {
  return invoke("assess_stabilization", { id });
}
