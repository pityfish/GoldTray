<script setup lang="ts">
import { onMounted, ref } from "vue";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import {
  NButton,
  NCard,
  NDivider,
  NForm,
  NFormItem,
  NInput,
  NSelect,
  NSlider,
  NSwitch,
  useMessage,
} from "naive-ui";
import { getVersion } from "@tauri-apps/api/app";
import {
  disable,
  enable,
  isEnabled,
} from "@tauri-apps/plugin-autostart";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { DisplayOptions } from "../api/goldtray";
import * as api from "../api/goldtray";
import { useSettingsStore } from "../stores/settings";

const settingsStore = useSettingsStore();
const message = useMessage();

const spreadInput = ref<string>("");

// ---------- 显示项（悬浮行情条 / 菜单栏价格文字） ----------
const displayOptions = ref<DisplayOptions>({
  hudEnabled: false,
  trayTitleEnabled: true,
});

async function reloadDisplayOptions() {
  displayOptions.value = await api.getDisplayOptions();
}

async function onDisplayChange(
  kind: "hud" | "trayTitle",
  value: boolean,
) {
  try {
    await api.setDisplayOption(kind, value);
    await reloadDisplayOptions();
  } catch (e) {
    message.error(`设置显示项失败：${String(e)}`);
  }
}

// ---------- 悬浮条透明度 ----------
const hudOpacityPercent = ref<number>(95);
let opacitySaveTimer: ReturnType<typeof setTimeout> | null = null;

async function reloadHudOpacity() {
  try {
    const pairs: Record<string, string | null> = await invoke("get_settings", {
      keys: ["hud_opacity"],
    });
    const v = Number(pairs["hud_opacity"] ?? NaN);
    hudOpacityPercent.value = Number.isFinite(v)
      ? Math.round(Math.min(1, Math.max(0.3, v)) * 100)
      : 95;
  } catch {
    hudOpacityPercent.value = 95;
  }
}

/** 拖动中先更新本地值（受控组件，否则滑块回弹），停止 250ms 后再持久化广播 */
function onHudOpacityChange(v: number) {
  hudOpacityPercent.value = v;
  if (opacitySaveTimer) {
    clearTimeout(opacitySaveTimer);
  }
  opacitySaveTimer = setTimeout(() => {
    void api
      .saveSetting("hud_opacity", (v / 100).toFixed(2))
      .catch((e) => message.error(`保存透明度失败：${String(e)}`));
  }, 250);
}

// ---------- 开机自启 ----------
const autostartEnabled = ref(false);

async function onAutostartChange(v: boolean) {
  try {
    if (v) {
      await enable();
    } else {
      await disable();
    }
    autostartEnabled.value = v;
    message.success(v ? "已开启开机自启" : "已关闭开机自启");
  } catch (e) {
    message.error(`设置开机自启失败：${String(e)}`);
  }
}

// ---------- 检查更新 ----------
const REPO_URL = "https://github.com/pityfish/GoldTray";
const currentVersion = ref("");
const checking = ref(false);

/** 语义化版本比较：返回 >0 表示 a 更新 */
function cmpVersion(a: string, b: string): number {
  const pa = a.split(".").map(Number);
  const pb = b.split(".").map(Number);
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
    const d = (pa[i] ?? 0) - (pb[i] ?? 0);
    if (d !== 0) return d;
  }
  return 0;
}

async function checkUpdate() {
  checking.value = true;
  try {
    const resp = await fetch(`${REPO_URL}/releases/latest`, {
      headers: { Accept: "application/vnd.github+json" },
    });
    if (!resp.ok) {
      throw new Error(`GitHub 返回 ${resp.status}`);
    }
    const data = (await resp.json()) as { tag_name?: string };
    const latest = String(data.tag_name ?? "").replace(/^v/, "");
    if (!latest) {
      throw new Error("响应中没有版本号");
    }
    if (cmpVersion(latest, currentVersion.value) > 0) {
      message.info(`发现新版本 v${latest}，正在打开下载页面`);
      await openUrl(`${REPO_URL}/releases/latest`);
    } else {
      message.success(`已是最新版本 v${currentVersion.value}`);
    }
  } catch {
    message.warning(
      "检查更新失败（可能是网络原因，或尚未发布过任何版本）。可手动访问 GitHub Releases 页面查看。",
    );
  } finally {
    checking.value = false;
  }
}

onMounted(async () => {
  await settingsStore.init();
  spreadInput.value = settingsStore.settings.estimatedSpread ?? "";
  currentVersion.value = await getVersion();
  try {
    autostartEnabled.value = await isEnabled();
  } catch {
    // dev 模式或系统限制下可能读取失败，保持默认关闭
    autostartEnabled.value = false;
  }
  await reloadDisplayOptions();
  await reloadHudOpacity();
  // 托盘菜单勾选变更时同步刷新（后端广播）
  await listen("display-options-changed", () => {
    void reloadDisplayOptions();
  });
});

const intervalOptions = [
  { label: "30 秒", value: 30 },
  { label: "60 秒（默认）", value: 60 },
  { label: "120 秒", value: 120 },
  { label: "300 秒", value: 300 },
];

const stalenessOptions = [
  { label: "2 分钟", value: 120 },
  { label: "5 分钟（默认）", value: 300 },
  { label: "10 分钟", value: 600 },
  { label: "30 分钟", value: 1800 },
];

