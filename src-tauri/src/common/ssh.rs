// ===== SSH 远端执行基建（多机互联用）=====
// 调系统 ssh 客户端，BatchMode 免交互、防挂起；不存储密码，仅支持密钥认证
// （-i 指定私钥，或依赖 ssh-agent / 默认 key）。

use crate::common::error::AppError;

/// 校验光口 IP 格式（IPv4 点分十进制 / IPv6）。SSH 客户端对非法主机名只会笼统报
/// "hostname contains invalid characters"，这里提前给出带实际值的友好提示。
/// 节点清单字段为「光口 IP」，不接受主机名。
pub fn validate_host(host: &str) -> Result<(), AppError> {
    let h = host.trim();
    if h.is_empty() {
        return Err(AppError::msg("节点 IP 为空".to_string()));
    }
    let ok = is_ipv4(h) || is_ipv6(h);
    if !ok {
        return Err(AppError::msg(format!(
            "节点 IP「{}」不是合法的 IP 地址（光口 IP 需为 IPv4 点分十进制，如 192.168.100.2；或 IPv6）",
            h
        )));
    }
    Ok(())
}

/// IPv4 点分十进制：4 段、每段 0-255 的十进制数
fn is_ipv4(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() != 4 {
        return false;
    }
    parts.iter().all(|p| {
        !p.is_empty()
            && p.len() <= 3
            && p.bytes().all(|b| b.is_ascii_digit())
            && p.parse::<u16>().map(|v| v <= 255).unwrap_or(false)
    })
}

/// IPv6（宽松）：含 `:` 的十六进制/冒号/点组合，至多一个 `::`，可带 `%` 接口名；
/// 无 `::` 时必须不含 `.`（杜绝 `192.168.100.2:22` 这类 IP:端口混填被放行）
fn is_ipv6(s: &str) -> bool {
    let body = s.split('%').next().unwrap_or(s);
    if !body.contains(':') {
        return false;
    }
    (body.contains("::") || !body.contains('.'))
        && body.split("::").count() <= 2
        && body.matches(':').count() <= 7
        && body
            .chars()
            .all(|c| c.is_ascii_hexdigit() || c == ':' || c == '.')
}

/// 校验 SSH 用户名。OpenSSH 按 `user@host` 解析，用户名若含 `@`（如误粘贴
/// `user@192.168.100.2` 完整目标）会导致 host 段带 `@`，报 "hostname contains
/// invalid characters"。
pub fn validate_ssh_user(user: &str) -> Result<(), AppError> {
    let u = user.trim();
    if u.is_empty() {
        return Err(AppError::msg("SSH 用户为空".to_string()));
    }
    let ok = u.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.');
    if !ok {
        return Err(AppError::msg(format!(
            "SSH 用户「{}」包含非法字符（只能含字母/数字/下划线/短横线/句点；不要粘贴 user@ip 完整形式）",
            u
        )));
    }
    Ok(())
}

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

/// 远端节点探活脚本：检查 GPU / Docker daemon / 镜像 / 模型目录。
///
/// 输出（每行 `LABEL:VALUE` 形式，便于 strip_prefix 解析）：
/// `GPU:` 行 GPU 名（`nvidia-smi` 探不到为空）；
/// `DOCKER:` 行 Docker ServerVersion（异常为 `DOCKER_ERR`）；
/// `IMAGE:` 行 `IMAGE_OK` / `IMAGE_MISSING` / `IMAGE:SKIPPED`（`check_image=false` 时输出 SKIPPED，
///           用于未指定模型的纯环境探测场景）；
/// `IMG_ERR:` 行镜像检查失败原因（仅 IMAGE_MISSING 时输出）；
/// `MODEL:` 行 `MODEL_OK` / `MODEL_MISSING`。
///
/// `model_dir_root_mode=true`：model_dir 为模型根目录，目录存在且其中任一子目录有 `.done` 即 MODEL_OK；
/// `model_dir_root_mode=false`：model_dir 为精确模型目录，自身有 `.done` 才 MODEL_OK。
///
/// 注：远端无需探测网卡——SSH 可达即说明互联已通。
pub fn probe_script(model_dir: &str, image: &str, model_dir_root_mode: bool, check_image: bool) -> String {
    let dir_q = quote_remote_path(model_dir);
    let model_chk = if model_dir_root_mode {
        format!(
            "if [ -d {d} ] && ( [ -e {d}/.done ] 2>/dev/null || ls {d}/*/.done >/dev/null 2>&1 ); then echo MODEL_OK; else echo MODEL_MISSING; fi",
            d = dir_q
        )
    } else {
        format!(
            "if [ -d {d} ] && [ -e {d}/.done ]; then echo MODEL_OK; else echo MODEL_MISSING; fi",
            d = dir_q
        )
    };
    let img = sh_quote(image);
    let image_block = if !check_image || image.is_empty() {
        "echo \"IMAGE:SKIPPED\"".to_string()
    } else {
        // 使用 echo "LABEL:$(...)" 确保 label 和 value 在同一行，
        // 解析端用 strip_prefix("LABEL:") 即可提取 value。
        format!(
            "echo \"IMAGE:$(sudo -n docker image inspect {img} >/dev/null 2>&1 || docker image inspect {img} >/dev/null 2>&1 && echo IMAGE_OK || echo IMAGE_MISSING)\"; \
             if ! (sudo -n docker image inspect {img} >/dev/null 2>&1 || docker image inspect {img} >/dev/null 2>&1); then echo \"IMG_ERR:$( (sudo -n docker image inspect {img} 2>&1 || docker image inspect {img} 2>&1) | tail -1 )\"; fi",
            img = img
        )
    };
    format!(
        "echo \"GPU:$(nvidia-smi --query-gpu=name --format=csv,noheader 2>/dev/null | head -1)\"; \
         echo \"DOCKER:$(sudo -n docker info --format '{{{{.ServerVersion}}}}' 2>/dev/null || docker info --format '{{{{.ServerVersion}}}}' 2>/dev/null || echo DOCKER_ERR)\"; \
         {img_block}; \
         echo \"MODEL:$({chk})\"",
        img_block = image_block,
        chk = model_chk
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
/// 自动尝试 sudo -n，失败回退普通 docker（兼容非 docker 组远端）。
pub fn stop_container_script(container_name: &str) -> String {
    let c = sh_quote(container_name);
    format!(
        "(sudo -n docker stop -t 5 {} 2>/dev/null || docker stop -t 5 {} 2>/dev/null); (sudo -n docker rm -f {} 2>/dev/null || docker rm -f {} 2>/dev/null); echo DONE",
        c, c, c, c
    )
}

/// 远端启动容器脚本：nohup 后台运行 docker run，日志落盘，立即回显 STARTED。
/// 所有参数 token 经单引号转义后拼入远端 shell 命令行（~ 路径前缀保持可展开）。
/// `use_sudo=true` 时 docker 命令加 `sudo -n` 前缀（远端非 docker 组环境）。
pub fn start_container_script(container_name: &str, docker_args: &[String], log_path: &str, use_sudo: bool) -> String {
    let dk = if use_sudo { "sudo -n docker" } else { "docker" };
    let mut cmd = format!("{dk}");
    for a in docker_args {
        cmd.push(' ');
        cmd.push_str(&quote_remote_path(a));
    }
    format!(
        "nohup {} > {} 2>&1 < /dev/null & echo STARTED; sleep 1; {} ps --filter name={} --format '{{{{.Names}}}} {{{{.Status}}}}' || true",
        cmd,
        sh_quote(log_path),
        dk,
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