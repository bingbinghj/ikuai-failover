# 总览爱快信息

爱快信息卡读取路由器的系统名称、版本、运行时间及可用的 CPU/内存占用，不展示运行容器的主机或进程信息。系统信息单独每 30 秒更新，读取失败保留上次结果并标记，不阻塞故障检测。

沿用已登录的爱快 Web 会话，执行只读 `show` 请求：`homepage` 的 `TYPE=sysstat`。只提取 `hostname`、`uptime`、`verinfo.verstring/version/modelname`、`cpu[0]` 和 `memory.used/total`。CPU 数组首项为总占用，运行时间为秒，内存容量为 KB；序列号、设备标识等不返回控制台。

部分 4.0 固件不再通过 sysstat 返回 CPU/内存，此时调用 `monitor_system`，参数为空对象，读取 `data` 中时间最新的样本。历史采样明确标记，缺失字段不替换为宿主机数据。

返回结构依据已有设备响应及源作者的接口调用记录：`wycxy/ikuai` 提交 `b32e99f28e19c438ac35a73110cf2b661f905948` 的 `IKUAI-4.0.315-MATRIX.md` 与 `data_fetcher.py`；`beck-8/ikuai-mcp` 提交 `e2ac8352a43d56db142702792f55f5b217628f7a` 的 `API_REFERENCE.md`。这些记录覆盖部分 4.0 固件；其他版本缺失字段时允许信息卡暂未取得数据。
