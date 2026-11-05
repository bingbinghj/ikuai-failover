export type InputKind = 'text' | 'password' | 'number' | 'boolean' | 'select' | 'list' | 'json'
export interface Choice { value: string | number; label: string }
export interface Field {
  key: string; label: string; kind?: InputKind; hint?: string; min?: number; max?: number
  options?: string[]; choices?: Choice[]; optional?: boolean; default?: unknown; icon?: string
}

export const providerLabels: Record<string, string> = {
  telegram: 'Telegram', wechat: '企业微信', qy_wechat_bot: '企业微信群机器人', feishu: '飞书',
  wechat_claw_bot: '微信 ClawBot', slack: 'Slack', discord: 'Discord', ding_talk: '钉钉',
  synology_chat: 'Synology Chat', voce_chat: 'VoceChat', web: 'Web', web_agent: 'WebAgent',
  web_push: 'WebPush', app_push: 'App 私有推送', qq: 'QQ',
}
export const providerFields: Record<string, Field[]> = {
  telegram: [
    { key: 'bot_token_env', label: 'Bot Token', default: 'NOTIFY_TELEGRAM_BOT_TOKEN' },
    { key: 'chat_id', label: 'Chat ID', hint: '接收通知的用户、群组或频道 Chat ID。' },
    { key: 'message_thread_id', label: 'Topic ID', kind: 'number', optional: true, hint: '留空发送到默认话题。' },
    { key: 'api_url', label: '代理API地址', default: 'https://api.telegram.org', optional: true },
  ],
  wechat: [
    { key: 'corp_id', label: '企业ID', hint: '企业微信后台企业信息中的企业ID。' },
    { key: 'agent_id', label: '应用 AgentId', kind: 'number', min: 1, default: 0 },
    { key: 'corp_secret_env', label: '应用 Secret', default: 'NOTIFY_WECHAT_SECRET' },
    { key: 'api_url', label: '代理地址', default: 'https://qyapi.weixin.qq.com', optional: true, hint: '不使用代理时保留默认地址。' },
    { key: 'touser', label: '默认通知目标', default: '@all', optional: true, hint: '用户 userid，多个用 | 分隔；@all 发送给全部应用可见成员。' },
  ],
  qy_wechat_bot: [
    { key: 'webhook_url_env', label: '机器人 Webhook', default: 'NOTIFY_QYWECHAT_WEBHOOK', hint: '企业微信群机器人的完整 Webhook 地址。' },
    { key: 'message_type', label: '消息格式', kind: 'select', options: ['text', 'markdown'], default: 'text', optional: true },
  ],
  feishu: [
    { key: 'app_id', label: 'App ID' }, { key: 'app_secret_env', label: 'App Secret', default: 'NOTIFY_FEISHU_SECRET' },
    { key: 'receive_id_type', label: '默认通知目标', kind: 'select', choices: [{ value: 'open_id', label: '默认用户 Open ID' }, { value: 'chat_id', label: '默认群聊 Chat ID' }], default: 'chat_id' },
    { key: 'receive_id', label: '默认群聊 Chat ID', hint: '填写默认接收通知的目标 ID。' },
    { key: 'api_url', label: 'API 地址', default: 'https://open.feishu.cn', optional: true },
  ],
  wechat_claw_bot: [
    { key: 'bot_token_env', label: 'Bot Token', default: 'NOTIFY_CLAWBOT_TOKEN' },
    { key: 'account_id', label: '账号ID' }, { key: 'recipient', label: '默认通知目标', hint: '填写已与机器人互动过的用户 userid。' },
    { key: 'context_token_env', label: '上下文 Token', optional: true },
    { key: 'api_url', label: 'iLink 地址', default: 'https://ilinkai.weixin.qq.com', optional: true },
  ],
  slack: [
    { key: 'bot_token_env', label: 'Slack Bot User OAuth Token', default: 'NOTIFY_SLACK_TOKEN' },
    { key: 'channel', label: '频道名称', hint: '填写接收通知的频道 ID，例如 C0123456789。' },
    { key: 'api_url', label: 'API 地址', default: 'https://slack.com/api', optional: true },
  ],
  discord: [
    { key: 'bot_token_env', label: 'Bot Token', default: 'NOTIFY_DISCORD_TOKEN' }, { key: 'channel_id', label: '频道 ID' },
    { key: 'api_url', label: 'API 地址', default: 'https://discord.com/api/v10', optional: true },
  ],
  ding_talk: [
    { key: 'webhook_url_env', label: '自定义机器人 Webhook', default: 'NOTIFY_DINGTALK_WEBHOOK' },
    { key: 'secret_env', label: '加签密钥', default: 'NOTIFY_DINGTALK_SECRET', optional: true, hint: '启用加签时填写 SEC 开头的密钥，其他安全方式可留空。' },
  ],
  synology_chat: [
    { key: 'webhook_url_env', label: '机器人传入URL', default: 'NOTIFY_SYNOLOGY_WEBHOOK' },
    { key: 'user_ids', label: '默认通知目标', kind: 'json', default: [], optional: true, hint: '接收用户的数字 ID 数组，留空使用机器人默认目标。' },
  ],
  voce_chat: [
    { key: 'api_url', label: '地址', hint: 'VoceChat 服务端地址，格式：http(s)://ip:port。' },
    { key: 'api_key_env', label: '机器人密钥', default: 'NOTIFY_VOCECHAT_KEY' },
    { key: 'target', label: '频道ID', hint: '填写频道数字 ID，不包含 # 号。' },
  ],
  web: [{ key: 'receive_url_env', label: '接收地址', default: 'NOTIFY_WEB_RECEIVE_URL' }, { key: 'token_env', label: 'Token', default: 'NOTIFY_WEB_TOKEN', optional: true }],
  web_agent: [{ key: 'receive_url_env', label: '接收地址', default: 'NOTIFY_WEBAGENT_RECEIVE_URL' }, { key: 'token_env', label: 'Token', default: 'NOTIFY_WEBAGENT_TOKEN', optional: true }],
  web_push: [
    { key: 'subscriptions_env', label: '订阅 JSON', default: 'NOTIFY_WEBPUSH_SUBSCRIPTIONS' },
    { key: 'private_key_env', label: 'VAPID 私钥', default: 'NOTIFY_WEBPUSH_PRIVATE_KEY' },
    { key: 'subject', label: '联系地址', default: 'mailto:admin@example.com' },
    { key: 'ttl_seconds', label: '消息有效期（秒）', kind: 'number', min: 0, max: 86400, default: 86400, optional: true },
  ],
  app_push: [
    { key: 'api_url', label: '消息中心地址' }, { key: 'relay_key_env', label: '实例中转密钥', default: 'NOTIFY_APPPUSH_RELAY_KEY' },
    { key: 'owner', label: '接收者' }, { key: 'route', label: '消息路由', default: '/?shotcut=message', optional: true },
  ],
  qq: [
    { key: 'app_id', label: 'AppID' }, { key: 'app_secret_env', label: 'AppSecret', default: 'NOTIFY_QQ_SECRET' },
    { key: 'recipient_type', label: '默认通知目标', kind: 'select', choices: [{ value: 'user', label: '用户 OpenID' }, { value: 'group', label: '群组 OpenID' }], default: 'user' },
    { key: 'recipient', label: '用户 OpenID', hint: '用户 OpenID 和群组 OpenID 二选一。' },
    { key: 'api_url', label: 'API 地址', default: 'https://api.sgroup.qq.com', optional: true },
    { key: 'token_url', label: 'Token 地址', default: 'https://bots.qq.com/app/getAppAccessToken', optional: true },
  ],
}


