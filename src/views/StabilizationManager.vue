<script setup lang="ts">
import { onMounted, reactive, ref } from "vue";
import { listen } from "@tauri-apps/api/event";
import {
  NButton,
  NCard,
  NCheckbox,
  NDivider,
  NEmpty,
  NForm,
  NFormItem,
  NInput,
  NInputNumber,
  NModal,
  NPopconfirm,
  NRadioButton,
  NRadioGroup,
  NSelect,
  NSwitch,
  NTag,
  NTimeline,
  NTimelineItem,
  useMessage,
} from "naive-ui";
import * as api from "../api/history";
import {
  STABILIZATION_STATE_META,
  type StabilizationAlert,
  type StabilizationEvent,
  type StabilizationInput,
  type StabilizationState,
} from "../types/stabilization";

const message = useMessage();

const alerts = ref<StabilizationAlert[]>([]);
const events = ref<StabilizationEvent[]>([]);

const showModal = ref(false);
const editingId = ref<string | null>(null);
const saving = ref(false);

const emptyForm = (): StabilizationInput => ({
  name: "",
  productCode: "GOLD_DEMAND",
  observationPrice: "",
  stableDays: 2,
  lowTolerance: "2.00",
  breakoutDays: 3,
  breakoutConfirmHours: 2,
  triggerMode: "ONCE",
  cooldownMinutes: 60,
  notifyStabilizing: true,
  notifyConfirmed: true,
});
const form = reactive<StabilizationInput>(emptyForm());

const stableDayOptions = [1, 2, 3, 5].map((v) => ({ label: `${v} 天`, value: v }));
const breakoutDayOptions = [2, 3, 5].map((v) => ({ label: `最近 ${v} 个有效交易日高点`, value: v }));
const confirmHourOptions = [1, 2, 3, 4].map((v) => ({
  label: v === 1 ? "连续 1 个有效小时窗口" : `连续 ${v} 个有效小时窗口`,
  value: v,
}));
const cooldownOptions = [15, 30, 60, 120, 240].map((v) => ({ label: `${v} 分钟`, value: v }));

async function reload() {
  alerts.value = await api.listStabilizationAlerts();
  events.value = await api.listStabilizationEvents(undefined, 50);
}

function openCreate() {
  editingId.value = null;
  Object.assign(form, emptyForm());
  showModal.value = true;
}

function openEdit(alert: StabilizationAlert) {
  editingId.value = alert.id;
  Object.assign(form, {
    name: alert.name,
    productCode: alert.productCode,
    observationPrice: alert.observationPrice,
    stableDays: alert.stableDays,
    lowTolerance: alert.lowTolerance,
    breakoutDays: alert.breakoutDays,
    breakoutConfirmHours: alert.breakoutConfirmHours,
    triggerMode: alert.triggerMode,
    cooldownMinutes: alert.cooldownMinutes,
    notifyStabilizing: alert.notifyStabilizing,
    notifyConfirmed: alert.notifyConfirmed,
  });
  showModal.value = true;
}

async function save() {
  const price = Number(form.observationPrice);
  const tol = Number(form.lowTolerance);
  if (!form.name.trim()) {
    message.warning("请填写规则名称");
    return;
  }
  if (!Number.isFinite(price) || price <= 0) {
    message.warning("请填写有效的观察价格");
    return;
  }
  if (!Number.isFinite(tol) || tol < 0) {
    message.warning("低点偏差必须为非负数字");
    return;
  }
  if (!form.notifyStabilizing && !form.notifyConfirmed) {
    message.warning("请至少选择一个通知阶段");
    return;
  }
  saving.value = true;
  try {
    const payload: StabilizationInput = {
      ...form,
      name: form.name.trim(),
      observationPrice: price.toFixed(2),
      lowTolerance: tol.toFixed(2),
    };
    if (editingId.value) {
      await api.updateStabilizationAlert(editingId.value, payload);
    } else {
      await api.createStabilizationAlert(payload);
    }
    showModal.value = false;
    await reload();
    message.success(editingId.value ? "规则已更新，观察重新开始" : "规则已创建");
  } catch (e) {
    message.error(`保存失败：${String(e)}`);
  } finally {
    saving.value = false;
  }
}

async function remove(id: string) {
  await api.deleteStabilizationAlert(id);
  await reload();
  message.success("规则已删除");
}

async function toggle(alert: StabilizationAlert, enabled: boolean) {
  await api.setStabilizationAlertEnabled(alert.id, enabled);
  await reload();
}

async function reset(alert: StabilizationAlert) {
  await api.resetStabilizationAlert(alert.id);
  await reload();
  message.success("已重置为等待触达状态");
}

onMounted(async () => {
  await reload();
  // 后台评估产生状态变更时刷新
  await listen("stabilization-changed", () => {
    void reload();
  });
});
</script>

