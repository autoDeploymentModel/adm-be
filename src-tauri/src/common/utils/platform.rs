#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

/// 探测免密 sudo（sudo -n true）是否可用：可用则本机 docker 命令自动加 sudo -n 前缀
/// （docker 非 docker 组环境兜底，如 DGX Spark 默认用户）。结果缓存，仅首次探测。
fn sudo_available() -> bool {
    static SUDO_OK: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *SUDO_OK.get_or_init(|| {
        std::process::Command::new("sudo")
            .args(["-n", "true"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    })
}

/// 创建一个 docker 命令（统一入口）：免密 sudo 可用时自动 `sudo -n docker`，
/// 否则直接用 docker（需用户在 docker 组）。
pub fn docker_cmd() -> std::process::Command {
    if sudo_available() {
        let mut c = std::process::Command::new("sudo");
        c.arg("-n").arg("docker");
        c
    } else {
        std::process::Command::new("docker")
    }
}

/// 创建一个 docker 命令（tokio 版）。
pub fn docker_cmd_tokio() -> tokio::process::Command {
    if sudo_available() {
        let mut c = tokio::process::Command::new("sudo");
        c.arg("-n").arg("docker");
        c
    } else {
        tokio::process::Command::new("docker")
    }
}

#[cfg(target_os = "windows")]
pub fn create_hidden_command(program: impl AsRef<std::ffi::OsStr>) -> std::process::Command {
    let mut cmd = std::process::Command::new(program);
    cmd.creation_flags(0x08000000);
    cmd
}

#[cfg(not(target_os = "windows"))]
pub fn create_hidden_command(program: impl AsRef<std::ffi::OsStr>) -> std::process::Command {
    std::process::Command::new(program)
}

/// 让子进程独立于父进程启动（Unix 上创建新会话/进程组），
/// 这样关闭时才能用 `kill -9 -<pgid>` 一次性杀掉整棵进程树，避免孤儿残留。
#[cfg(not(target_os = "windows"))]
pub fn spawn_detached(cmd: &mut std::process::Command) -> std::io::Result<std::process::Child> {
    use std::os::unix::process::CommandExt;
    // process_group(0) 表示新建进程组，pgid 等于新进程自身的 pid
    cmd.process_group(0);
    // Command 的 builder 方法（args/stdout/stderr/env 等）均返回 &mut Self，
    // 因此调用方传入的链式表达式类型为 &mut Command，这里按可变引用接收，
    // spawn(&mut self) 同样基于可变引用执行。
    cmd.spawn()
}

/// 获取进程可执行文件名（小写）。路径不存在/无权限返回 None。
#[cfg(target_os = "windows")]
fn process_image_name(pid: u32) -> Option<String> {
    let out = create_hidden_command("tasklist")
        .args(["/FI", &format!("PID eq {}", pid), "/FO", "CSV", "/NH"])
        .output()
        .ok()?;
    let line = String::from_utf8_lossy(&out.stdout);
    let name = line.split(',').next()?.trim_matches('"');
    if name.is_empty() {
        None
    } else {
        Some(name.to_lowercase())
    }
}

#[cfg(not(target_os = "windows"))]
fn process_image_name(pid: u32) -> Option<String> {
    std::fs::read_to_string(format!("/proc/{}/comm", pid))
        .ok()
        .map(|s| s.trim().to_lowercase())
}

/// 仅当 pid 当前仍指向已知的模型进程（docker/sudo/llama-server）时才强杀。
/// 防止 pid 失效后被操作系统复用于无关进程（如远程会话组件）导致误杀。
fn is_known_model_process(pid: u32) -> bool {
    matches!(
        process_image_name(pid).as_deref(),
        Some(
            "docker" | "docker.exe" | "sudo" | "sudo.exe" | "llama-server" | "llama-server.exe"
        )
    )
}

/// 强杀整个进程树（含子进程），避免 llama-server 派生的子进程残留为孤儿。
///
/// - Windows: `taskkill /PID <pid> /T /F`
/// - Unix: 先尝试按进程组（kill -9 -<pgid>），失败再直接 kill PID
/// - 执行前校验 pid 仍指向已知模型进程（防 pid 复用误杀无关/会话级进程）
#[cfg(target_os = "windows")]
pub fn kill_process_tree(pid: u32) {
    use std::os::windows::process::CommandExt;
    if !is_known_model_process(pid) {
        return;
    }
    let _ = std::process::Command::new("taskkill")
        .creation_flags(0x08000000)
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .spawn();
}

#[cfg(not(target_os = "windows"))]
pub fn kill_process_tree(pid: u32) {
    if !is_known_model_process(pid) {
        return;
    }
    // 尝试杀掉整个进程组（llama-server 启动时已用 setsid 独立成组）
    let _ = std::process::Command::new("kill")
        .args(["-9", &format!("-{}", pid)])
        .spawn();
    // 兜底：直接杀 PID（进程组不存在时也不影响）
    let _ = std::process::Command::new("kill")
        .args(["-9", &pid.to_string()])
        .spawn();
}

/// 按进程名强杀所有匹配进程（整棵进程树）。用于关闭窗口时兜底清理残留。
///
/// - Windows: `taskkill /IM <name> /T /F`
/// - Unix: `pkill -9 -f <name>`
#[cfg(target_os = "windows")]
pub fn kill_process_by_name(name: &str) {
    use std::os::windows::process::CommandExt;
    let _ = std::process::Command::new("taskkill")
        .creation_flags(0x08000000)
        .args(["/IM", name, "/T", "/F"])
        .spawn();
}

#[cfg(not(target_os = "windows"))]
pub fn kill_process_by_name(name: &str) {
    let _ = std::process::Command::new("pkill")
        .args(["-9", "-f", name])
        .spawn();
}

pub fn get_gpu_info() -> (u64, u64, bool) {
    let mut total_vram: u64 = 0;
    #[allow(unused_mut)]
    let mut used_vram: u64 = 0;
    let mut has_gpu = false;

    #[cfg(target_os = "windows")]
    {
        // 使用 PowerShell Get-CimInstance 替代已弃用的 wmic（Windows 11 22H2+ 标记为 deprecated）
        if let Ok(output) = create_hidden_command("powershell")
            .args([
                "-NoProfile", "-NonInteractive", "-Command",
                "Get-CimInstance Win32_VideoController | Select-Object -ExpandProperty AdapterRAM",
            ])
            .output()
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                let trimmed = line.trim();
                if let Ok(ram) = trimmed.parse::<u64>() {
                    total_vram += ram;
                    has_gpu = true;
                }
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        // 方案 1：nvidia-smi 查询显存（NVIDIA 独立显卡）
        if let Ok(output) = create_hidden_command("nvidia-smi")
            .args([
                "--query-gpu=memory.total,memory.used",
                "--format=csv,noheader,nounits",
            ])
            .output()
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                let parts: Vec<&str> = line.split(',').collect();
                if parts.len() == 2 {
                    if let Ok(total) = parts[0].trim().parse::<u64>() {
                        total_vram += total * 1024 * 1024;
                        has_gpu = true;
                    }
                    if let Ok(used) = parts[1].trim().parse::<u64>() {
                        used_vram += used * 1024 * 1024;
                    }
                }
            }
        }

        // 方案 2：nvidia-smi 显存查询失败（如 GB10 SoC 显示 "Not Supported"），
        // 用 nvidia-smi -L 检测 GPU 是否存在，显存取系统总内存（统一内存架构）
        if !has_gpu {
            if let Ok(output) = create_hidden_command("nvidia-smi").arg("-L").output() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                if stdout.contains("GPU") {
                    has_gpu = true;
                    let sys = sysinfo::System::new_all();
                    total_vram = sys.total_memory();
                }
            }
        }
    }

    (total_vram, used_vram, has_gpu)
}

