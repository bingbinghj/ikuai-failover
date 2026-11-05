# 通知

- 新配置的通知渠道默认为空。在可视化配置中点击添加渠道，从下拉列表选择需要的类型，再填写接收目标和实际凭据；只展示已添加的渠道，已有配置过的渠道保留。
- 可同时使用多个渠道，标识自动生成并用于投递追踪。普通配置直接填写 Token、Webhook 或密钥，默认隐藏输入；事件通过勾选筛选，消息格式等可选参数使用下拉列表。
- 配置文件中的 `notifications` 保存通知设置，实际凭据保存在 `notifications.secrets`。同名容器环境变量仍可提供凭据，非空环境变量优先；可视化页面逐个字段提供密码显示按钮。
- 配置页面只使用可视化表单。实际配置文件仅保存在部署目录，不提交到公共仓库。
- notifications.enabled 默认 false，channels 中的 enabled 分别控制各渠道。--check 完全离线校验配置；--dry-run 同时关闭真实通知发送。

| 公共字段 | 默认值 | 含义 |
| --- | --- | --- |
| enabled | false | 通知总开关 |
| timeout_ms | 10000 | 单渠道请求总超时 |
| events | 空列表 | 只投递指定事件；空列表发送网络、策略和配置事件，服务生命周期仅记录 |
| cooldown_ms | 60000 | 相同事件类型、标题和正文的冷却时间 |
| queue_capacity | 128 | 最多保留的待投递事件数 |
| channels | 空列表 | 按需添加的渠道；每项有内部标识、启用状态和类型 |
| secrets | 空对象 | 渠道凭据名称与值；未设置环境变量时使用 |
| templates | 内置默认模板 | 默认及各事件的 JSON + Jinja2 模板 |

| 渠道（配置类型） | 普通配置字段 | 凭据引用字段（文本配置） |
| --- | --- | --- |
| Telegram（telegram） | api_url、chat_id、可选 message_thread_id | bot_token_env |
| Slack（slack） | api_url、channel | bot_token_env |
| Discord（discord） | api_url、channel_id | bot_token_env |
| 钉钉（ding_talk） | 无 | webhook_url_env、可选 secret_env |
| Synology Chat（synology_chat） | 可选 user_ids | webhook_url_env |
| VoceChat（voce_chat） | api_url、target | api_key_env |
| 企业微信应用（wechat） | api_url、corp_id、agent_id、touser | corp_secret_env |
| 企业微信机器人（qy_wechat_bot） | 可选 message_type（text 或 markdown，默认 text） | webhook_url_env |
| 飞书（feishu） | api_url、app_id、receive_id、receive_id_type | app_secret_env |
| 微信 ClawBot（wechat_claw_bot） | api_url、account_id、recipient | bot_token_env、可选 context_token_env |
| QQ（qq） | api_url、token_url、app_id、recipient、recipient_type | app_secret_env |
| 浏览器推送（web_push） | subject、ttl_seconds | subscriptions_env、private_key_env |
| AppPush（app_push） | api_url、owner、route | relay_key_env |
| Web（web） | 无 | receive_url_env、可选 token_env |
| WebAgent（web_agent） | 无 | receive_url_env、可选 token_env |

- Telegram、Slack、Discord、企业微信、飞书、微信 ClawBot、QQ 的 api_url 可沿用各渠道默认地址。VoceChat 与 AppPush 需要填写自身服务地址。VoceChat target 使用 UID# 或 GID# 前缀选择用户或群组；QQ recipient_type 为 user 或 group。
- WebPush 的 subscriptions_env 指向浏览器订阅列表 JSON，单项包含 endpoint 及 keys.p256dh、keys.auth；private_key_env 为 base64url 编码的 VAPID 私钥，subject 为 mailto 或 HTTPS 联系地址。
- AppPush 使用 MoviePilotServer 的 push/messages 接收推送，relay_key_env 为至少 32 字符的中继密钥，owner 为接收者，route 为通知打开路径。
- Web 与 WebAgent 使用接收地址桥接 MoviePilot 内置渠道。服务向该地址 POST channel 与 event，event 包含 id、kind、title、text、timestamp_ms；可选 token_env 使用 Bearer 认证。接收端完成处理后需返回 success 或 ok 为 true 的 JSON。
- `.env.example` 列出可用的通知环境变量名称，供选择环境变量方式时使用；不需要为未添加的渠道填写凭据。
- 单渠道缺少凭据或投递失败只记录该渠道的错误代码，其他渠道继续投递，WAN 检查与故障转移继续运行。

