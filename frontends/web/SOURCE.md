# 页面主题与交互参考

本前端使用 Vue 3 与 Vuetify 3.7.3 实现故障转移控制台。页面布局、设置表单、通知配置、日志与主题采用 [MoviePilot-Frontend v3](https://github.com/jxxghp/MoviePilot-Frontend/tree/v3) 的组件和交互方式，核对源码提交为 `c6fc6562fa3ebbe6d3790a8f58ed68879347a49c`。Vuetify 版本与该提交一致，MDI 图标通过本地字体资源提供。

参考文件包括 `src/utils/themePalette.ts`、`src/plugins/vuetify/theme.ts`、`defaults.ts`、`src/layouts/default/components/DefaultLayout.vue`、`src/@layouts/styles/_variables.scss`、`src/styles/themes/glass.scss`、`UserProfile.vue`、`UserNotification.vue`、`LoggingView.vue`、`AccountSettingNotification.vue`、`NotificationChannelInfoDialog.vue` 和 `NotificationTemplateEditorDialog.vue`。应用使用 VApp、VNavigationDrawer、VAppBar、VMain 布局，导航宽度为 240 像素、收起宽度为 80 像素、顶栏高度为 64 像素，页面内容填满导航右侧。

主题保留 MP 的浅色和深色语义调色，组件默认值沿用其 outlined 表单、primary 操作色、无额外 elevation 卡片和菜单设定。主题偏好支持浅色、深色和跟随系统；Vuetify、浏览器控件和主题色同步。通知表单仅配置本服务实际发送所需参数，双向聊天命令和媒体管理功能不引入。

MoviePilot-Frontend 使用 MIT 许可证，副本保存在 `third-party/MoviePilot-LICENSE`；本项目沿用 AGPL-3.0 许可证。localStorage 保存登录账号、记住选择、主题偏好和通知已读 ID；记住密码使用 IndexedDB 加密保存，并兼容浏览器密码管理器。通知正文不持久保存到浏览器存储。

2026-11-05 界面修正：移除旧表单标签及选择框样式覆盖；设置按 MP `src/pages/setting.vue` 使用 VTabs/VWindow/VRow/VCol。通知配置逐项使用固定源码的大小写字段、中文名称、提示与条件显示，渠道图标由同一提交的 `src/assets` 提取，随原 MIT 许可提供。卡片弹窗使用独立 CardSettingsPanel，不嵌入完整配置页面。

本轮流程优化：页面与标签使用 hash 路由保持刷新位置；配置页仅保留可视化分类和保存操作。账号菜单、日志及通知沿用 MP 组件密度；通知类型改为项目事件，技术字段保留 MP 英文命名。记住密码在当前浏览器的 IndexedDB 中保存密文，HTTPS 使用 Web Crypto，HTTP 局域网使用相同 AES-GCM 算法的 noble-ciphers 兼容实现；本地保存不改变服务器会话认证逻辑。

本轮按用户提供的 MP 页面截图重构：主容器去掉最大宽度限制、头像两行菜单、Bing 每日壁纸及透明登录卡、全套 MDI 图标、卡片左下独立保存。日志仅正文滚动，文件下拉保留原名且删除下载操作；通知合并三类模板，删除服务模板入口。规则类型与主备选择框间距经过桌面、平板及手机实际渲染验证。新增 Bing 接口只代理每日图片，不缓存应用页面。

总览采用 MP 仪表盘的 4/8 分栏：左侧当前出口和线路状态，右侧网络统计、快捷操作和爱快信息；系统名称、版本、运行时间来自爱快，CPU/内存仅在爱快提供时显示，历史采样标记“采样”。头像菜单按图标和文字自然宽度显示。侧栏仅保留总览和配置，日志与通知由右上角通知中心进入。通知发送卡片与通知设置共享渠道图标。通知模板预览可选择启用的渠道，直接发送当前未保存的草稿；测试不保存配置，发送的网络信息来自运行时状态。

浏览器标签页使用爱快官网首页声明的原始 favicon（`/templates/ikuaitemplate/images/favicon.ico`），原样保存为静态图标，未重绘。
