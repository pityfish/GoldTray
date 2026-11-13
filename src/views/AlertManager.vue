<script setup lang="ts">
import { onMounted, reactive, ref } from "vue";
import {
  NAlert,
  NButton,
  NCard,
  NDivider,
  NEmpty,
  NForm,
  NFormItem,
  NInput,
  NModal,
  NPopconfirm,
  NRadioGroup,
  NRadioButton,
  NRadio,
  NSelect,
  NSwitch,
  NTag,
  NTimeline,
  NTimelineItem,
  useMessage,
} from "naive-ui";
import type { AlertInput, PriceAlert } from "../types/gold";
import { useAlertStore } from "../stores/alerts";
import { useSettingsStore } from "../stores/settings";
import StabilizationManager from "./StabilizationManager.vue";

/** 预警类型页签：价格预警 / 智能企稳预警 */
const alertTab = ref<"price" | "stabilization">("price");

const alertStore = useAlertStore();
const settingsStore = useSettingsStore();
const message = useMessage();

onMounted(async () => {
  await Promise.all([alertStore.init(), settingsStore.init()]);
});

const showModal = ref(false);
const editingId = ref<string | null>(null);
const saving = ref(false);

const emptyForm = (): AlertInput => ({
  name: "",
  productCode: "GOLD_DEMAND",
  direction: "BELOW",
  priceType: "CUSTOMER_BUY",
  targetPrice: "",
  triggerMode: "ONCE",
  cooldownMinutes: 30,
});
const form = reactive<AlertInput>(emptyForm());

const directionOptions = [
  { label: "上涨达到（≥）", value: "ABOVE" },
  { label: "下跌达到（≤）", value: "BELOW" },
];
const priceTypeOptions = [
  { label: "客户买入参考价", value: "CUSTOMER_BUY" },
  { label: "客户卖出价（需价差估算）", value: "CUSTOMER_SELL" },
];
const triggerModeOptions = [
  { label: "仅提醒一次", value: "ONCE" },
  { label: "重复提醒（可重新激活）", value: "REARMABLE" },
];

function openCreate() {
  editingId.value = null;
  Object.assign(form, emptyForm());
  showModal.value = true;
}

function openEdit(alert: PriceAlert) {
  editingId.value = alert.id;
  Object.assign(form, {
    name: alert.name,
    productCode: alert.productCode,
    direction: alert.direction,
    priceType: alert.priceType,
    targetPrice: alert.targetPrice,
    triggerMode: alert.triggerMode,
    cooldownMinutes: alert.cooldownMinutes,
  });
  showModal.value = true;
}

async function save() {
  const target = Number(form.targetPrice);
  if (!form.name.trim()) {
    message.warning("请填写规则名称");
    return;
  }
  if (!Number.isFinite(target) || target <= 0) {
    message.warning("请填写有效的目标价格");
    return;
  }
  saving.value = true;
  try {
    const payload: AlertInput = {
      ...form,
      name: form.name.trim(),
      targetPrice: target.toFixed(2),
    };
    if (editingId.value) {
      await alertStore.update(editingId.value, payload);
    } else {
      await alertStore.create(payload);
    }
    showModal.value = false;
    message.success(editingId.value ? "规则已更新" : "规则已创建");
  } catch (e) {
    message.error(`保存失败：${String(e)}`);
  } finally {
    saving.value = false;
  }
}

async function remove(id: string) {
  await alertStore.remove(id);
  message.success("规则已删除，其触发记录已保留");
}

async function clearAllHistory() {
  await alertStore.clearHistory();
  message.success("触发记录已清空");
}

async function toggle(alert: PriceAlert, enabled: boolean) {
  await alertStore.toggle(alert.id, enabled);
}

/** 规则当前监控状态描述 */
function ruleStatus(alert: PriceAlert): { text: string; type: "success" | "warning" | "info" | "default" } {
  if (!alert.enabled) return { text: "已停用", type: "default" };
  if (alert.priceType === "CUSTOMER_SELL" && !settingsStore.settings.estimatedSpread) {
    return { text: "无法准确监控（未设置价差估算）", type: "warning" };
  }
  if (!alert.armed) return { text: "已处于目标区间，等待重新激活", type: "info" };
  return { text: "监控中", type: "success" };
}

