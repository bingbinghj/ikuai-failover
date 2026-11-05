通知协议字段参考 MoviePilot v3 官方源码，Rust 适配器为独立实现，随本仓库以 AGPL-3.0 发布。

- 来源仓库：jxxghp/MoviePilot
- 核验提交：e99f6bb5722905014c81e9bbc966493a0223cdd4
- 渠道枚举：app/schemas/types.py 的 NotificationChannel
- 外部通知协议：app/modules/{telegram,slack,discord,dingtalk,synologychat,vocechat,wechat,feishu,wechatclawbot,qqbot,webpush,apppush}
- AppPush 中转协议：app/adapters/external/server.py 的 relay_push
- 原项目许可证：AGPL-3.0，https://github.com/jxxghp/MoviePilot/blob/e99f6bb5722905014c81e9bbc966493a0223cdd4/LICENSE
- WebPush 加密：web-push-native 0.5，MIT OR Apache-2.0，https://github.com/leotaku/web-push-native
- Web 与 WebAgent 属于 MoviePilot 内部通知中心和助手会话。本服务需要实际部署的通知接收桥接端点，不将用户输入接口当作通知发送接口。

企业微信机器人为独立的 qy_wechat_bot 渠道，按 Webhook 协议以 Rust 实现，没有复制插件代码。

- 用户指定参考：[bfjy2024/MoviePilot-Plugins 的 qywechatmsg](https://github.com/bfjy2024/MoviePilot-Plugins/blob/1d5a6912ba80267da402a3909719a93660f0d991/plugins/qywechatmsg/__init__.py)，核验提交 1d5a6912ba80267da402a3909719a93660f0d991；原仓库 GPL-3.0。
- [企业微信官方消息推送配置说明](https://developer.work.weixin.qq.com/document/path/91770)：群机器人 Webhook 使用 HTTP POST；text 内容限制为 2048 个 UTF-8 字节，markdown 为 4096 个 UTF-8 字节，errcode 数值为 0 表示成功。
- 核对日期：2026-11-05。正文按 UTF-8 字符边界限制长度并附截断提示；完整事件仍保存在本服务事件历史中。

2026-11-05 消息格式核验：MP Telegram 默认 MarkdownV2，也支持 HTML/纯文本；Slack 使用 mrkdwn（条件构建 Block Kit），Discord 默认 Embed，钉钉与 VoceChat 使用 Markdown，SynologyChat 使用其消息标记，飞书使用含 Markdown 元素的 interactive 卡片，QQ 优先原生 Markdown 并在格式权限失败时回退文本。企业微信应用默认 Text、有图片走 News；智能机器人使用 WebSocket Markdown。ClawBot、WebPush、AppPush 为文本。MP 普通 Web 通知正文直接插值，WebAgent 助手使用 Markdown 渲染；本服务对应 Web/WebAgent 的外部桥接由接收端完成展示。

本服务允许各平台支持的兼容文本选项；Telegram 增加对应 HTML/MarkdownV2 转换，Discord 默认 Embed。已明确配置的旧格式仍可读取。系统推送将模板转换为可读文本。网络上下文在事件入队时固定，格式和长度处理优先保留接口、运营商、IP 和北京时间。

- [企业微信应用消息](https://developer.work.weixin.qq.com/document/path/90236)：原生应用 Markdown 2048 UTF-8 字节，微工作台不支持展示；News 标题 128 字节、摘要 512 字节，封面可选，URL 或小程序必填。
- 本项目按用户要求提供无封面、无跳转 News，沿用 MP `url: null` 载荷。MP 的同类测试模拟返回成功，不能证明真实平台接受；本项目仅对明确格式/内容拒绝回退 Text，不虚构 URL。
- [Synology 官方集成文档](https://kb.synology.com/en-us/DSM/help/Chat/chat_integration?version=7)：链接使用 `<url|label>`。
- [Telegram Bot API](https://core.telegram.org/bots/api#formatting-options)：MarkdownV2 与 HTML 支持子集及实体转义。

完整逐渠道源码依据与本项目对应格式记录在 [MP通知格式对照](../../../../docs/MP通知格式对照.md)。
