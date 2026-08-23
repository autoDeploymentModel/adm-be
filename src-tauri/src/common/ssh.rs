// ===== SSH 远端执行基建（多机互联用）=====
// 调系统 ssh 客户端，BatchMode 免交互、防挂起；不存储密码，仅支持密钥认证
// （-i 指定私钥，或依赖 ssh-agent / 默认 key）。

use crate::common::error::AppError;

/// 在远端执行命令，返回 (status_success, stdout, stderr)。
///
/// - `key`：私钥路径（`-i`），None 时用 ssh-agent / 默认 key
/// - `timeout`：整体超时（含连接与命令执行），超时强制 kill 子进程并返回 Err
pub async fn ssh_run(
    host: &str,
    user: &str,
    port: u16,
    key: Option<&str>,
    cmd: &str,
    timeout: std::time::Duration,
) -> Result<(bool, String, String), AppError> {
    if host.trim().is_empty() {
        return Err(AppError::msg("节点 IP 为空".to_string()));
    }
    let mut ssh = tokio::process::Command::new("ssh");
    ssh.arg("-o").arg("BatchMode=yes")
        .arg("-o").arg("ConnectTimeout=5")
        .arg("-o").arg("StrictHostKeyChecking=accept-new")
        .arg("-o").arg("ServerAliveInterval=15")
        .arg("-p").arg(port.to_string());
    if let Some(k) = key.filter(|k| !k.trim().is_empty()) {
        ssh.arg("-i").arg(k.trim());
    }
    ssh.arg(format!("{}@{}", user.trim(), host.trim()))
        .arg(cmd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let mut child = ssh
        .spawn()
        .map_err(|e| AppError::msg(format!("执行 ssh 失败（本机未安装 OpenSSH 客户端？）: {}", e)))?;

    // spawn + 显式等待：超时时 kill 子进程，避免孤儿 ssh 泄漏
    //（tokio Child 默认 kill_on_drop=false，直接 timeout(output()) 会让子进程继续运行）
    let out = tokio::time::timeout(timeout, async {
        let mut stdout = String::new();
        let mut stderr = String::new();
        if let Some(mut so) = child.stdout.take() {
            let _ = tokio::io::AsyncReadExt::read_to_string(&mut so, &mut stdout).await;
        }
        if let Some(mut se) = child.stderr.take() {
            let _ = tokio::io::AsyncReadExt::read_to_string(&mut se, &mut stderr).await;
        }
        let status = child
            .wait()
            .await
            .map_err(|e| AppError::msg(format!("ssh 等待失败: {}", e)))?;
        Ok::<(bool, String, String), AppError>((status.success(), stdout, stderr))
    })
    .await;

    match out {
        Ok(res) => res,
        Err(_) => {
            let _ = child.kill();
            Err(AppError::msg(format!("SSH 连接 {} 超时（{}s）", host, timeout.as_secs())))
        }
    }
}

/// 远端探活脚本：nvidia-smi 简版 + docker 版本 + 本机所用镜像存在性 + 模型目录存在性。
/// 输出解析约定（换行分隔）：
/// `GPU:` 行 GPU 名（无 GPU 时为空）；`DOCKER:` 行 docker ServerVersion（异常时 "DOCKER_ERR"）；
/// `IMAGE:` 行 "IMAGE_OK" / "IMAGE_MISSING"（远端是否已下载 `image`）；
/// `MODEL:` 行 "MODEL_OK" / "MODEL_MISSING"。
/// 注：远端无需探测网卡——SSH 可达即说明互联已通。
pub fn probe_script(model_dir: &str, image: &str) -> String {
    format!(
        "echo 'GPU:'; nvidia-smi --query-gpu=name --format=csv,noheader 2>/dev/null | head -1; \
         echo 'DOCKER:'; docker info --format '{{{{.ServerVersion}}}}' 2>/dev/null || echo DOCKER_ERR; \
         echo 'IMAGE:'; docker image inspect {} >/dev/null 2>&1 && echo IMAGE_OK || echo IMAGE_MISSING; \
         echo 'MODEL:'; if [ -d {} ] && [ -d {}/.done ]; then echo MODEL_OK; else echo MODEL_MISSING; fi",
        sh_quote(image),
        quote_remote_path(model_dir),
        quote_remote_path(model_dir)
    )
}

/// 简单 shell 单引号转义（远端命令在远端 shell 中执行，防路径/参数含特殊字符）
pub fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// 远端路径转义：`~/` 前缀保持引号外（否则波浪号不展开），其余走 sh_quote。
pub fn quote_remote_path(s: &str) -> String {
    if let Some(rest) = s.strip_prefix("~/") {
        format!("~/{}", sh_quote(rest))
    } else {
        sh_quote(s)
    }
}

/// 远端停止/删除容器（多机停止用）：docker stop -t 5 + docker rm -f。
pub fn stop_container_script(container_name: &str) -> String {
    let c = sh_quote(container_name);
    format!(
        "docker stop -t 5 {} >/dev/null 2>&1; docker rm -f {} >/dev/null 2>&1; echo DONE",
        c, c
    )
}

/// 远端启动容器脚本：nohup 后台运行 docker run，日志落盘，立即回显 STARTED。
/// 所有参数 token 经单引号转义后拼入远端 shell 命令行（~ 路径前缀保持可展开）。
pub fn start_container_script(container_name: &str, docker_args: &[String], log_path: &str) -> String {
    let mut cmd = format!("docker");
    for a in docker_args {
        cmd.push(' ');
        cmd.push_str(&quote_remote_path(a));
    }
    format!(
        "nohup {} > {} 2>&1 < /dev/null & echo STARTED; sleep 1; docker ps --filter name={} --format '{{{{.Names}}}} {{{{.Status}}}}' || true",
        cmd,
        sh_quote(log_path),
        sh_quote(container_name)
    )
}

/// 同步版 ssh 执行（应用退出兜底清理用，进程内轮询等待，超时强杀）。
pub fn ssh_run_blocking(
    host: &str,
    user: &str,
    port: u16,
    key: Option<&str>,
    cmd: &str,
    timeout: std::time::Duration,
) -> Result<(bool, String, String), AppError> {
    let mut ssh = std::process::Command::new("ssh");
    ssh.arg("-o").arg("BatchMode=yes")
        .arg("-o").arg("ConnectTimeout=3")
        .arg("-o").arg("StrictHostKeyChecking=accept-new")
        .arg("-p").arg(port.to_string());
    if let Some(k) = key.filter(|k| !k.trim().is_empty()) {
        ssh.arg("-i").arg(k.trim());
    }
    ssh.arg(format!("{}@{}", user.trim(), host.trim()))
        .arg(cmd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let mut child = ssh.spawn()
        .map_err(|e| AppError::msg(format!("执行 ssh 失败（本机未安装 OpenSSH 客户端？）: {}", e)))?;

    // 轮询等待结果，超时强杀（防 docker/远端命令挂死阻塞应用退出）
    let deadline = std::time::Instant::now() + timeout;
    let mut status = None;
    while std::time::Instant::now() < deadline {
        match child.try_wait() {
            Ok(Some(st)) => {
                status = Some(st);
                break;
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(200)),
            Err(e) => {
                let _ = child.kill();
                return Err(AppError::msg(format!("ssh 等待失败: {}", e)));
            }
        }
    }
    if status.is_none() {
        let _ = child.kill();
        return Err(AppError::msg(format!("SSH 连接 {} 超时（{}s）", host, timeout.as_secs())));
    }
    let out = child.wait_with_output().map_err(|e| AppError::msg(format!("ssh 读取输出失败: {}", e)))?;
    Ok((
        status.map(|s| s.success()).unwrap_or(false),
        String::from_utf8_lossy(&out.stdout).trim().to_string(),
        String::from_utf8_lossy(&out.stderr).trim().to_string(),
    ))
}