// ===== 密码 SSH 执行基建（DGX 双机直连一键部署用）=====
// russh 纯 Rust SSH 客户端：承载首次部署阶段——现有 ssh_run（common/ssh.rs）走系统 ssh
// 仅支持密钥认证（BatchMode），OpenSSH 密码输入需要 tty，无法承载带密码流程。
//
// 设计约束：
// - 密码只存内存：不落盘、不进日志/事件；任何外发字符串（事件 detail / dbg_log!）统一先过 redact()
// - 首连信任服务器密钥（与 ssh_run 的 StrictHostKeyChecking=accept-new 语义一致，密钥不持久化）
// - 超时统一 tokio::time::timeout 兜底，退出前显式 disconnect，避免句柄泄漏

use crate::common::error::AppError;
use russh::client;
use russh::keys::PublicKeyOrCertificate;
use russh::{ChannelMsg, Disconnect};
use std::time::Duration;

/// 将 secrets（密码等）在字符串中的出现替换为 ******（事件/日志外发前统一调用）。
/// 同时处理 shell 单引号/双引号包裹变体（命令串经 sh_quote 后密码带引号）。
pub fn redact(s: &str, secrets: &[&str]) -> String {
    let mut out = s.to_string();
    for sec in secrets {
        if sec.is_empty() || sec.len() < 4 {
            continue;
        }
        out = out.replace(sec, "******");
        out = out.replace(&format!("'{}'", sec), "'******'");
        out = out.replace(&format!("\"{}\"", sec), "\"******\"");
    }
    out
}

#[derive(Clone, Default)]
struct PwHandler;

impl client::Handler for PwHandler {
    type Error = russh::Error;

    /// accept-new 语义（首次连接即信任，与 ssh_run 的 StrictHostKeyChecking=accept-new 对齐）；
    /// 不持久化到 known_hosts（部署流程无需记录）。
    async fn check_server_key(&mut self, _key: &PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

/// 密码 SSH 远端执行单条命令，返回 (status_success, stdout, stderr)。
///
/// - `timeout`：命令执行整体超时（不含连接/认证，各 10s）
/// - 密码错误 / 地址不可达 / 认证失败均返回带宿主信息的 Err
pub async fn ssh_pw_run(
    host: &str,
    port: u16,
    user: &str,
    pass: &str,
    cmd: &str,
    timeout: Duration,
) -> Result<(bool, String, String), AppError> {
    let host = host.trim();
    let user = user.trim();
    if host.is_empty() {
        return Err(AppError::msg("节点 IP 为空".to_string()));
    }
    if user.is_empty() {
        return Err(AppError::msg("SSH 用户为空".to_string()));
    }
    if pass.is_empty() {
        return Err(AppError::msg("SSH 密码为空".to_string()));
    }

    let mut cfg = client::Config::default();
    // 无数据接收容忍 = 命令整体超时（长命令如 docker 安装期间远端可能长时间无输出）
    cfg.inactivity_timeout = Some(timeout);
    cfg.keepalive_interval = Some(Duration::from_secs(15));

    let mut session = tokio::time::timeout(
        Duration::from_secs(10),
        client::connect(cfg.into(), (host.to_string(), port), PwHandler),
    )
    .await
    .map_err(|_| AppError::msg(format!("SSH 连接 {} 超时（请确认地址可达、22 端口开放/已配推流）", host)))?
    .map_err(|e| AppError::msg(format!("SSH 连接 {} 失败: {}", host, e)))?;

    let auth = tokio::time::timeout(
        Duration::from_secs(10),
        session.authenticate_password(user, pass),
    )
    .await
    .map_err(|_| AppError::msg(format!("SSH 认证 {} 超时", host)))?
    .map_err(|e| AppError::msg(format!("SSH 认证 {} 失败: {}", host, e)))?;
    if !auth.success() {
        return Err(AppError::msg(format!(
            "SSH 认证失败（用户名或密码错误？）: {}@{}",
            user, host
        )));
    }

    let mut channel = session
        .channel_open_session()
        .await
        .map_err(|e| AppError::msg(format!("SSH 打开会话失败: {}", e)))?;

    let result = tokio::time::timeout(timeout, async {
        channel
            .exec(true, cmd)
            .await
            .map_err(|e| AppError::msg(format!("SSH 执行命令失败: {}", e)))?;

        let mut stdout = String::new();
        let mut stderr = String::new();
        let mut status: Option<u32> = None;
        while let Some(msg) = channel.wait().await {
            match msg {
                ChannelMsg::Data { data } => {
                    stdout.push_str(&String::from_utf8_lossy(&data));
                }
                ChannelMsg::ExtendedData { data, ext: 1 } => {
                    stderr.push_str(&String::from_utf8_lossy(&data));
                }
                ChannelMsg::ExitStatus { exit_status } => {
                    status = Some(exit_status);
                }
                ChannelMsg::Close => break,
                _ => {}
            }
        }
        let _ = channel.close().await;
        Ok::<_, AppError>((status.map(|s| s == 0).unwrap_or(false), stdout, stderr))
    })
    .await;

    let _ = session.disconnect(Disconnect::ByApplication, "", "").await;

    match result {
        Ok(r) => r,
        Err(_) => Err(AppError::msg(format!(
            "SSH 执行超时（{}s）: {}",
            timeout.as_secs(),
            host
        ))),
    }
}