<template>
  <div class="stab-manager">
    <div class="toolbar">
      <span class="toolbar-tip">观察触达 → 初步企稳 → 反弹确认，仅供参考信号</span>
      <n-button size="small" type="primary" @click="openCreate">新增企稳预警</n-button>
    </div>

    <n-empty
      v-if="alerts.length === 0"
      description="暂无企稳预警规则，点击右上角新增"
      style="margin-top: 40px"
    />

    <n-card
      v-for="alert in alerts"
      :key="alert.id"
      size="small"
      style="margin-bottom: 10px"
    >
      <div class="rule-row">
        <div class="rule-main">
          <div class="rule-title">
            <n-tag
              size="small"
              :type="STABILIZATION_STATE_META[alert.state as StabilizationState].type"
              :bordered="false"
            >
              {{ STABILIZATION_STATE_META[alert.state as StabilizationState].text }}
            </n-tag>
            <span class="rule-name">{{ alert.name }}</span>
          </div>
          <div class="rule-sub">
            观察价 ≤¥{{ alert.observationPrice }} · 观察 {{ alert.stableDays }} 天 ·
            偏差 ¥{{ alert.lowTolerance }} · 突破参考 {{ alert.breakoutDays }} 天 ·
            确认 {{ alert.breakoutConfirmHours }} 小时
          </div>
          <div class="rule-sub" v-if="alert.observationLow">
            阶段最低 ¥{{ alert.observationLow }}
            <template v-if="alert.observationStartedAt">
              · 观察开始于
              {{ new Date(alert.observationStartedAt).toLocaleString("zh-CN", { hour12: false }) }}
            </template>
          </div>
        </div>
        <div class="rule-actions">
          <n-switch
            size="small"
            :value="alert.enabled"
            @update:value="(v: boolean) => toggle(alert, v)"
          />
          <n-button size="tiny" quaternary @click="openEdit(alert)">编辑</n-button>
          <n-button size="tiny" quaternary type="warning" @click="reset(alert)">
            重置
          </n-button>
          <n-popconfirm @positive-click="remove(alert.id)">
            <template #trigger>
              <n-button size="tiny" quaternary type="error">删除</n-button>
            </template>
            确认删除该企稳规则？
          </n-popconfirm>
        </div>
      </div>
    </n-card>

    <template v-if="events.length > 0">
      <n-divider title-placement="left" style="margin-top: 20px">状态记录</n-divider>
      <n-timeline style="margin-left: 8px">
        <n-timeline-item
          v-for="ev in events"
          :key="ev.id"
          :type="ev.currentState === 'CONFIRMED' ? 'success' : ev.currentState === 'INVALIDATED' ? 'error' : 'info'"
          :title="`${ev.alertName} → ${STABILIZATION_STATE_META[ev.currentState as StabilizationState]?.text ?? ev.currentState}`"
          :content="ev.reason ?? ''"
          :time="new Date(ev.eventTime).toLocaleString('zh-CN', { hour12: false })"
        />
      </n-timeline>
    </template>

    <n-modal
      v-model:show="showModal"
      preset="card"
      :title="editingId ? '编辑企稳预警' : '新增智能企稳预警'"
      style="width: 380px"
    >
      <n-form label-placement="left" label-width="96" size="small">
        <n-form-item label="规则名称">
          <n-input v-model:value="form.name" placeholder="例如：第一轮加仓企稳确认" />
        </n-form-item>
        <n-form-item label="观察价格">
          <n-input
            :value="form.observationPrice"
            placeholder="元/克，例如 835.00"
            @update:value="(v: string) => (form.observationPrice = v)"
          >
            <template #suffix>元/克</template>
          </n-input>
        </n-form-item>
        <n-form-item label="观察交易日">
          <n-select v-model:value="form.stableDays" :options="stableDayOptions" />
        </n-form-item>
        <n-form-item label="低点偏差">
          <n-input-number
            :value="Number(form.lowTolerance)"
            :min="0"
            :step="0.5"
            style="width: 100%"
            @update:value="(v: number | null) => (form.lowTolerance = (v ?? 2).toFixed(2))"
          >
            <template #suffix>元/克</template>
          </n-input-number>
        </n-form-item>
        <n-form-item label="突破参考">
          <n-select v-model:value="form.breakoutDays" :options="breakoutDayOptions" />
        </n-form-item>
        <n-form-item label="确认方式">
          <n-select v-model:value="form.breakoutConfirmHours" :options="confirmHourOptions" />
        </n-form-item>
        <n-form-item label="通知阶段">
          <n-checkbox v-model:checked="form.notifyStabilizing">初步企稳</n-checkbox>
          <n-checkbox v-model:checked="form.notifyConfirmed">反弹确认</n-checkbox>
        </n-form-item>
        <n-form-item label="预警模式">
          <n-radio-group v-model:value="form.triggerMode">
            <n-radio-button value="ONCE">仅提醒一次</n-radio-button>
            <n-radio-button value="REARMABLE">重复提醒</n-radio-button>
          </n-radio-group>
        </n-form-item>
        <n-form-item label="冷却时间">
          <n-select v-model:value="form.cooldownMinutes" :options="cooldownOptions" />
        </n-form-item>
      </n-form>
      <div class="modal-tip">
        价格进入观察区后：连续有效交易日未明显创新低且波动收窄 → 初步企稳；
        突破近几日高点并持续保持 → 反弹确认。信号仅辅助判断，不构成交易建议。
      </div>
      <template #footer>
        <div class="modal-footer">
          <n-button size="small" @click="showModal = false">取消</n-button>
          <n-button size="small" type="primary" :loading="saving" @click="save">
            保存
          </n-button>
        </div>
      </template>
    </n-modal>
  </div>
</template>

<style scoped>
.stab-manager {
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
.modal-tip {
  margin-top: 8px;
  font-size: 12px;
  color: #999;
  line-height: 1.6;
}
.modal-footer {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}
</style>
