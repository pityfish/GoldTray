<script setup lang="ts">
import { onMounted, ref } from "vue";
import {
  NCard,
  NDivider,
  NForm,
  NFormItem,
  NInput,
  NSelect,
  NSwitch,
  useMessage,
} from "naive-ui";
import { useSettingsStore } from "../stores/settings";

const settingsStore = useSettingsStore();
const message = useMessage();

const spreadInput = ref<string>("");

onMounted(async () => {
  await settingsStore.init();
  spreadInput.value = settingsStore.settings.estimatedSpread ?? "";
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
    <n-card size="small" title="行情设置">
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

    <n-divider />
    <div class="about">
      <div>GoldTray v0.1.0</div>
      <div class="about-sub">
        数据来源：汇率速递（第三方参考行情，非银行官方实时成交价）。
        本应用仅提供行情展示与提醒，不构成投资建议，不对盈利作出保证。
      </div>
    </div>
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
.about {
  text-align: center;
  color: #999;
  font-size: 12px;
}
.about-sub {
  margin-top: 6px;
  line-height: 1.6;
}
</style>
