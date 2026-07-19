import type { SiteContent } from './types';
import { nav } from './zh/nav';
import { common } from './zh/common';
import { dashboardScalars, canvasGraph, reposBase } from './zh/dashboard';
import { canvasSidebar } from './zh/sidebar';
import { canvasModals, reposModals } from './zh/modals';
import { canvasEnv, environmentScalar } from './zh/environments';
import { detail } from './zh/detail';
import { pages } from './zh/pages';
import { tour } from './zh/tour';

// 简体中文目录，由 `zh/` 下的各领域模块组合而成，与 `en.ts` 的组合方式逐一对应，
// 因此两种语言的键路径始终一致。带反引号的 token、`###` 标题、代码、命令、正则、
// 数字、单位和 emoji 与英文有意保持逐字节一致 —— 只翻译散文。GitHub 领域术语
// （issue、PR、package、Pod、label）在最自然处保留英文。`: SiteContent` 注解是
// 完整性的兜底 —— 每个键都必须恰好出现一次。
export const zh: SiteContent = {
  nav,
  ...common,
  dashboard: {
    ...dashboardScalars,
    ...environmentScalar,
    canvas: { ...canvasGraph, ...canvasSidebar, ...canvasModals, ...canvasEnv },
    repos: { ...reposBase, ...reposModals },
    detail,
  },
  ...pages,
  tour,
};
