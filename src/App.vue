<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import {
  NConfigProvider,
  NMessageProvider,
  NTab,
  NTabs,
  darkTheme,
  zhCN,
  dateZhCN,
} from "naive-ui";
import { useOsTheme } from "naive-ui";
import Dashboard from "./views/Dashboard.vue";
import AlertManager from "./views/AlertManager.vue";
import SettingsView from "./views/SettingsView.vue";

const osTheme = useOsTheme();
const theme = computed(() => (osTheme.value === "dark" ? darkTheme : null));

const tab = ref<"dashboard" | "alerts" | "settings">("dashboard");

onMounted(() => {
  // 预警状态在 AlertManager 内部初始化
});
</script>

<template>
  <n-config-provider
    :theme="theme"
    :locale="zhCN"
    :date-locale="dateZhCN"
    style="height: 100vh; display: flex; flex-direction: column"
  >
    <n-message-provider>
      <header class="app-header">
        <n-tabs
          v-model:value="tab"
          type="segment"
          size="small"
          style="padding: 8px 12px 4px"
        >
          <n-tab name="dashboard" tab="行情" />
          <n-tab name="alerts" tab="预警" />
          <n-tab name="settings" tab="设置" />
        </n-tabs>
      </header>
      <main class="app-main">
        <Dashboard v-if="tab === 'dashboard'" />
        <AlertManager v-else-if="tab === 'alerts'" />
        <SettingsView v-else />
      </main>
    </n-message-provider>
  </n-config-provider>
</template>

<style scoped>
.app-main {
  flex: 1;
  overflow-y: auto;
  padding: 0 12px 12px;
}
</style>
