# 分流配置指南

规则应结合实际出口、默认路由和 DNS 路径选择。本文使用通用角色描述，不包含部署方的现场配置、地址或设备快照。

## 覆盖与规则来源

| 规则 | 用途 | 注意事项 |
| --- | --- | --- |
| 国内 CIDR | 按目标地址选择物理出口 | 主备规则的流量选择器应相同，两条同时启用不能证明严格主备 |
| GFW 和业务代理域名 | 指定流量进入旁路由 | 最终直连或代理由旁路由自身规则决定 |
| 直连例外 | 覆盖需要直连的特定业务 | 与代理域名、IP 分类和 DNS 结果保持一致 |
| 默认 IPv4 路由 | 接收没有匹配其他规则的流量 | 应确认最终出口及防回环路径，避免将未匹配等同于无路由 |
| IPv6 | 独立的地址、路由、PD 与 LAN 通告 | IPv4 地址列表或 Fake IPv6 路由不能证明所有真实 AAAA 流量被覆盖 |

- 保留有维护来源的基础库，优先补充实际使用的 AI、开发、通信和媒体业务；重复合并多个同源大名单不等于提高覆盖率。
- [Loyalsoldier/v2ray-rules-dat](https://github.com/Loyalsoldier/v2ray-rules-dat) 的 GFW 文本可作为原版域名基础。其他导出文件可能含 `full:`、`regexp:` 等规则，接入前逐项确认格式。
- [Loyalsoldier/geoip](https://github.com/Loyalsoldier/geoip) 和 [gaoyifan/china-operator-ip](https://github.com/gaoyifan/china-operator-ip) 提供 CIDR 数据；同源国内数据不必重复叠加。
- [MetaCubeX/meta-rules-dat](https://github.com/MetaCubeX/meta-rules-dat)、[blackmatrix7/ios_rule_script](https://github.com/blackmatrix7/ios_rule_script) 和 [v2fly/domain-list-community](https://github.com/v2fly/domain-list-community) 可提供业务分类。后缀、精确主机、正则、IP 和引用指令应分别处理，不能将混合规则原样导入纯域名字段。
- 仓库近期维护不代表每一份业务文件都刚刚更新；核对具体文件的提交和格式，并记录数据来源。

## 原版模式与输入格式

- `ispdomain` 更新自定义运营商和域名规则；配置中的 IP 分组、IPv6 分组与端口分流属于其他模块，不能视为已经生效的补充。
- 不应仅为启用闲置模块而改变模式。切换组合模式前先核对匹配方向、反向选择、协议、源范围和固件枚举。
- 原版域名下载和过滤逻辑不解释 Clash/Surge 指令、YAML、`full:`、`regexp:`、属性或 `include`。转换时保留原始语义，不能无条件把精确主机改成后缀。
- 原版通过 HTTP(S) 下载规则，不直接读取本地挂载文件或 `file://`。自维护纯文本应放在可访问的 HTTP(S) 或 GitHub raw 地址。
- 自维护名单应去重并清理失效项。带有类似 `-del` 后缀的字面域名不是跨名单删除指令。
- 原版域名更新会写入 `enabled=yes` 和固定优先级。配置中增加未支持的优先级字段不能改变源码行为，例外的最终目标需要明确。

## 与主备协调

- 在本地配置填写实际规则 ID、标签和接口，保持成对规则选择相同流量。删除、改名或改变匹配条件后应同步故障转移配置。
- 定时更新如果重置域名启停状态，后续协调循环会重新读取并恢复主备互斥。两条名单分别更新期间若选择器暂时不同，会暂停该轮应用，更新完成一致后继续。
- 自定义运营商地址更新与运营商策略启停是不同操作，不应把域名的启停重置行为泛化到所有资源。
- 直连例外需与 DNS 白名单、Fake-IP 排除和旁路由策略配合，避免把 Fake 地址送到物理出口。新增 VLAN、VPN 或子网时按进入爱快的实际源地址核对范围。
- 源于旁路由自身的出口流量需要明确的物理出口规则，避免默认路由把流量再次送回旁路由。
- tracker 域名不能覆盖所有下载 Peer。需要单独分流某项业务时优先使用专用源地址；监听端口不能当作所有出站连接的固定源端口，也不能无差别改变同一主机其他服务的出口。

## 参考

- [原版分流模式说明](https://github.com/joyanhui/ikuai-bypass/blob/main/docs/router-mode.md)
- [原版域名规则写入源码](https://github.com/joyanhui/ikuai-bypass/blob/main/crates/core/src/ikuai/stream_domain.rs)
- [原版模块更新源码](https://github.com/joyanhui/ikuai-bypass/blob/main/crates/core/src/update.rs)

实际部署配置、规则统计与诊断记录应保留在本地，不作为公开项目文档或配置模板发布。
