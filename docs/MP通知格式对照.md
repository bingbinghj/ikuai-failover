# MoviePilot 通知格式对照

核验日期：2026-11-05。以下区分 **MP 实际发送方式**、**平台消息类型**和**本项目提供的格式选项**。消息带图片或采用卡片布局，并不等于发送了 Markdown。

- MP 后端固定提交：[`e99f6bb5722905014c81e9bbc966493a0223cdd4`](https://github.com/jxxghp/MoviePilot/tree/e99f6bb5722905014c81e9bbc966493a0223cdd4)。
- MP 前端固定提交：[`c6fc6562fa3ebbe6d3790a8f58ed68879347a49c`](https://github.com/jxxghp/MoviePilot-Frontend/tree/c6fc6562fa3ebbe6d3790a8f58ed68879347a49c)。
- MP 内置渠道以 [`NotificationChannel`](https://github.com/jxxghp/MoviePilot/blob/e99f6bb5722905014c81e9bbc966493a0223cdd4/app/schemas/types.py#L584) 的 14 项为准；企业微信群 Webhook 机器人是本项目的独立扩展。

## 逐渠道核验

表中的默认值适用于新建渠道或没有显式保存格式的配置。已有格式保持兼容；本项目的 `markdown` 配置值会按目标平台转换，界面显示平台对应名称。

| 渠道 | MP 实际默认与发送分支 | 本项目默认 | 本项目可选格式 | 源码证据 |
|---|---|---|---|---|
| Telegram | 普通 `send_msg` 默认 `MarkdownV2`；支持显式 `HTML`、`plain`。无图使用 `sendMessage`，有图使用 `sendPhoto` 和 caption。另有独立 `rich_message` 分支。 | MarkdownV2 | MarkdownV2、HTML、Text | [telegram.py：1000](https://github.com/jxxghp/MoviePilot/blob/e99f6bb5722905014c81e9bbc966493a0223cdd4/app/modules/telegram/telegram.py#L1000) |
| Slack | `chat_postMessage` 使用 `mrkdwn=True`。普通消息可只带顶层 `text`；有图片、按钮、链接或渠道富文本时构建 Block Kit，正文区块使用 `mrkdwn`，标题区块可为 `plain_text`。 | Markdown（mrkdwn） | Markdown（mrkdwn）、Text | [slack.py：328](https://github.com/jxxghp/MoviePilot/blob/e99f6bb5722905014c81e9bbc966493a0223cdd4/app/modules/slack/slack.py#L328) |
| Discord | 常规通知发送 Embed，正文放 `description` 或字段，使用 Discord 的 Markdown 展示。图片放 Embed 图片字段，链接放 Embed URL 或按钮；部分媒体事件把“字段：值”拆为卡片字段。 | Embed | Embed、Markdown、Text | [discord.py：982](https://github.com/jxxghp/MoviePilot/blob/e99f6bb5722905014c81e9bbc966493a0223cdd4/app/modules/discord/discord.py#L982) |
| 钉钉 | 自定义机器人固定发送 `msgtype=markdown`，标题、正文、图片语法及详情链接组合成 Markdown。 | Markdown | Markdown、Text | [dingtalk.py：103](https://github.com/jxxghp/MoviePilot/blob/e99f6bb5722905014c81e9bbc966493a0223cdd4/app/modules/dingtalk/dingtalk.py#L103) |
| SynologyChat | 请求中的字段叫 `text`，内容采用聊天格式：`*标题*`、多行正文及链接；有图附 `file_url`。MP 渠道专用正文声明为 `mrkdwn`，不是 HTML 消息。 | Markdown | Markdown、Text | [synologychat.py：67](https://github.com/jxxghp/MoviePilot/blob/e99f6bb5722905014c81e9bbc966493a0223cdd4/app/modules/synologychat/synologychat.py#L67) |
| VoceChat | HTTP 正文以 `Content-Type: text/markdown` 发送；标题加粗，图片和链接使用 Markdown 语法。 | Markdown | Markdown、Text | [vocechat.py：89](https://github.com/jxxghp/MoviePilot/blob/e99f6bb5722905014c81e9bbc966493a0223cdd4/app/modules/vocechat/vocechat.py#L89) |
| 企业微信应用 | 无图片默认 `msgtype=text`；有图片发送 `msgtype=news`，标题、摘要、封面和链接放在 `articles`。MP 应用通知实现没有原生 Markdown 分支。 | Text | Text、News、Markdown | [wechat.py：249](https://github.com/jxxghp/MoviePilot/blob/e99f6bb5722905014c81e9bbc966493a0223cdd4/app/modules/wechat/wechat.py#L249) |
| 企业微信智能机器人 | 与应用模式属于同一 MP 渠道，但使用 WebSocket `aibot_send_msg`，实际发送 `msgtype=markdown`。标题、图片和链接合并进 Markdown 正文。 | Markdown | Markdown、Text | [wechatbot.py：754](https://github.com/jxxghp/MoviePilot/blob/e99f6bb5722905014c81e9bbc966493a0223cdd4/app/modules/wechat/wechatbot.py#L754) |
| 飞书 | 普通通知发送 `msg_type=interactive` 交互卡片，卡片内部使用 `tag=markdown`。图片上传后使用 `img_key`；按钮放卡片操作区域。受保护结果另有 `plain` → `text` 分支。 | Markdown（交互卡片） | Markdown（交互卡片）、Text | [feishu.py：1785](https://github.com/jxxghp/MoviePilot/blob/e99f6bb5722905014c81e9bbc966493a0223cdd4/app/modules/feishu/feishu.py#L1785) |
| 微信 ClawBot | 常规通知优先发送纯文本。只在没有标题、正文和链接形成的文字时发送纯图片；多行文本和链接作为文字组合。能力声明中的 Markdown 不代表此通知适配器存在原生 Markdown 消息类型。 | Text | Text | [wechatclawbot.py：2454](https://github.com/jxxghp/MoviePilot/blob/e99f6bb5722905014c81e9bbc966493a0223cdd4/app/modules/wechatclawbot/wechatclawbot.py#L2454) |
| QQ | 非空通知默认尝试 `msg_type=2` 和 `markdown.content`，可附键盘；Markdown 权限相关错误回退为完整正文 `msg_type=0`。图片用 QQ Markdown 尺寸语法，链接放正文。 | Markdown | Markdown、Text | [qqbot.py：349](https://github.com/jxxghp/MoviePilot/blob/e99f6bb5722905014c81e9bbc966493a0223cdd4/app/modules/qqbot/qqbot.py#L349) |
| Web | 内部消息历史与通知事件，不是外部机器人协议。固定版本前端通知正文直接使用文本插值和 `white-space: pre-wrap`，没有调用 Markdown 渲染器。 | Text | Text | [UserNotification.vue：918](https://github.com/jxxghp/MoviePilot-Frontend/blob/c6fc6562fa3ebbe6d3790a8f58ed68879347a49c/src/layouts/default/components/UserNotification.vue#L918) |
| WebAgent | 内部助手会话与 SSE 消息更新；前端采用 MarkdownIt 渲染 Markdown，禁用原始 HTML。支持图片、附件和选项等会话结构。 | Markdown | Markdown、Text | [agentMarkdown.ts：4](https://github.com/jxxghp/MoviePilot-Frontend/blob/c6fc6562fa3ebbe6d3790a8f58ed68879347a49c/src/utils/agentMarkdown.ts#L4) |
| WebPush | 浏览器系统推送发送 JSON 的 `title`、`body`、`url`，正文没有 Markdown 渲染协议。 | Text | Text | [webpush/__init__.py：79](https://github.com/jxxghp/MoviePilot/blob/e99f6bb5722905014c81e9bbc966493a0223cdd4/app/modules/webpush/__init__.py#L79) |
| AppPush | 通过 MoviePilot-Server 中转 `title`、`body`、实例内 `route` 和可选图片，系统通知正文为文本。 | Text | Text | [apppush/__init__.py：248](https://github.com/jxxghp/MoviePilot/blob/e99f6bb5722905014c81e9bbc966493a0223cdd4/app/modules/apppush/__init__.py#L248) |
| 企业微信群机器人（本项目扩展） | 不属于上述 MP 内置枚举；本项目通过群 Webhook 发送。 | Text | Text、Markdown | [企业微信官方群机器人协议](https://developer.work.weixin.qq.com/document/path/91770) |

### 格式选项与消息结构

- Slack 的 Block Kit、飞书的交互卡片是消息结构；其中正文才使用平台富文本语法。本项目界面用 Markdown 选项驱动对应正文与卡片，不额外提供无法独立配置的 Block Kit 枚举。
- Discord 的 Embed 是独立消息结构，因此提供单独选项并作为默认；普通 Markdown 和 Text 消息是本项目增加的可用发送模式，不是 MP 常规通知的默认结构。
- MP 钉钉、VoceChat 等适配器默认发送 Markdown。本项目额外提供 Text：按平台文本协议发送，或在只有聊天 Markdown 结构的接口中转义格式字符。企业微信智能机器人的 Text 使用转义后的 Markdown 结构展示文字，不声明另一种未经实现的机器人消息类型。
- Web、微信 ClawBot 和系统推送仅在界面提供 Text；历史配置中的 Markdown 保持可读取，发送时转换成可读文字。
- MP 的 [ChannelCapabilityManager](https://github.com/jxxghp/MoviePilot/blob/e99f6bb5722905014c81e9bbc966493a0223cdd4/app/schemas/notification.py#L117) 描述渠道能力与专用正文格式；具体展示仍须核对发送适配器和接收前端。Web、微信 ClawBot 的声明与普通通知实际渲染就是两种需要区分的情况。

本项目表中选项描述线路、地址、切换、策略与配置等网络事件通知的正文及消息类型。上游图片、媒体列表、交互按钮、附件和流式助手分支是源码对照依据，不表示这些业务流程已随格式选项一并实现。Web/WebAgent 在本项目使用通知接收桥接，接收端负责展示和渲染。

## Telegram 的三种正文与富文本分支

普通 `send_msg` 的 `parse_mode=None` 默认为 `MarkdownV2`；显式 `HTML` 使用 Telegram HTML 子集；`plain` 或空字符串省略 API 的 `parse_mode`。因此本项目 MarkdownV2、HTML、Text 必须对应实际发送参数，不能只更改界面名称。

MP 的图片 caption 使用 1024 字符预算，普通文字使用 4096。超长 HTML/纯文本在 [`__send_long_plain_message`](https://github.com/jxxghp/MoviePilot/blob/e99f6bb5722905014c81e9bbc966493a0223cdd4/app/modules/telegram/telegram.py#L2198) 分块，HTML 保持实体及跨块标签完整；有图时第一块发送图片，其余发送文字，该实现仍沿用 1024 的分块预算。超长 Markdown 使用 `telegramify`，转换后可产生文字、图片或文件。海报下载或上传失败回到同一正文的文字消息；限流不立即追加发送。

另一个 [`rich_message` 分支](https://github.com/jxxghp/MoviePilot/blob/e99f6bb5722905014c81e9bbc966493a0223cdd4/app/modules/telegram/telegram.py#L1892) 接收 Markdown，使用 `richify(..., mode="html")` 和 `split_rich` 生成结构化 InputRichMessage，再调用 [`sendRichMessage`](https://github.com/jxxghp/MoviePilot/blob/e99f6bb5722905014c81e9bbc966493a0223cdd4/app/modules/telegram/botapi.py#L431)；失败可回退纯文本。它与 `sendMessage` 的 `parse_mode=HTML` 是两条不同路径。本项目当前三项正文格式使用 `sendMessage`，不声称实现了 MP 的全部结构化富文本路径。

## 企业微信应用的兼容性

[企业微信官方“发送应用消息”](https://developer.work.weixin.qq.com/document/path/90236)明确说明：**微工作台（原企业号）不支持展示 Markdown 消息**。在微信中打开企业微信应用会话，收到原生 `msgtype=markdown` 时，可能显示“暂不支持此消息类型，请在企业微信中查看”。接口接受发送与客户端能够展示是两个不同结果。

截图中 MP 的封面、标题和灰色多行摘要对应 `msgtype=news` 图文消息，不是 Markdown 渲染。MP 应用模式有图发送 News、无图发送 Text。原生应用 Markdown 是平台支持、本项目另行提供的模式，MP 此适配器没有使用它；本项目默认采用 Text，选择 Markdown 时默认转换为兼容文本，只有明确选择仅企业微信客户端接收才发送原生 Markdown。应用 Markdown 最长 **2048 UTF-8 字节**，群 Webhook Markdown 的 **4096 字节**不能套用到应用消息。

### News 无封面、无跳转的处理

官方 News 一个请求支持 1–8 篇：`title` 必填，最多 128 UTF-8 字节；`description` 可选，最多 512 字节；`picurl` 可选。标题和描述的单位是字节，不是中文字符数。News 摘要没有 Markdown/HTML 渲染协议。

应用 News 的 URL 字段虽然标记为可选，但同时要求“**小程序或者 url 必须填写一个**”；URL 最长 2048 字节并应带 HTTP/HTTPS 协议。因此无封面符合官方约束，无链接且无小程序不能据此宣称符合完整官方协议。Textcard 要求 URL，Mpnews 要求缩略图素材，不能作为同时省略链接和封面的等价替代。

MP 的 `__send_image_message` 直接写入 `"url": link`，未传入链接时生成 `null`。[上游图文载荷测试](https://github.com/jxxghp/MoviePilot/blob/e99f6bb5722905014c81e9bbc966493a0223cdd4/tests/test_wechat_notification_image.py)采用未传 `link` 的调用，但发送方法被 mock 成成功；它证明 MP 的载荷行为，不能证明企业微信真实接口接受空链接。

按本次用户要求，本项目 News 使用无封面、无跳转载荷，不自动生成地址。摘要优先保留线路、运营商、地址和时间；无法容纳必要信息时使用兼容 Text。接口返回已识别的图文内容或格式拒绝错误时，尝试兼容 Text；其他发送错误仍按实际错误处理。真实企业微信对无链接 News 的接受情况需以发送结果为准，本地载荷测试不替代实际客户端验证。

群 Webhook 也有官方 News 类型，但其 URL 明确必填；本项目的群机器人本次仍只提供 Text、Markdown。
