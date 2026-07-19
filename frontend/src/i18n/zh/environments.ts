import type { CanvasEnvSlice, EnvironmentScalarSlice } from '../slices';
// The manager dictionary implements the English file's contract type, so
// TypeScript flags any key that drifts out of sync between the two locales.
import type { EnvManagerStrings } from '../en/environments';

// 环境相关字符串。本文件服务两类使用者：
//  1. 组合进共享 `SiteContent` 目录的少量“环境”标记/字段字符串
//     （`canvasEnv` / `environmentScalar`，由 `zh.ts` 使用）——必须保持其切片形状。
//  2. 完整的环境管理抽屉（`environmentsManager`）——一个自包含的字典，由管理器组件
//     通过 `useLang()` 直接读取；它不接入 `SiteContent`，因此新增内容不会牵动由其他
//     工作项拥有的共享 `types.ts` / `en.ts` / `zh.ts`。

/** 组合进仪表盘标量（`dashboard.environment`）。 */
export const environmentScalar: EnvironmentScalarSlice = {
  environment: '环境',
};

/** 与创建会话对话框一起组合进 `dashboard.canvas`。 */
export const canvasEnv: CanvasEnvSlice = {
  createEnvironmentLabel: '环境（可选）',
};

/** 中文环境管理字典。占位符 `{name}` / `{n}` / `{max}` / `{time}` 在调用处替换。 */
export const environmentsManager: EnvManagerStrings = {
  title: '环境',
  close: '关闭',
  closeAria: '关闭环境',
  back: '返回',
  backAria: '返回环境列表',
  newEnvironment: '新建环境',

  listLoading: '正在加载环境…',
  listLoadFailed: '无法加载你的环境。',
  listEmpty: '还没有任何环境。',
  listEmptyHint: '创建一个环境，以在多个会话间复用安装步骤、变量和密钥。',
  retry: '重试',
  validatedAt: '已验证 {time}',
  neverValidated: '未验证',
  installCount: '{n} 条安装命令',
  variableCount: '{n} 个变量',
  secretCount: '{n} 个密钥',
  openAria: '打开环境 {name}',

  editorCreateTitle: '新建环境',
  editorEditTitle: '编辑环境',
  nameLabel: '名称',
  namePlaceholder: 'my-environment',
  nameHint: '仅限小写字母、数字和连字符——用于构造存储对象名。',
  nameLockedHint: '创建后名称不可更改。',
  nameErrorFormat: '请使用小写字母、数字和连字符（不能位于首尾）。',
  nameErrorLength: '名称最多 {max} 个字符。',
  installLegend: '安装命令',
  installPlaceholder: 'pip install -r requirements.txt',
  installHint: '保存时将在一个临时 Pod 中按顺序执行。',
  addInstall: '添加命令',
  removeInstallAria: '删除第 {n} 条安装命令',
  variablesLegend: '变量',
  variableNamePlaceholder: 'NAME',
  variableValuePlaceholder: 'value',
  addVariable: '添加变量',
  removeVariableAria: '删除第 {n} 个变量',
  secretsLegend: '密钥',
  secretNamePlaceholder: 'NAME',
  secretValuePlaceholder: 'value（只写）',
  secretsHint: '密钥值为只写——保存后将不再显示。',
  secretsEditHint: '请重新输入每个密钥值；保存时留空的密钥会被删除。',
  addSecret: '添加密钥',
  removeSecretAria: '删除第 {n} 个密钥',
  validatingNote: '正在隔离 Pod 中验证安装命令……这可能需要一些时间。',
  save: '保存',
  saving: '保存中…',
  cancel: '取消',
  saveFailed: '无法保存环境。',
  validationTitle: '安装验证失败',
  validationCommand: '失败的命令',
  validationIndex: '命令序号',
  validationExitCode: '退出码',
  validationTimedOut: '已超时',
  validationStderr: 'stderr（末尾）',
  savedToast: '环境“{name}”已保存。',

  detailLoading: '正在加载环境…',
  detailLoadFailed: '无法加载该环境。',
  statusLabel: '状态',
  validatedLabel: '已验证',
  installTitle: '安装命令',
  installEmpty: '没有安装命令。',
  variablesTitle: '变量',
  variablesEmpty: '没有变量。',
  secretsTitle: '密钥',
  secretsEmpty: '没有密钥。',
  secretsValueNote: '密钥值已隐藏且永不返回。',
  edit: '编辑',
  deleteButton: '删除',
  deleteConfirmTitle: '删除环境？',
  deleteConfirmBody: '删除“{name}”？引用它的会话将无法再找到它。此操作不可撤销。',
  deleteConfirm: '删除',
  deletePending: '删除中…',
  deleteCancel: '取消',
  deleteFailed: '无法删除环境。',
  deletedToast: '环境“{name}”已删除。',

  yes: '是',
  no: '否',
};
