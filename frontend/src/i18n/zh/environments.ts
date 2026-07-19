import type { CanvasEnvSlice, EnvironmentScalarSlice } from '../slices';

// 环境相关字符串。目前仅有会话卡片上的“环境”标记，以及创建会话表单中的环境字段
// 标签；更完整的环境管理界面将在后续批次到来，届时编辑本文件（绝不触碰同级的
// dashboard/sidebar/modals 模块）。

/** 组合进仪表盘标量（`dashboard.environment`）。 */
export const environmentScalar: EnvironmentScalarSlice = {
  environment: '环境',
};

/** 与创建会话对话框一起组合进 `dashboard.canvas`。 */
export const canvasEnv: CanvasEnvSlice = {
  createEnvironmentLabel: '环境（可选）',
};
