import wechatLogo from './assets/notification/wechat.png'
import telegramLogo from './assets/notification/telegram.webp'
import feishuLogo from './assets/notification/feishu.png'
import slackLogo from './assets/notification/slack.webp'
import discordLogo from './assets/notification/discord.png'
import dingtalkLogo from './assets/notification/dingtalk.jpg'
import synologyLogo from './assets/notification/synologychat.png'
import voceLogo from './assets/notification/vocechat.png'
import qqLogo from './assets/notification/qq.png'
import notificationLogo from './assets/notification/notification.png'

export const notificationProviderIcons: Readonly<Record<string, string>> = Object.freeze({
  wechat: wechatLogo,
  qy_wechat_bot: wechatLogo,
  wechat_claw_bot: wechatLogo,
  telegram: telegramLogo,
  feishu: feishuLogo,
  slack: slackLogo,
  discord: discordLogo,
  ding_talk: dingtalkLogo,
  synology_chat: synologyLogo,
  voce_chat: voceLogo,
  qq: qqLogo,
})
export const defaultNotificationIcon = notificationLogo
export function notificationProviderIcon(provider: unknown): string {
  const icon = typeof provider === 'string' ? notificationProviderIcons[provider] : undefined
  return typeof icon === 'string' ? icon : defaultNotificationIcon
}