企业微信机器人使用群聊机器人的 Webhook，与企业微信应用分别配置。在页面添加“企业微信机器人”，填入完整 Webhook 并选择文本或 Markdown 即可。需要文本配置时可参考：

```yaml
notifications:
  enabled: true
  channels:
  - tag: qy_wechat_bot
    enabled: true
    provider: qy_wechat_bot
    webhook_url_env: NOTIFY_QYWECHAT_WEBHOOK
    message_type: text
```

- 可以直接在页面填写完整 Webhook，或填写容器环境变量 `NOTIFY_QYWECHAT_WEBHOOK`。地址中的 key 属于凭据，只保存在本地部署配置或环境变量中。
- 发送内容包含事件标题与正文。text 最多 2048 个 UTF-8 字节，markdown 最多 4096 个 UTF-8 字节；超过限制会按字符边界截断并附提示，完整事件仍保留在 /notifications。
- 只有 HTTP 请求成功且响应 errcode 为数值 0 才记录成功；缺少返回码、非零返回码或通信失败均记录失败，并沿用现有渠道重试。错误记录不保存 Webhook 地址或原始响应。
- 此渠道覆盖同样的 IP、线路、PD、策略和配置事件，可与 Telegram、企业微信应用及其他通知渠道同时启用。

| 事件 | 触发条件 |
| --- | --- |
| service_started、service_restarting、service_stopped、service_failed | 服务生命周期保留在日志与历史中，默认不推送；旧配置显式指定 events 时仍兼容 |
| wan_health_changed | 爱快 WAN 原生线路检测状态变化 |
| wan_ipv4_changed | WAN IPv4 地址获得、丢失或变化，包含运营商 CGNAT 地址 |
| wan_ipv6_changed、ipv6_pd_changed | WAN IPv6 地址或 DHCPv6 PD 前缀变化 |
| egress_switched、egress_recovered | 物理出口自动切换或恢复回切 |
| failover_pending | 等待故障阈值或策略应用 |
| policy_synchronized、policy_conflict | 策略变化应用完成或规则身份、快照冲突 |
| cycle_failed、cycle_recovered | 检查或应用失败及恢复 |
| ipv6_degraded、ipv6_recovered | IPv6 地址、PD 或 LAN 前缀异常及恢复 |

- 正常首次观察只建立地址与线路基线；首次检查中发生的物理出口切换仍通知。基线持久化后，服务重启不会重复报告未变化的地址。
- 通知在独立后台发送，WAN 检查不等待外部通知服务。成功渠道立即从待投递目标中移除，只对失败渠道重试；总计最多五次尝试，失败后的等待时间为 1、2、4、8 秒。
- 队列达到上限时丢弃最旧的待投递事件并计数，/status 的 notifications 展示队列、失败、重试及丢弃统计。/notifications 展示最近 200 条事件、待投递渠道与投递结果。
- 服务退出最多等待 15 秒处理当前可投递的通知，未完成队列保留到下次启动。状态文件保存事件正文、渠道 tag 和错误代码，不保存凭据、渠道地址或原始服务响应。
- 损坏的通知状态保留为同目录中的 notifications.corrupt-时间戳.json，通知基线和队列重新初始化。通知后台出现存储或发送异常时单独记录并重试，线路检查与网络故障转移继续运行。

## 通知模板和测试发送

每个渠道卡片提供“测试”按钮，使用当前表单内容向该渠道发送一条测试消息，无需先保存或重启。测试不写入配置，也不会修改爱快规则。

