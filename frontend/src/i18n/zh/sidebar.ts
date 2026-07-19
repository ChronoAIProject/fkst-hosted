import type { CanvasSidebarSlice } from '../slices';

// 画布“详情面板”（右侧边栏）：每一层级的引导语，以及所选节点的会话/拉取请求列表。
// 由 index 组合进 `dashboard.canvas`。归 sidebar 集群所有。
export const canvasSidebar: CanvasSidebarSlice = {
  sidebarAria: '详情面板',
  loadingSidebar: '正在加载详情…',
  viewRoot:
    '你正在查看你可触达的所有 GitHub 账户 —— 你的个人账户和你的组织。每张卡片内的圆点是它的仓库，使用相同的状态颜色。点击一个账户即可放大。',
  viewAccount: '你正在查看 {login} 的仓库。点击一个仓库即可打开它的 fkst 会话。',
  viewRepo: '你正在查看 {repo} 的 fkst 会话 —— 每个触发 issue 及其工作 issue 和拉取请求。',
  sessionsTitle: '会话',
  pollNote: '打开期间每 15 s 自动刷新。',
  sessionsFreshness: '更新于 {time}',
  sessionsRetry: '重试',
  sessionsRefreshing: '刷新中…',
  sessionsLoadFailed: '无法加载此仓库的会话，请重试。',
  sessionsStaleNotice: '刷新失败 —— 显示最近一次加载的会话。',
  notInstalledNote: '此仓库尚未安装 App，会话无法在这里运行。',
  livenessStarting: '启动中',
  livenessLive: '运行中',
  livenessTerminating: '终止中',
  logDownload: '下载日志',
  prsTitle: '拉取请求',
  prMerged: '已合并',
  prForIssue: '对应 #{n}',
  createdWord: '创建于',
  updatedWord: '更新于',
  closedWord: '关闭于',
  firstRunTitle: '开始使用 fkst',
  firstRunBody:
    '在你的账户或某个组织上安装 GitHub App，让 fkst 直接从你的 GitHub issue 运行编码会话。',
  firstRunInstall: '安装 GitHub App',
  firstRunGuide: '了解工作原理 →',
  needsInstall: '待安装',
  needsInstallHint: '在此仓库上安装 App，会话才能在这里运行。',
};
