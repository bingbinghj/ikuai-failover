export type NotificationFormat = 'text' | 'markdown' | 'html' | 'embed' | 'news'
export interface NotificationFormatOption { value: NotificationFormat; title: string }
type Channel = Record<string, unknown>
interface FormatProfile { defaultFormat: NotificationFormat; formats: NotificationFormatOption[] }

const text: NotificationFormatOption = { value: 'text', title: 'Text' }
const markdown: NotificationFormatOption = { value: 'markdown', title: 'Markdown' }
const profiles: Record<string, FormatProfile> = {
  telegram: { defaultFormat: 'markdown', formats: [{ value: 'markdown', title: 'MarkdownV2' }, { value: 'html', title: 'HTML' }, text] },
  slack: { defaultFormat: 'markdown', formats: [{ value: 'markdown', title: 'Markdown（mrkdwn）' }, text] },
  discord: { defaultFormat: 'embed', formats: [{ value: 'embed', title: 'Embed（MP 默认）' }, markdown, text] },
  ding_talk: { defaultFormat: 'markdown', formats: [markdown, text] },
  synology_chat: { defaultFormat: 'markdown', formats: [markdown, text] },
  voce_chat: { defaultFormat: 'markdown', formats: [markdown, text] },
  wechat: { defaultFormat: 'text', formats: [text, { value: 'news', title: 'News（图文）' }, markdown] },
  wechat_bot: { defaultFormat: 'markdown', formats: [markdown, text] },
  qy_wechat_bot: { defaultFormat: 'text', formats: [text, markdown] },
  feishu: { defaultFormat: 'markdown', formats: [{ value: 'markdown', title: 'Markdown（交互卡片）' }, text] },
  qq: { defaultFormat: 'markdown', formats: [markdown, text] },
  wechat_claw_bot: { defaultFormat: 'text', formats: [text] },
  web_push: { defaultFormat: 'text', formats: [text] },
  app_push: { defaultFormat: 'text', formats: [text] },
  web: { defaultFormat: 'text', formats: [text] },
  web_agent: { defaultFormat: 'markdown', formats: [markdown, text] },
}

function object(value: unknown): Channel { return value !== null && typeof value === 'object' && !Array.isArray(value) ? value as Channel : {} }
export function isWechatApp(channel: Channel | undefined): boolean {
  return channel?.provider === 'wechat' && object(channel.mp_config).WECHAT_MODE !== 'bot'
}
function profile(channel: Channel | undefined): FormatProfile {
  const provider = channel?.provider === 'wechat' && object(channel.mp_config).WECHAT_MODE === 'bot' ? 'wechat_bot' : String(channel?.provider ?? '')
  return profiles[provider] ?? { defaultFormat: 'text', formats: [text] }
}
export function notificationFormatOptions(channel: Channel | undefined): NotificationFormatOption[] { return profile(channel).formats }
export function notificationDefaultFormat(channel: Channel | undefined): NotificationFormat { return profile(channel).defaultFormat }
export function notificationMessageFormat(channel: Channel | undefined): NotificationFormat {
  const configured = channel?.message_format ?? (channel?.provider === 'qy_wechat_bot' ? channel.message_type : undefined)
  return notificationFormatOptions(channel).find(item => item.value === configured)?.value ?? notificationDefaultFormat(channel)
}

export function notificationFormatHint(channel: Channel | undefined, format = notificationMessageFormat(channel)): string {
  const provider = String(channel?.provider ?? '')
  const options = object(channel?.mp_config)
  if (format === 'news') return '尝试发送无封面、无跳转图文；接口明确拒绝时改为兼容文本。'
  if (format === 'embed') return 'MoviePilot 默认使用 Embed 卡片，标题与正文分开展示，正文支持 Discord Markdown。'
  if (format === 'html') return '使用 Telegram 支持的 HTML 消息格式。'
  if (['wechat_claw_bot', 'web_push', 'app_push', 'web'].includes(provider)) return '此渠道仅显示文本；Markdown 模板转换为可读文本。'
  if (format === 'text') return '以文本发送，保留文字与换行。'
  if (provider === 'telegram') return '使用 MoviePilot 默认的 Telegram MarkdownV2 格式。'
  if (provider === 'slack') return '使用 Slack 支持的 mrkdwn，转换标题、加粗、列表与链接。'
  if (provider === 'discord') return '使用 Discord Markdown 文本；MoviePilot 默认使用 Embed 卡片。'
  if (isWechatApp(channel)) return options.WECHAT_APP_MARKDOWN === true ? '原生 Markdown 仅在企业微信客户端显示；微信微工作台不支持此消息类型。' : 'Markdown 转为微信和企业微信可读文本；仅企业微信接收时可选择原生 Markdown。'
  if (provider === 'wechat') return '使用企业微信智能机器人支持的 Markdown。'
  if (provider === 'feishu') return '使用 MoviePilot 的飞书交互卡片，正文采用 Markdown。'
  if (provider === 'qq') return options.QQ_MARKDOWN === false ? '沿用旧配置，将 Markdown 转为兼容文本。' : '优先使用原生 Markdown；原生格式不受支持时发送兼容文本。'
  if (provider === 'web_agent') return '沿用 MoviePilot 助手的富文本格式，Markdown 由桥接接收端渲染。'
  return '使用此渠道支持的 Markdown 语法。'
}
