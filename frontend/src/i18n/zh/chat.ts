/** 聊天助理文案：启动器、面板框架、对话记录、输入框。 */
export const chat = {
  launcherLabel: '智能助理',
  launcherAria: '打开 fkst 智能助理',
  launcherCloseAria: '关闭 fkst 智能助理',

  panelTitle: 'FKST // 智能助理',
  panelAria: 'fkst 智能助理',
  linkActive: '连接正常',
  streaming: '生成中',
  clear: '清空',
  clearAria: '清空对话',
  close: '关闭',
  closeAria: '关闭助理面板',

  welcomeTitle: '询问你的会话',
  welcomeBody:
    '我可以查看正在运行的会话、读取失败的日志，并解释平台的运作方式 —— 只使用你有权访问的内容。',
  starters: {
    running: '有哪些会话正在运行？',
    unrouted: '我的 issue 为什么没有被路由？',
    start: '如何启动一个会话？',
  },

  transcriptAria: '对话',
  jumpToLatest: '跳到最新',
  assistantRole: '助理',
  userRole: '你',
  copyAnswer: '复制',
  answerAria: '助理回答',
  activityToggle: '活动记录',
  toolRunning: '执行中',
  toolOk: '成功',
  toolDenied: '无权限',
  toolError: '错误',
  toolTruncated: '已截断',

  placeholder: '询问某个会话、日志，或某项功能的原理……',
  inputAria: '向助理发送消息',
  send: '发送',
  sendAria: '发送消息',
  stop: '停止',
  stopAria: '停止当前回答',
  charCount: '{used} / {max}',

  signInTitle: '登录后使用智能助理',
  signInBody: '助理使用你自己的 GitHub 权限作答，因此需要先登录。',
} as const;
