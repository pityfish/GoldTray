import { defineStore } from "pinia";
import { ref } from "vue";
import { listen } from "@tauri-apps/api/event";
import type { AlertTriggeredEvent, AlertTriggerRecord, PriceAlert } from "../types/gold";
import * as api from "../api/goldtray";

export const useAlertStore = defineStore("alerts", () => {
  const alerts = ref<PriceAlert[]>([]);
  const history = ref<AlertTriggerRecord[]>([]);
  const loading = ref(false);
  let initialization: Promise<void> | undefined;

  async function reloadAlerts() {
    alerts.value = await api.listAlerts();
  }

  async function reloadHistory() {
    history.value = await api.listAlertHistory();
  }

  async function load() {
    const unlisteners: Array<() => void> = [];
    try {
      unlisteners.push(await listen<AlertTriggeredEvent>("alert-triggered", () => {
        // 触发后同步刷新规则状态与触发记录
        void reloadAlerts();
        void reloadHistory();
      }));
      unlisteners.push(await listen("alert-state-changed", () => {
        void reloadAlerts();
      }));
      const [rules, records] = await Promise.all([
        api.listAlerts(),
        api.listAlertHistory(),
      ]);
      alerts.value = rules;
      history.value = records;
    } catch (error) {
      await Promise.allSettled(unlisteners.map((unlisten) => Promise.resolve().then(unlisten)));
      throw error;
    }
  }

  function init() {
    return (initialization ??= load().catch((error: unknown) => {
      initialization = undefined;
      throw error;
    }));
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
    clearHistory,
  };
});
