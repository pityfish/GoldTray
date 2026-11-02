import { defineStore } from "pinia";
import { ref } from "vue";
import type { AppSettings } from "../types/gold";
import { SETTING_KEYS } from "../types/gold";
import * as api from "../api/goldtray";

export const useSettingsStore = defineStore("settings", () => {
  const settings = ref<AppSettings>({
    refreshIntervalSecs: 60,
    enableNotification: true,
    enableChangeDisplay: true,
    estimatedSpread: null,
    stalenessThresholdSecs: 300,
  });
  let initialized = false;

  async function init() {
    if (initialized) return;
    initialized = true;
    settings.value = await api.loadSettings();
  }

  async function save<K extends keyof AppSettings>(
    key: K,
    value: AppSettings[K],
  ) {
    // SETTING_KEYS 的键与 AppSettings 字段名一一对应
    const settingKey = SETTING_KEYS[key];
    // null（关闭估算）存为空字符串
    const stored = value === null ? "" : String(value);
    await api.saveSetting(settingKey, stored);
    settings.value = { ...settings.value, [key]: value };
  }

  return { settings, init, save };
});
