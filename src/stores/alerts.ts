import { defineStore } from "pinia";
import { ref } from "vue";
import { listen } from "@tauri-apps/api/event";
import type { AlertTriggeredEvent, AlertTriggerRecord, PriceAlert } from "../types/gold";
import * as api from "../api/goldtray";

export const useAlertStore = defineStore("alerts", () => {
  const alerts = ref<PriceAlert[]>([]);
  const history = ref<AlertTriggerRecord[]>([]);
  const loading = ref(false);
  let initialized = false;

  async function reloadAlerts() {
    alerts.value = await api.listAlerts();
  }

  async function reloadHistory() {
    history.value = await api.listAlertHistory();
  }

  async function init() {
    if (initialized) return;
    initialized = true;
    await listen<AlertTriggeredEvent>("alert-triggered", () => {
      // 触发后同步刷新规则状态与触发记录
      void reloadAlerts();
      void reloadHistory();
    });
    await Promise.all([reloadAlerts(), reloadHistory()]);
  }

  async function create(input: Parameters<typeof api.createAlert>[0]) {
    await api.createAlert(input);
    await reloadAlerts();
  }

  async function update(id: string, input: Parameters<typeof api.updateAlert>[1]) {
    await api.updateAlert(id, input);
    await reloadAlerts();
  }

  async function remove(id: string) {
    await api.deleteAlert(id);
    await Promise.all([reloadAlerts(), reloadHistory()]);
  }

  /** 删除单条触发记录（不影响规则） */
  async function removeHistory(id: string) {
    await api.deleteAlertHistory(id);
    await reloadHistory();
  }

  /** 清空全部触发记录（不影响规则） */
  async function clearHistory() {
    await api.clearAlertHistory();
    await reloadHistory();
  }

  async function toggle(id: string, enabled: boolean) {
    await api.setAlertEnabled(id, enabled);
    await reloadAlerts();
  }

  return {
    alerts,
    history,
    loading,
    init,
    reloadAlerts,
    reloadHistory,
    create,
    update,
    remove,
    toggle,
    removeHistory,
    clearHistory,
  };
});
