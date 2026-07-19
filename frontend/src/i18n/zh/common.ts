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
    getStarted: '快速开始',
    github: 'GitHub',
    manual: '操作手册',
  },
};
