import type { CommonSlice } from '../slices';

// 每个页面共享的界面外壳：语言切换、认证操作、页脚。
export const common: CommonSlice = {
  toggle: {
    aria: '语言',
    en: 'EN',
    zh: '中文',
  },
  auth: {
    signIn: '使用 GitHub 登录',
    signOut: '退出登录',
  },
  footer: {
    tagline: '· ChronoAI 托管云',
    github: 'GitHub',
    manual: '操作手册',
  },
  shell: {
    errorTitle: '出错了',
    errorBody:
      '发生了意外错误，页面被中断。刷新通常即可恢复；若仍然反复出现，下方的详细信息有助于定位问题。',
    errorReload: '刷新页面',
    errorDetailsSummary: '错误详情',
    notFoundEyebrow: '错误 404',
    notFoundTitle: '此页面不存在',
    notFoundBody: '`{path}` 没有对应的路由。它可能已被移动，或链接输入有误。',
    notFoundHome: '返回首页 →',
    notFoundMetaTitle: 'FKST — 页面未找到',
    toastDismiss: '关闭',
  },
};