export interface MoviePilotField extends Field { mode?: 'app' | 'bot' }
export const moviePilotProviderFields: Record<string, MoviePilotField[]> = {
  "wechat": [
    {
      "key": "WECHAT_MODE",
      "label": "使用智能机器人",
      "hint": "开启后使用企业微信智能机器人长连接发送通知。",
      "kind": "boolean",
      "default": "app"
    },
    {
      "key": "WECHAT_BOT_ID",
      "label": "机器人 BotID",
      "hint": "企业微信智能机器人的 BotID",
      "mode": "bot"
    },
    {
      "key": "WECHAT_BOT_SECRET",
      "label": "机器人 Secret",
      "hint": "企业微信智能机器人长连接专用 Secret",
      "mode": "bot"
    },
    {
      "key": "WECHAT_BOT_CHAT_ID",
      "label": "默认通知目标",
      "hint": "必填。填写用户 userid；主动发群消息时填写 group:群聊chatid。",
      "mode": "bot"
    },
    {
      "key": "WECHAT_BOT_WS_URL",
      "label": "长连接地址",
      "hint": "企业微信智能机器人 WebSocket 地址，通常使用默认值",
      "mode": "bot",
      "default": "wss://openws.work.weixin.qq.com"
    },
    {
      "key": "WECHAT_CORPID",
      "label": "企业ID",
      "hint": "企业微信后台企业信息中的企业ID",
      "mode": "app"
    },
    {
      "key": "WECHAT_APP_ID",
      "label": "应用 AgentId",
      "hint": "企业微信自建应用的AgentId",
      "mode": "app"
    },
    {
      "key": "WECHAT_APP_SECRET",
      "label": "应用 Secret",
      "hint": "企业微信自建应用的Secret",
      "mode": "app"
    },
    {
      "key": "WECHAT_PROXY",
      "label": "代理地址",
      "hint": "微信消息的转发代理地址，2022年6月20日后创建的自建应用才需要，不使用代理时需要保留默认值",
      "mode": "app"
    },
    {
      "key": "WECHAT_TOKEN",
      "label": "Token",
      "hint": "微信企业自建应用->API接收消息配置中的Token。可保存回调配置；本服务未开放该平台的消息回调接口。",
      "mode": "app"
    },
    {
      "key": "WECHAT_ENCODING_AESKEY",
      "label": "EncodingAESKey",
      "hint": "微信企业自建应用->API接收消息配置中的EncodingAESKey。可保存回调配置；本服务未开放该平台的消息回调接口。",
      "mode": "app"
    },
    {
      "key": "WECHAT_ADMINS",
      "label": "管理员白名单",
      "hint": "管理员用户 ID 列表，多个用逗号分隔。用于记录渠道权限；本服务只发送事件通知，不执行聊天命令。"
    }
  ],
  "wechat_claw_bot": [
    {
      "key": "WECHATCLAWBOT_BASE_URL",
      "label": "iLink 地址",
      "hint": "微信 ClawBot iLink 服务地址，通常使用默认值",
      "default": "https://ilinkai.weixin.qq.com"
    },
    {
      "key": "WECHATCLAWBOT_DEFAULT_TARGET",
      "label": "默认通知目标",
      "hint": "可填写用户 userid；不填则默认发给已互动用户"
    },
    {
      "key": "WECHATCLAWBOT_ADMINS",
      "label": "管理员白名单",
      "hint": "管理员用户 ID 列表，多个用逗号分隔。用于记录渠道权限；本服务只发送事件通知，不执行聊天命令。"
    },
    {
      "key": "WECHATCLAWBOT_POLL_TIMEOUT",
      "label": "轮询超时（秒）",
      "hint": "长轮询请求超时时间，建议 20-30 秒",
      "kind": "number",
      "default": 25
    }
  ],
  "feishu": [
    {
      "key": "FEISHU_APP_ID",
      "label": "App ID",
      "hint": "飞书开放平台应用的 App ID"
    },
    {
      "key": "FEISHU_APP_SECRET",
      "label": "App Secret",
      "hint": "飞书开放平台应用的 App Secret"
    },
    {
      "key": "FEISHU_OPEN_ID",
      "label": "默认用户 Open ID",
      "hint": "默认通知接收用户的 Open ID，留空则优先使用互动用户"
    },
    {
      "key": "FEISHU_CHAT_ID",
      "label": "默认群聊 Chat ID",
      "hint": "默认通知接收群聊的 Chat ID，和 Open ID 二选一即可"
    },
    {
      "key": "FEISHU_ADMINS",
      "label": "管理员白名单",
      "hint": "管理员用户 ID 列表，多个用逗号分隔。用于记录渠道权限；本服务只发送事件通知，不执行聊天命令。"
    },
    {
      "key": "FEISHU_VERIFICATION_TOKEN",
      "label": "Verification Token",
      "hint": "飞书事件订阅的 Verification Token，启用事件校验时填写。可保存回调配置；本服务未开放该平台的消息回调接口。"
    },
    {
      "key": "FEISHU_ENCRYPT_KEY",
      "label": "Encrypt Key",
      "hint": "飞书事件订阅的 Encrypt Key，启用消息加密时填写。可保存回调配置；本服务未开放该平台的消息回调接口。"
    }
  ],
  "telegram": [
    {
      "key": "TELEGRAM_TOKEN",
      "label": "Bot Token",
      "hint": "Telegram机器人token，格式：123456:ABC-DEF1234ghIkl-zyx57W2v1u123ew11"
    },
    {
      "key": "TELEGRAM_CHAT_ID",
      "label": "Chat ID",
      "hint": "接受消息通知的用户、群组或频道Chat ID"
    },
    {
      "key": "TELEGRAM_TOPIC_ID",
      "label": "Topic ID",
      "hint": "可选，向该群组的指定话题发送通知；留空发送到默认话题"
    },
    {
      "key": "TELEGRAM_USERS",
      "label": "用户白名单",
      "hint": "用户 ID 白名单，多个用逗号分隔。本服务向 Chat ID 指定的目标发送通知，不使用此名单筛选通知。"
    },
    {
      "key": "TELEGRAM_ADMINS",
      "label": "管理员白名单",
      "hint": "管理员用户 ID 列表，多个用逗号分隔。用于记录渠道权限；本服务只发送事件通知，不执行聊天命令。"
    },
    {
      "key": "API_URL",
      "label": "代理API地址",
      "hint": "自定义代理API地址，格式：https://api.telegram.org"
    }
  ],
  "slack": [
    {
      "key": "SLACK_OAUTH_TOKEN",
      "label": "Slack Bot User OAuth Token",
      "hint": "Slack应用`OAuth & Permissions`页面中的`Bot User OAuth Token`"
    },
    {
      "key": "SLACK_APP_TOKEN",
      "label": "Slack App-Level Token",
      "hint": "Slack App-Level Token，用于 Socket Mode；本服务的通知发送使用 Bot User OAuth Token。"
    },
    {
      "key": "SLACK_CHANNEL",
      "label": "频道名称",
      "hint": "消息发送频道，默认`全体`"
    },
    {
      "key": "SLACK_ADMINS",
      "label": "管理员白名单",
      "hint": "管理员用户 ID 列表，多个用逗号分隔。用于记录渠道权限；本服务只发送事件通知，不执行聊天命令。"
    }
  ],
  "discord": [
    {
      "key": "DISCORD_BOT_TOKEN",
      "label": "Bot Token",
      "hint": "Discord Bot Token（需在开发者后台开启 Message Content Intent）"
    },
    {
      "key": "DISCORD_GUILD_ID",
      "label": "服务器 ID",
      "hint": "服务器 ID，用于标记渠道所属服务器；本服务根据频道 ID 发送消息。"
    },
    {
      "key": "DISCORD_CHANNEL_ID",
      "label": "频道 ID",
      "hint": "可选，默认广播频道；为空则自动选择可发送消息的频道"
    },
    {
      "key": "DISCORD_ADMINS",
      "label": "管理员白名单",
      "hint": "管理员用户 ID 列表，多个用逗号分隔。用于记录渠道权限；本服务只发送事件通知，不执行聊天命令。"
    }
  ],
  "synology_chat": [
    {
      "key": "SYNOLOGYCHAT_WEBHOOK",
      "label": "机器人传入URL",
      "hint": "Synology Chat机器人传入URL"
    },
    {
      "key": "SYNOLOGYCHAT_TOKEN",
      "label": "令牌",
      "hint": "Synology Chat机器人令牌。可保存回调配置；本服务未开放该平台的消息回调接口。"
    },
    {
      "key": "SYNOLOGYCHAT_ADMINS",
      "label": "管理员白名单",
      "hint": "管理员用户 ID 列表，多个用逗号分隔。用于记录渠道权限；本服务只发送事件通知，不执行聊天命令。"
    }
  ],
  "ding_talk": [
    {
      "key": "DINGTALK_WEBHOOK",
      "label": "自定义机器人 Webhook",
      "hint": "钉钉群自定义机器人的完整 Webhook 地址"
    },
    {
      "key": "DINGTALK_SECRET",
      "label": "加签密钥",
      "hint": "机器人安全设置启用“加签”时填写，以 SEC 开头；使用关键词或 IP 白名单时可留空",
      "kind": "password"
    }
  ],
  "voce_chat": [
    {
      "key": "VOCECHAT_HOST",
      "label": "地址",
      "hint": "VoceChat服务端地址，格式：http(s)://ip:port"
    },
    {
      "key": "VOCECHAT_API_KEY",
      "label": "机器人密钥",
      "hint": "VoceChat机器人密钥"
    },
    {
      "key": "VOCECHAT_CHANNEL_ID",
      "label": "频道ID",
      "hint": "VoceChat的频道ID，不包含#号"
    },
    {
      "key": "VOCECHAT_ADMINS",
      "label": "管理员白名单",
      "hint": "管理员用户 ID 列表，多个用逗号分隔。用于记录渠道权限；本服务只发送事件通知，不执行聊天命令。"
    }
  ],
  "qq": [
    {
      "key": "QQ_APP_ID",
      "label": "AppID",
      "hint": "QQ 开放平台机器人 AppID"
    },
    {
      "key": "QQ_APP_SECRET",
      "label": "AppSecret",
      "hint": "QQ 开放平台机器人 AppSecret"
    },
    {
      "key": "QQ_OPENID",
      "label": "用户 OpenID",
      "hint": "默认接收者 openid（单聊），用户需曾与机器人交互过"
    },
    {
      "key": "QQ_GROUP_OPENID",
      "label": "群组 OpenID",
      "hint": "默认群组 openid（群聊），与用户 OpenID 二选一"
    },
    {
      "key": "QQBOT_ADMINS",
      "label": "管理员白名单",
      "hint": "管理员用户 ID 列表，多个用逗号分隔。用于记录渠道权限；本服务只发送事件通知，不执行聊天命令。"
    }
  ],
  "app_push": [
    {
      "key": "PUSH_SERVER_URL",
      "label": "消息中心地址（可选）",
      "hint": "留空使用 MoviePilot 已配置的中心服务；接入凭据由后台自动登记。"
    },
    {
      "key": "PUSH_RELAY_KEY",
      "label": "实例中转密钥",
      "hint": "通常无需填写。仅兼容已有 Server 手工密钥，至少 32 位；填写后停用自动接入。",
      "kind": "password"
    }
  ],
  "web_push": [
    {
      "key": "WEBPUSH_USERNAME",
      "label": "登录用户名",
      "hint": "只有对应的用户登录后才会推送消息"
    }
  ]
}
