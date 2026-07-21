import type { TourSlice } from '../slices';

// 引导式产品导览文案：`?` 启动按钮标签、教练标记控件、以及每一步的卡片。作为独立的
// 顶层 `tour` 领域，由 `zh.ts` 组合进目录。键与 `components/tour/tour-steps.ts`
// 中的步骤 id 一一对应。`{n}`/`{m}` 占位符、`?`、`Esc` 与 GitHub 术语（issue、PR、
// package、label）在最自然处保留，仅翻译散文。
export const tour: TourSlice = {
  helpAria: '开始产品导览',
  closeAria: '结束导览',
  progress: '{n} / {m}',
  skip: '跳过',
  back: '上一步',
  next: '下一步',
  done: '完成',
  getStarted: '打开「快速上手」',
  steps: {
    welcome: {
      title: '欢迎使用 fkst',
      body: '每个会话都作为由 GitHub 驱动的 substrate 智能体运行，并为你开启 pull request。这个仪表盘就是你观察与控制它们的地方 —— 花 60 秒了解它能做什么。',
    },
    canvas: {
      title: '画布',
      body: '你工作的可缩放图谱：先点账户，再点仓库，再点会话逐层深入。滚轮滚动页面，拖拽平移，Controls 缩放图谱。',
    },
    breadcrumb: {
      title: '当前位置',
      body: '面包屑显示你所在的层级。点击任一层级 —— 或按 Esc —— 即可返回上层：仓库、账户或根视图。',
    },
    sidebar: {
      title: '详情面板',
      body: '这个随层级变化的面板列出你的账户、仓库或会话，附带活动图表，以及解读每个徽标含义的状态图例。',
    },
    newSession: {
      title: '启动会话',
      body: '通过创建 trigger issue 启动会话：填写名称、要加载的 package、work label、可选的环境，以及是否自动合并其 pull request。',
    },
    sessionDetail: {
      title: '会话详情',
      body: '打开任一会话即可看到四个分页 —— 状态（生命周期 + 实时引擎）、包（配置 + 队列）、日志（带搜索的内置查看器）、成果（pull request 与文件预览）。',
    },
    workItem: {
      title: '追加任务',
      body: '无需前往 GitHub，就能给运行中的会话再派一个任务 —— 它会被加入该会话的工作队列，并在下一轮扫描时被取用。',
    },
    environments: {
      title: '环境',
      body: '在这里构建可复用的安装命令、变量与 secret 配置档，然后在启动会话时按名称引用其中之一。',
    },
    newRepo: {
      title: '创建仓库',
      body: '新建一个用于运行会话的仓库 —— 随后在其上安装 App 并开启 trigger issue。',
    },
    refresh: {
      title: '保持最新',
      body: '你查看时数据会自动刷新。Refresh 会立即强制更新，面板也会显示当前视图的新鲜程度。',
    },
    help: {
      title: '重新打开导览',
      body: '需要再看一遍？随时通过 ? 按钮重新启动本导览。顶栏还提供 GitHub 链接与语言切换；「快速上手」可从首页进入。',
    },
    finish: {
      title: '一切就绪',
      body: '这就是整个仪表盘。前往「快速上手」查看完整讲解，或直接开始使用。',
    },
  },
};