通知模板在可视化设置中以卡片展示，点击可编辑、预览或恢复默认。`notifications.templates.default` 为默认模板；三类分类模板优先于旧逐事件模板和默认模板。模板为空对象 `{}` 时保持原消息并补齐已知网络信息，单个模板最多 16 KiB。

```yaml
notifications:
  templates:
    default: |
      {"title": "{{ title }}", "text": "{{ text }}"}
    egress_switched: |
      {"title": "出口已切换", "text": "{{ text }}"}
```

JSON 支持 `title`、`text` 字符串字段，字段内容支持 Jinja2 语法。可用变量为 `title`、`text`、`message`、`kind`、`event_type`、`timestamp_ms` 和 `event`。模板渲染不会改变事件的内部标识和去重依据。

## 实时和历史日志

右上角通知中心打开日志与通知弹窗，默认实时日志，通知记录保留在单独标签中；总览的日志快捷操作也可打开此弹窗。实时视图支持级别选择、关键字搜索、暂停及查看最新；向上阅读时停止自动滚动。历史视图默认当前文件 `service.log`（兼容 `server.log`），下拉选项直接显示文件名，旧轮转文件单独列出，可选择单个轮转文件或全部轮转文件，筛选在文件完整内容中进行，返回最近最多 1000 条匹配记录。

服务重启后先等待首次有效采样建立状态基线，再报告真实线路变化。首次检查失败时仍记录具体错误，不把尚未完成检测误报为线路离线。

常规轮询不会重复记录“已取得主备线路状态”或等待首次采样的进度日志；状态变化、规则调整、失败与恢复仍正常记录。

## MoviePilot 配置字段

通知卡片、配置弹窗及字段名称来自固定版本 MoviePilot-Frontend v3。企业微信保留应用和智能机器人模式；企业微信群 Webhook 机器人作为独立渠道保留原配置。完整 MP 扩展字段保存在 `channels[].mp_config`，名称保存在 `name`，消息类型选项保存在 `switchs`，发送参数与原有配置字段保持兼容。

通知类型使用本项目的线路、地址、PD、出口切换、策略与配置事件；类型为空时发送全部允许的网络事件。服务启动、重启、停止、异常默认只记录。旧版 MP 的媒体通知类型在加载时移除，升级后恢复按本项目事件选择。机器人、应用凭据和目标字段驱动实际通知发送；MP 的用户与管理员名单、回调验证 Token 和 AESKey 等字段保留原名称及配置内容，本服务不提供 MoviePilot 的媒体管理聊天命令。

飞书与 QQ 同时填写用户目标和群组目标时，会向两类目标发送。企业微信智能机器人使用 MP 相同的 WebSocket 订阅与消息确认协议。浏览器推送使用服务端生成的 VAPID 凭据与浏览器订阅，凭据不需要在 MP 表单中手填。

## 默认模板

内置模板参考用户提供的 `notification.txt`：使用 emoji 标题、Jinja2 条件内容、事件名称和时间。模板中保留真实网络消息，不使用 MP 的媒体库变量。模板合并为线路与地址（network_change）、切换与恢复（egress_change）、策略与配置（policy_change）三类；默认模板保留在小入口。分类覆盖优先于旧的逐事件模板；旧 service_change 自定义仍兼容检查与配置事件。

模板预览中的“发送测试”使用当前编辑的模板、标题、正文和所选渠道的草稿配置，无需先保存；发送时使用服务实际采集的网络信息。测试不会更新配置或持久化未保存的模板。

附加变量包括 `event_icon`、`event_label`、`timestamp`、`time` 和 `service`。`timestamp` 与 `time` 采用北京时间（UTC+8），消息只显示日期和时间。模板配置为空对象 `{}` 时保留原始标题和正文，同时仍附上必需的网络信息与时间。

## 消息格式与网络信息

配置页和模板测试只列出当前渠道支持的格式；新建渠道与未指定格式的配置采用 MP 的实际默认发送方式。已有明确指定的 Text/Markdown 配置仍可读取。