function directionText(alert: PriceAlert) {
  return alert.direction === "ABOVE" ? "↑" : "↓";
}

function priceTypeText(alert: PriceAlert) {
  return alert.priceType === "CUSTOMER_BUY" ? "买入参考价" : "卖出价（估算）";
}

/** 卖出价规则换算后的等效监控值 */
function effectiveTarget(alert: PriceAlert): string {
  if (alert.priceType !== "CUSTOMER_SELL") return alert.targetPrice;
  const spread = Number(settingsStore.settings.estimatedSpread ?? "0");
  const t = Number(alert.targetPrice) + spread;
  return Number.isFinite(t) ? t.toFixed(2) : alert.targetPrice;
}

</script>

<template>
  <div class="alert-manager">
    <n-radio-group
      v-model:value="alertTab"
      size="small"
      style="display: flex; margin-bottom: 10px"
    >
      <n-radio-button value="price">价格预警</n-radio-button>
      <n-radio-button value="stabilization">智能企稳</n-radio-button>
    </n-radio-group>

    <template v-if="alertTab === 'price'">
    <div class="toolbar">
      <span class="toolbar-tip">价格穿越触发，连续两次有效行情确认</span>
      <n-button size="small" type="primary" @click="openCreate">新增预警</n-button>
    </div>

    <n-empty
      v-if="alertStore.alerts.length === 0"
      description="暂无预警规则，点击右上角新增"
      style="margin-top: 48px"
    />

    <n-card
      v-for="alert in alertStore.alerts"
      :key="alert.id"
      size="small"
      style="margin-bottom: 10px"
    >
      <div class="rule-row">
        <div class="rule-main">
          <div class="rule-title">
            <n-tag
              size="small"
              :type="alert.direction === 'ABOVE' ? 'error' : 'success'"
              :bordered="false"
            >
              {{ directionText(alert) }} ¥{{ alert.targetPrice }}
            </n-tag>
            <span class="rule-name">{{ alert.name }}</span>
          </div>
          <div class="rule-sub">
            {{ priceTypeText(alert) }}
            <template v-if="alert.priceType === 'CUSTOMER_SELL'">
              · 等效监控值 ¥{{ effectiveTarget(alert) }}
            </template>
            · {{ alert.triggerMode === "ONCE" ? "仅一次" : "重复" }}
            · 冷却 {{ alert.cooldownMinutes }} 分钟
          </div>
          <n-tag size="tiny" :type="ruleStatus(alert).type" :bordered="false">
            {{ ruleStatus(alert).text }}
          </n-tag>
        </div>
        <div class="rule-actions">
          <n-switch
            size="small"
            :value="alert.enabled"
            @update:value="(v: boolean) => toggle(alert, v)"
          />
          <n-button size="tiny" quaternary @click="openEdit(alert)">编辑</n-button>
          <n-popconfirm @positive-click="remove(alert.id)">
            <template #trigger>
              <n-button size="tiny" quaternary type="error">删除</n-button>
            </template>
            确认删除该预警规则？
          </n-popconfirm>
        </div>
      </div>
    </n-card>

    <template v-if="alertStore.history.length > 0">
      <div class="history-header">
        <n-divider title-placement="left" style="flex: 1; margin-bottom: 0">
          触发记录
        </n-divider>
        <n-popconfirm @positive-click="clearAllHistory">
          <template #trigger>
            <n-button size="tiny" quaternary type="error">清空</n-button>
          </template>
          确认清空全部触发记录？（不影响预警规则）
        </n-popconfirm>
      </div>
      <n-timeline style="margin-left: 8px">
        <n-timeline-item
          v-for="record in alertStore.history"
          :key="record.id"
          :content="`${record.historicalFrom ? '时段极值' : '触发价'} ¥${record.triggeredPrice} / 目标 ¥${record.targetPrice}`"
          :time="record.historicalFrom
            ? `${new Date(record.historicalFrom).toLocaleString('zh-CN', { hour12: false })} ～ ${new Date(record.triggeredAt).toLocaleTimeString('zh-CN', { hour12: false })}`
            : new Date(record.triggeredAt).toLocaleString('zh-CN', { hour12: false })"
        >
          <template #header>
            {{ record.alertName }}（{{ record.historicalFrom ? '离线补记 · ' : '' }}{{ record.isEstimated ? '估算提醒' : '参考价' }}）
          </template>
        </n-timeline-item>
      </n-timeline>
    </template>

    <n-modal
      v-model:show="showModal"
      preset="card"
      :title="editingId ? '编辑价格预警' : '新增价格预警'"
      style="width: 360px"
    >
      <n-form label-placement="left" label-width="82" size="small">
        <n-form-item label="规则名称">
          <n-input v-model:value="form.name" placeholder="例如：第一轮加仓观察" />
        </n-form-item>
        <n-form-item label="监控价格">
          <n-radio-group v-model:value="form.priceType">
            <n-radio
              v-for="opt in priceTypeOptions"
              :key="opt.value"
              :value="opt.value"
            >
              {{ opt.label }}
            </n-radio>
          </n-radio-group>
        </n-form-item>
        <n-form-item label="预警方向">
          <n-radio-group v-model:value="form.direction">
            <n-radio
              v-for="opt in directionOptions"
              :key="opt.value"
              :value="opt.value"
            >
              {{ opt.label }}
            </n-radio>
          </n-radio-group>
        </n-form-item>
        <n-form-item label="目标价格">
          <n-input
            :value="form.targetPrice"
            placeholder="元/克，例如 835.00"
            @update:value="(v: string) => (form.targetPrice = v)"
          >
            <template #suffix>元/克</template>
          </n-input>
        </n-form-item>
        <n-form-item label="预警方式">
          <n-radio-group v-model:value="form.triggerMode">
            <n-radio
              v-for="opt in triggerModeOptions"
              :key="opt.value"
              :value="opt.value"
            >
              {{ opt.label }}
            </n-radio>
          </n-radio-group>
        </n-form-item>
        <n-form-item label="冷却时间">
          <n-select
            :value="form.cooldownMinutes"
            :options="[
              { label: '5 分钟', value: 5 },
              { label: '15 分钟', value: 15 },
              { label: '30 分钟', value: 30 },
              { label: '60 分钟', value: 60 },
              { label: '120 分钟', value: 120 },
            ]"
            @update:value="(v: number) => (form.cooldownMinutes = v)"
          />
        </n-form-item>
      </n-form>
      <n-alert
        v-if="form.priceType === 'CUSTOMER_SELL'"
        type="warning"
        :bordered="false"
        size="small"
      >
        当前数据源仅提供买入参考价，将按
        「目标价 + 价差估算」换算为等效买入价监控，
        通知中会提示您到银行 App 核对实际卖出价。
      </n-alert>
      <template #footer>
        <div class="modal-footer">
          <n-button size="small" @click="showModal = false">取消</n-button>
          <n-button size="small" type="primary" :loading="saving" @click="save">
            保存
          </n-button>
        </div>
      </template>
    </n-modal>
    </template>

    <StabilizationManager v-else />
  </div>
</template>

<style scoped>
.alert-manager {
  padding-top: 8px;
}
.toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 10px;
}
.toolbar-tip {
  font-size: 12px;
  color: #999;
}
.rule-row {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 8px;
}
.rule-title {
  display: flex;
  align-items: center;
  gap: 8px;
}
.rule-name {
  font-weight: 600;
}
.rule-sub {
  margin: 4px 0;
  font-size: 12px;
  color: #999;
}
.rule-actions {
  display: flex;
  align-items: center;
  gap: 4px;
  flex-shrink: 0;
}
.history-header {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 20px;
}
.modal-footer {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}
</style>
