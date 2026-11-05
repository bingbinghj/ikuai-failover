#![deny(clippy::unwrap_used, clippy::expect_used)]

use clap::Parser;
use ikb_failover::{
    api::{Client, DisabledClient, RouterApi},
    config, logging,
    runtime::Runtime,
};
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "/etc/ikuai-failover/config.yml")]
    config: PathBuf,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    check: bool,
    #[arg(long)]
    once: bool,
}

#[tokio::main]
async fn main() {
    if let Err(error) = run(Args::parse()).await {
        if logging::initialized() {
            logging::error(&format!("运行失败：{error}"));
        } else {
            // Invalid YAML may contain credentials; do not echo parser excerpts.
            logging::error("服务启动失败，请检查配置文件、目录挂载和读写权限。");
        }
        logging::flush();
        std::process::exit(1);
    }
    logging::flush();
}

async fn run(args: Args) -> Result<(), String> {
    let mut config = config::load(&args.config).await?;
    if args.dry_run {
        config.management.apply = false;
        config.notifications.enabled = false;
    }
    if args.check {
        println!(
            "{}",
            serde_json::json!({ "config_valid": true, "apply": config.management.apply,
            "preserved_default": config.wan.preserved_default, "ipv6_capability": "requires_runtime_preflight",
            "notification_channels": config.notifications.channels.iter().map(|channel|channel.provider.name()).collect::<Vec<_>>() })
        );
        return Ok(());
    }
    let client: Arc<dyn RouterApi> = if config.ikuai.has_credentials() {
        Arc::new(Client::new(&config.ikuai)?)
    } else {
        Arc::new(DisabledClient)
    };
    let runtime = Arc::new(Runtime::new(config, client).await?);
    if runtime.config.wan.enabled && !runtime.config.ikuai.has_credentials() {
        logging::warn("请在配置页面或 config.yml 填写爱快连接账号密码，保存后重启服务。");
    }
    runtime
        .notify(
            "service_started",
            "故障转移服务已启动",
            if runtime.config.wan.enabled && !runtime.config.ikuai.has_credentials() {
                "控制台已启动；请填写爱快账号密码并重启服务后启用线路监控。"
            } else if runtime.config.management.apply {
                "已加载配置，故障转移已开启。"
            } else {
                "已加载配置，当前只生成计划，路由器策略未开启自动应用。"
            },
        )
        .await;
    if args.once {
        let result = match tokio::time::timeout(Duration::from_secs(120), runtime.cycle()).await {
            Ok(result) => result,
            Err(_) => Err("Single cycle exceeded its timeout".into()),
        };
        if result.is_err() {
            runtime
                .notify(
                    "service_failed",
                    "故障转移任务失败",
                    "单次检查或策略应用失败，请查看状态与运行日志。",
                )
                .await;
        }
        runtime
            .notify("service_stopped", "故障转移任务已结束", "单次任务已结束。")
            .await;
        runtime.events.flush(Duration::from_secs(15)).await?;
        result?;
        println!("{}", runtime.snapshot().await);
        return Ok(());
    }
    let cancel = CancellationToken::new();
    let run = Arc::clone(&runtime).run(cancel.clone());
    tokio::pin!(run);
    tokio::select! {
        result = &mut run => {
            if result.is_err() { runtime.notify("service_failed", "故障转移服务异常退出", "服务运行失败，请查看运行日志。" ).await; }
            runtime.events.flush(Duration::from_secs(15)).await?;
            result
        },
        result = shutdown_signal() => {
            result.map_err(|e| e.to_string())?;
            runtime.notify("service_stopped", "故障转移服务正在退出", "收到停止信号，服务正常退出。" ).await;
            cancel.cancel();
            let result = run.await;
            runtime.events.flush(Duration::from_secs(15)).await?;
            result
        },
    }
}

async fn shutdown_signal() -> Result<(), std::io::Error> {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! { result = tokio::signal::ctrl_c() => result, _ = terminate.recv() => Ok(()) }
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c().await
}