async function onIntervalChange(v: number) {
  await settingsStore.save("refreshIntervalSecs", v);
  message.success("刷新间隔已更新，将于下个周期生效");
}

async function onNotifyChange(v: boolean) {
  await settingsStore.save("enableNotification", v);
}

async function onChangeDisplayChange(v: boolean) {
  await settingsStore.save("enableChangeDisplay", v);
}

async function onStalenessChange(v: number) {
  await settingsStore.save("stalenessThresholdSecs", v);
}

async function onSpreadBlur() {
  const v = spreadInput.value.trim();
  if (v === "") {
    await settingsStore.save("estimatedSpread", null);
    message.success("已关闭卖出价差估算提醒");
    return;
  }
  const n = Number(v);
  if (!Number.isFinite(n) || n < 0) {
    message.warning("价差必须为非负数字");
    spreadInput.value = settingsStore.settings.estimatedSpread ?? "";
    return;
  }
  await settingsStore.save("estimatedSpread", n.toFixed(2));
  message.success(`已启用价差估算：${n.toFixed(2)} 元/克`);
}
</script>

<template>
  <div class="settings-view">
    <n-card size="small" title="通用设置">
      <n-form label-placement="left" label-width="110" size="small">
        <n-form-item label="开机自启">
          <n-switch
            :value="autostartEnabled"
            @update:value="onAutostartChange"
          />
        </n-form-item>
      </n-form>
    </n-card>

    <n-card size="small" title="显示设置" style="margin-top: 12px">
      <n-form label-placement="left" label-width="110" size="small">
        <n-form-item label="悬浮行情条">
          <n-switch
            :value="displayOptions.hudEnabled"
            @update:value="(v: boolean) => onDisplayChange('hud', v)"
          />
        </n-form-item>
        <n-form-item label="菜单栏显示">
          <n-switch
            :value="displayOptions.trayTitleEnabled"
            @update:value="(v: boolean) => onDisplayChange('trayTitle', v)"
          />
        </n-form-item>
        <n-form-item label="悬浮条透明度">
          <n-slider
            :value="hudOpacityPercent"
            :min="30"
            :max="100"
            :step="5"
            :format-tooltip="(v: number) => `${v}%`"
            @update:value="onHudOpacityChange"
          />
        </n-form-item>
      </n-form>
      <div class="card-tip">
        悬浮行情条：桌面置顶小条持续显示金价，可拖动、拖边缘调整大小（Windows 上推荐开启）。
        菜单栏显示：macOS 菜单栏图标旁显示「Au ¥价格」（仅 macOS 生效）。
        两项均与托盘右键菜单的勾选状态联动。
      </div>
    </n-card>

    <n-card size="small" title="行情设置" style="margin-top: 12px">
      <n-form label-placement="left" label-width="110" size="small">
        <n-form-item label="刷新频率">
          <n-select
            :value="settingsStore.settings.refreshIntervalSecs"
            :options="intervalOptions"
            @update:value="onIntervalChange"
          />
        </n-form-item>
        <n-form-item label="显示涨跌金额">
          <n-switch
            :value="settingsStore.settings.enableChangeDisplay"
            @update:value="onChangeDisplayChange"
          />
        </n-form-item>
        <n-form-item label="数据过期阈值">
          <n-select
            :value="settingsStore.settings.stalenessThresholdSecs"
            :options="stalenessOptions"
            @update:value="onStalenessChange"
          />
        </n-form-item>
      </n-form>
    </n-card>

    <n-card size="small" title="通知设置" style="margin-top: 12px">
      <n-form label-placement="left" label-width="110" size="small">
        <n-form-item label="开启原生通知">
          <n-switch
            :value="settingsStore.settings.enableNotification"
            @update:value="onNotifyChange"
          />
        </n-form-item>
      </n-form>
    </n-card>

    <n-card size="small" title="卖出价差估算" style="margin-top: 12px">
      <n-form label-placement="left" label-width="110" size="small">
        <n-form-item label="价差（元/克）">
          <n-input
            v-model:value="spreadInput"
            placeholder="留空关闭；例如 5.00"
            @blur="onSpreadBlur"
          />
        </n-form-item>
      </n-form>
      <div class="card-tip">
        当前行情源仅提供客户买入参考价。填写价差后，客户卖出价预警将按
        「目标卖出价 + 价差」换算为等效买入价进行监控，
        通知中会提示您到银行 App 核对实际价格，不代表真实可成交价。
      </div>
    </n-card>

    <n-card size="small" title="关于" style="margin-top: 12px">
      <div class="about-row">
        <span>GoldTray v{{ currentVersion || "…" }}</span>
        <n-button
          size="tiny"
          secondary
          :loading="checking"
          @click="checkUpdate"
        >
          检查更新
        </n-button>
      </div>
      <div class="card-tip" style="margin-top: 8px">
        数据来源：汇率速递（第三方参考行情，非银行官方实时成交价）。
        本应用仅提供行情展示与提醒，不构成投资建议，不对盈利作出保证。
      </div>
    </n-card>

    <n-divider />
  </div>
</template>

<style scoped>
.settings-view {
  padding-top: 8px;
}
.card-tip {
  font-size: 12px;
  color: #999;
  line-height: 1.6;
}
.about-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-weight: 600;
}
</style>