| 渠道 | 默认格式 | 可选格式 |
|---|---|---|
| Telegram | MarkdownV2 | MarkdownV2、HTML、Text |
| Slack | mrkdwn | Markdown、Text |
| Discord | Embed | Embed、Markdown、Text |
| 钉钉、SynologyChat、VoceChat | Markdown | Markdown、Text |
| 企业微信应用 | Text | Text、News、Markdown（接收端兼容处理） |
| 企业微信智能机器人 | Markdown | Markdown、兼容 Text |
| 企业微信群 Webhook 机器人 | Text | Text、Markdown |
| 飞书 | Markdown 交互卡片 | Markdown、Text |
| QQ | Markdown | Markdown、Text |
| 微信 ClawBot、WebPush、AppPush、Web | Text | Text |
| WebAgent | Markdown | Markdown、Text |

Telegram 按 MarkdownV2 或 HTML 子集转换，处理转义、标签闭合与最终长度。Slack 使用 mrkdwn，短消息使用 Block Kit，长正文使用文本载荷；Discord 默认通过 Embed 的标题和描述发送；飞书把正文放在 interactive 卡片的 markdown 元素中。QQ 默认尝试原生 Markdown，明确返回 11244 格式权限拒绝时回退文本；旧 `mp_config.QQ_MARKDOWN: false` 仍按兼容文本发送。各平台仅支持自己的语法子集，表格不代表完整 CommonMark 或相同视觉布局。

企业微信应用的原生 Markdown 最大 2048 UTF-8 字节，微信微工作台无法展示；默认转换为兼容文本，`mp_config.WECHAT_APP_MARKDOWN: true` 才发送原生 Markdown。MP 应用通知常规使用 Text，有封面时使用 News，这与 Markdown 是不同类型。

按用户要求，News 不要求封面或跳转配置，单篇标题最多 128 字节、摘要最多 512 字节，优先保留事件前后线路、运营商、IP 和时间。无链接载荷采用 MP 的 `url: null` 方式；官方文档仍要求链接或小程序，因此不能保证真实接口接受。明确的图文格式/内容拒绝会记录兼容回退日志并再发送 Text；网络超时、身份或目标拒绝不重试图文转换，避免重复发送。摘要无法完整容纳必需信息时也改发 Text。

纯文本渠道将 Markdown 模板转为可读文本，保留链接、列表和换行。Web/WebAgent 本项目使用外部桥接，接收端决定最终展示；MP 普通 Web 通知自身为文本，WebAgent 助手消息才使用 Markdown 渲染。完整上游对照见 [MP通知格式对照](MP通知格式对照.md)。

模板预览中的测试发送可选择“按渠道配置”或临时格式；选项随测试渠道变化，News 仅企业微信应用、HTML 仅 Telegram、Embed 仅 Discord。临时覆盖在测试请求中传递，不保存格式、模板或凭据。预览卡片展示模板生成的文字，真实客户端样式以实际发送结果为准。

无论是否自定义模板，每条消息都会带上已获取的接口、运营商、IPv4、IPv6、PD 与主备线路信息，并使用北京时间。未取得数据的字段直接省略，不输出“尚未获取”、unknown 或 unavailable 占位。暂未完成新采样但保留旧数据时标记为上次记录。格式转换或平台长度限制下，优先保留前后线路、网络信息及时间，再缩短正文。

切换与恢复通知保存事件发生之前和之后的实际网络快照，显示前后接口、运营商及 IP。同一接口地址变化使用该接口的旧值与新值；不通过主备角色猜测旧出口。变量包括 `transition_summary`、`from`、`to`，以及 `from_interface`/`to_interface`、`from_isp`/`to_isp`、`from_ipv4`/`to_ipv4`、`from_ipv6`/`to_ipv6`、`from_prefix`/`to_prefix`。旧历史无前态时只展示已知方向。

运行日志页面已移除下载入口；历史列表只显示实际文件名。日志正文是唯一滚动区域，统计位于工具栏。
