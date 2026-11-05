登录校验依据 joyanhui/ikuai-bypass 原版 apps/cli/src/web.rs 的 basic_auth 和 constant_time_eq。原版账号来自 webui.user/pass，和爱快 username/password 分开；密码按原文 UTF8 字节比较，不使用密码哈希或权限角色检查。webui.user 为空时关闭认证。原版没有 Cookie、服务端登录会话、过期时间或注销令牌。

本服务保留 Basic Authorization 的账号密码比较方式。启动时由配置模块为空缺的 WebUI 账号密码生成随机值并保存，这属于本服务的初始化行为，不宣称原版存在此功能。IKB_WEBUI_USER/PASS 是本服务新增的部署变量，非空值去除首尾空白后覆盖 webui 配置；原版对应环境覆盖代码处于注释块中。

为支持刷新页面和容器重启后继续登录，本服务使用 Cookie 会话。/auth/login 仍用提交的 Basic 账号密码校验，并接受可选 JSON 字段 remember。记住密码开启时使用 webui.remember_lifetime_seconds，有效期默认 30 天，并设置 Cookie 的 Max-Age；未开启时使用 webui.session_lifetime_seconds，令牌有效期默认 7 天，Cookie 随浏览器会话结束。两个有效期可配置为 60 秒至 365 天，记住密码有效期不能短于普通会话。Cookie 使用 Path=/、HttpOnly、SameSite=Strict，页面只保存用户名和记住密码偏好，不将明文密码写入 localStorage。Cookie 只携带版本、到期时间、随机值及 HMAC-SHA256 签名，不包含账号密码。签名使用系统安全随机数生成的密钥，保存到状态目录的 auth-session.key；签名同时绑定当前 WebUI 凭据，修改这些凭据并重启服务后，旧 Cookie 无法继续认证。现有 v1 Cookie 保持兼容。直接 HTTPS 请求额外使用 Secure 属性；当前 HTTP 部署可使用会话，HTTPS 反向代理应在代理层设置 Secure 属性。

/auth/session 与受保护 API 同时支持现有 Basic 和有效 Cookie。/auth/logout 清除当前浏览器的 Cookie；它不修改连接爱快的账号或其他浏览器的登录凭据。Cookie 认证的写操作核对浏览器 Origin/Host 及 Sec-Fetch-Site，拒绝跨源请求。认证状态响应禁用缓存，未认证的 API 返回 401 JSON，不发送 WWW-Authenticate，因此浏览器不会弹出原生 Basic 登录框，页面统一返回自身登录界面。AuthState::new 保留原 Basic 模式；生产服务使用 with_state_dir 启用持久会话。

协议与库依据：HMAC crate 的 Mac::verify_slice、getrandom 的系统随机源，以及 HTTP Set-Cookie 的 HttpOnly、SameSite、Max-Age 属性。新增会话代码在本项目中实现，不复制外部实现。
