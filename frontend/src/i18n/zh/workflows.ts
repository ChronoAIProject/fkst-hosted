import type { WorkflowsSlice } from '../slices';

export const workflows: WorkflowsSlice = {
  metaTitle: '定时工作流 — FKST',
  nav: '工作流',
  title: '定时工作流',
  loading: '正在加载定时工作流',

  gateTitle: '登录后查看你的定时工作流',
  gateBody:
    '定时工作流按节奏运行仓库里的工作流定义——运行一次，或按 cron 周期运行。用 GitHub 登录即可看到你有权访问的那些。',
  gateAction: '用 GitHub 登录',
  unconfiguredTitle: '未配置 API',
  unconfiguredBody: '此构建没有 API 基础地址，无法发起请求。请在构建时设置 `VITE_FKST_API_BASE`。',

  repoLabel: '仓库',
  repoPlaceholder: 'owner/name',
  repoHint: '要查看其定时工作流的仓库。',

  emptyTitle: '还没有定时工作流',
  emptyBody:
    '一个定时工作流就是一个 GitHub issue：用 **FKST scheduled workflow** 模板创建它，写明工作流、运行模式，并指派会话创建者。除此之外无需安装任何东西。',
  emptyAction: '在 GitHub 上打开模板',
  notInstalled: 'FKST 应用未安装在此仓库上，或者你无权查看它。',
  loadFailed: '无法加载此仓库的定时工作流。',
  retry: '重试',

  colWorkflow: '工作流',
  colCadence: '节奏',
  colNextRun: '下次运行',
  colState: '状态',
  colLastRun: '上次运行',
  colSuccess: '30 天成功率',

  lifecycle: {
    idle: '空闲',
    running: '运行中',
    paused: '已暂停',
    invalid: '无效',
  },
  runStatus: {
    dispatched: '运行中',
    ok: '成功',
    failed: '失败',
    timeout: '超时',
    'skipped-overlap': '已跳过',
  },
  stepStatus: {
    ok: '成功',
    failed: '失败',
    skipped: '未运行',
  },

  inDays: '{d} 天后',
  inHours: '{h} 小时后',
  inMinutes: '{m} 分钟后',
  imminent: '即将运行',
  overdue: '已逾期',
  never: '—',

  detailBack: '返回全部工作流',
  upcoming: '接下来的运行',
  argumentsTitle: '参数',
  noArguments: '此工作流不接受参数。',
  runsTitle: '运行记录',
  noRuns: '此工作流尚未运行过。',
  stepsTitle: '步骤',
  noSteps: '此次运行没有记录分步结果。',
  openOnGithub: '在 GitHub 上打开定义',
  runIssue: '运行 issue',
  editHint:
    '这里刻意没有编辑器：调度定义保存在它的 GitHub issue 上，并且始终可编辑。要修改节奏或参数，请编辑那个 issue。',

  actionRunNow: '立即运行',
  actionPause: '暂停',
  actionResume: '恢复',
  actionBusy: '处理中…',
  actionFailed: '操作未成功。',
  runNowStarted: '已启动。会话领取后即可看到这次运行。',

  slot: '时刻',
  duration: '耗时',
  manual: '手动',
  detailColumn: '详情',
  stepperAria: '本次运行的步骤',
  runsAria: '运行历史',
  schedulesAria: '此仓库的定时工作流',
};