/// Linux 启动权限预检 + 自动修复（幂等，由 lib.rs setup() 延迟 3 秒调用）：
/// 1. 免密 sudo（sudo -n true）可用 → docker_cmd 自动加 sudo -n 前缀，无需处理
/// 2. docker CLI 不存在 → 无修复意义（Docker 安装属于 DGX 直连部署流程）
/// 3. docker info 可访问（用户已在 docker 组）→ 无需处理
/// 4. docker info 因 socket 权限拒绝 → 弹 pkexec 系统密码框将当前用户加入 docker 组。
///    成功后 emit "sudo-permission-fixed"（前端 toast 提示重新登录后免 sudo），不注销会话；
///    用户取消/拒绝（pkexec 退出码 126/127）静默，60 秒超时防 polkitd/agent 异常挂起，
///    其余失败仅记日志，设置页「权限修复」按钮仍可手动重试。
#[cfg(not(target_os = "windows"))]
pub async fn startup_ensure_docker_permission(app: tauri::AppHandle) {
    use tauri::Emitter;

    // 1) 免密 sudo 可用：后续 docker 命令自动走 sudo -n，无需干预
    if sudo_available() {
        return;
    }

    // 2) docker CLI 不存在：弹 usermod 加组无意义（Docker 安装属于 DGX 直连部署流程）
    let cli_ok = std::process::Command::new("sh")
        .args(["-c", "command -v docker >/dev/null 2>&1"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !cli_ok {
        crate::dbg_log!("startup permission check: docker CLI not found, skip pkexec fix");
        return;
    }

    // 3) 已在 docker 组可正常访问 daemon；仅 socket 权限拒绝时才需要加组
    //    （daemon 未启动/未安装属另一类问题，弹加组框无意义）
    match std::process::Command::new("docker").arg("info").output() {
        Ok(o) if o.status.success() => return,
        Ok(o) => {
            let err = String::from_utf8_lossy(&o.stderr).to_lowercase();
            if !err.contains("permission denied") {
                crate::dbg_log!(
                    "startup permission check: docker info failed without permission error: {}",
                    err.trim()
                );
                return;
            }
        }
        Err(e) => {
            crate::dbg_log!("startup permission check: docker info spawn failed: {}", e);
            return;
        }
    }

    let user = std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_default();
    if user.is_empty() {
        crate::dbg_log!("startup permission check: USER/LOGNAME env unset, skip pkexec fix");
        return;
    }

    // 4) pkexec 弹系统原生密码框执行修复（与设置页 fix_docker_permission 同一命令，幂等）。
    //    pkexec 退出码语义：126 = 用户取消（Request dismissed），127 = 未授权（用户拒绝），
    //    二者均属正常用户交互分支，静默；其余（polkitd 通信失败、无 agent 等）写日志便于排查。
    //    kill_on_drop：超时后杀掉挂起的 pkexec，避免孤儿进程。
    let pkexec = tokio::process::Command::new("pkexec")
        .args(["usermod", "-aG", "docker", &user])
        .kill_on_drop(true)
        .output();
    match tokio::time::timeout(std::time::Duration::from_secs(60), pkexec).await {
        Ok(Ok(o)) if o.status.success() => {
            let _ = app.emit("sudo-permission-fixed", serde_json::json!({ "user": user }));
        }
        Ok(Ok(o)) => {
            let code = o.status.code().unwrap_or(-1);
            let err_raw = String::from_utf8_lossy(&o.stderr);
            let err = err_raw.to_lowercase();
            let user_action = code == 126
                || (code == 127
                    && (err.contains("not authorized") || err.contains("incident has been reported")));
            if !user_action {
                crate::dbg_log!("startup pkexec usermod failed (exit {}): {}", code, err_raw.trim());
            }
        }
        Ok(Err(e)) => {
            crate::dbg_log!("pkexec unavailable: {}", e);
        }
        Err(_) => {
            crate::dbg_log!("startup pkexec usermod timed out after 60s, process killed");
        }
    }
}
