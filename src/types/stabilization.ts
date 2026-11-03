/** 企稳状态机状态 */
export type StabilizationState =
  | "WAITING_FOR_DIP"
  | "OBSERVING"
  | "STABILIZING"
  | "CONFIRMED"
  | "INVALIDATED"
  | "INSUFFICIENT_DATA";

/** 企稳预警规则 */
export interface StabilizationAlert {
  id: string;
  name: string;
  productCode: string;
  observationPrice: string;
  stableDays: number;
  lowTolerance: string;
  breakoutDays: number;
  breakoutConfirmHours: number;
  triggerMode: "ONCE" | "REARMABLE";
  cooldownMinutes: number;
  notifyStabilizing: boolean;
  notifyConfirmed: boolean;
  enabled: boolean;
  state: StabilizationState;
  observationStartedAt: string | null;
  observationLow: string | null;
  lastEvaluatedAt: string | null;
  createdAt: string;
  updatedAt: string;
}

/** 新增/编辑企稳规则的输入 */
export interface StabilizationInput {
  name: string;
  productCode: string;
  observationPrice: string;
  stableDays: number;
  lowTolerance: string;
  breakoutDays: number;
  breakoutConfirmHours: number;
  triggerMode: "ONCE" | "REARMABLE";
  cooldownMinutes: number;
  notifyStabilizing: boolean;
  notifyConfirmed: boolean;
}

/** 企稳状态变更事件 */
export interface StabilizationEvent {
  id: string;
  alertId: string;
  alertName: string;
  previousState: string | null;
  currentState: StabilizationState;
  referencePrice: string | null;
  reason: string | null;
  eventTime: string;
}

/** 企稳判断依据 */
export interface StabilizationAssessment {
  state: string;
  observationPrice: string;
  observationLow: string | null;
  validDayCount: number;
  lowStable: boolean;
  narrowing: boolean;
  breakoutPrice: string | null;
  breakoutMet: boolean;
  dataSufficient: boolean;
}

/** 状态中文描述与颜色 */
export const STABILIZATION_STATE_META: Record<
  StabilizationState,
  { text: string; type: "success" | "warning" | "info" | "error" | "default" }
> = {
  WAITING_FOR_DIP: { text: "等待触达", type: "default" },
  OBSERVING: { text: "观察中", type: "info" },
  STABILIZING: { text: "初步企稳", type: "warning" },
  CONFIRMED: { text: "反弹确认", type: "success" },
  INVALIDATED: { text: "信号失效", type: "error" },
  INSUFFICIENT_DATA: { text: "数据不足", type: "warning" },
};
