import type { WorkflowsSlice } from '../slices';

export const workflows: WorkflowsSlice = {
  loading: '正在加载定时工作流',

  emptyTitle: '此会话还没有定时工作流',
  emptyBody:
    '一个定时工作流就是一个 GitHub issue：用 **FKST scheduled workflow** 模板创建它，写明工作流和运行模式，并指派本会话的创建者。除此之外无需安装任何东西。',
  emptyAction: '在 GitHub 上打开模板',
  notInstalled: 'FKST 应用未安装在此仓库上，或者你无权查看它。',
  loadFailed: '无法加载此仓库的定时工作流。',
  retry: '重试',

  railTitle: '调度',
  railAria: '本会话拥有的定时工作流',
  unroutedTitle: '未路由到任何会话',
  unroutedBody:
    '只有当调度恰好有一位受指派人是会话创建者时，它才会运行。这些调度没有受指派人，或者有多位，因此不会被任何会话运行。',
  unroutedOnly: '目前没有任何调度路由到本会话。',

  cadenceLabel: '节奏',
  successLabel: '30 天成功率',

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

  upcoming: '接下来的运行',
  argumentsTitle: '参数',
  noArguments: '此工作流不接受参数。',
  latestRunTitle: '最近一次运行',
  earlierRunsTitle: '更早的运行',
  noRuns: '此工作流尚未运行过。',
  noSteps: '此次运行没有记录分步结果。',
  awaitingSteps: '正在等待第一条步骤记录——运行结束时才会上报分步结果。',
  runningFor: '已运行 {d}',
  openOnGithub: '在 GitHub 上打开定义',
  openRunIssue: '运行 issue',
  editHint:
    '这里刻意没有编辑器：调度定义保存在它的 GitHub issue 上，并且始终可编辑。要修改节奏或参数，请编辑那个 issue。',

  actionRunNow: '立即运行',
  actionPause: '暂停',
  actionResume: '恢复',
  actionBusy: '处理中…',
  actionFailed: '操作未成功。',

  manual: '手动',
  stepperAria: '本次运行的步骤',
  runsAria: '更早的运行',
};
