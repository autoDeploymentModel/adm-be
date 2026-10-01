#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

/// 探测免密 sudo（sudo -n true）是否可用：可用则本机 docker 命令自动加 sudo -n 前缀
/// （docker 非 docker 组环境兜底，如 DGX Spark 默认用户）。结果缓存，仅首次探测。
#[cfg(not(target_os = "windows"))]
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

/// 本机 docker 调用方式（首次调用时探测，结果缓存）：
/// - `Sudo`：免密 sudo 可用 → `sudo -n docker`
/// - `Direct`：当前登录会话已具备 docker socket 属组身份 → 直接 `docker`
/// - `Shim`：账号已在组但当前会话未带上组身份，且 `sg <组>` 可免密取得组身份
///   → 经自动生成的 shim（`sg` 包装）执行，**无需注销重登**
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(target_os = "windows", allow(dead_code))]
enum DockerMode {
    Sudo,
    Direct,
    Shim(String),
}

static DOCKER_MODE: std::sync::OnceLock<std::sync::Mutex<Option<DockerMode>>> =
    std::sync::OnceLock::new();

fn mode_cache() -> &'static std::sync::Mutex<Option<DockerMode>> {
    DOCKER_MODE.get_or_init(|| std::sync::Mutex::new(None))
}

/// docker 调用方式（解析一次后缓存；权限修复后需 `invalidate_docker_mode()`）
fn docker_mode() -> DockerMode {
    if let Ok(guard) = mode_cache().lock() {
        if let Some(m) = guard.as_ref() {
            return m.clone();
        }
    }
    let resolved = resolve_docker_mode();
    if let Ok(mut guard) = mode_cache().lock() {
        *guard = Some(resolved.clone());
    }
    resolved
}

/// 预热 docker 调用方式缓存（探测本身是阻塞的：`sudo -n` / `stat` / 最多 3 次 `docker info` / `sg`）。
/// 应在阻塞线程池里调用一次（应用启动后），之后再调用 `docker_cmd()/docker_cmd_tokio()`
/// 就是纯缓存命中，不会阻塞 async 运行时的工作线程。
#[cfg_attr(target_os = "windows", allow(dead_code))]
pub fn warm_docker_mode() {
    let _ = docker_mode();
}

/// 权限被修复后必须失效缓存：否则仍按修复前的路径调用 docker（新加的组身份用不上）
pub fn invalidate_docker_mode() {
    if let Ok(mut guard) = mode_cache().lock() {
        *guard = None;
    }
}

#[cfg(target_os = "windows")]
fn resolve_docker_mode() -> DockerMode {
    DockerMode::Direct
}

/// docker 调用方式解析（按**探测事实**决定，不靠猜）：
/// 1. 免密 sudo 可用 → `sudo -n docker`；
/// 2. `docker info` 直接可访问 → 直接 `docker`；
/// 3. 探测脚本已**实测** `sg <socket 属组> -c docker info` 成功 → 用 sg shim（当前登录会话无需注销）；
/// 4. 其余 → 直接 `docker`（失败后由 UI 引导修复 / 注销）。
///
/// 第 3 条是「无需注销」的全部依据：只有当 sg 跑通真实 docker 才启用 shim，
/// 不做「账号在组就当能用」的推断（那正是历史误判的来源）。
#[cfg(not(target_os = "windows"))]
fn resolve_docker_mode() -> DockerMode {
    use crate::common::utils::docker_perm as dp;
    if sudo_available() {
        return DockerMode::Sudo;
    }
    let Some((perm, state)) = probe_perm_local_sync() else {
        return DockerMode::Direct;
    };
    if state == dp::DockerPermState::Ready {
        return DockerMode::Direct;
    }
    if perm.sg_ok {
        if let Some(p) = ensure_sg_shim(&perm.effective_group()) {
            return DockerMode::Shim(p.to_string_lossy().into_owned());
        }
    }
    if perm.sudo_nopass {
        return DockerMode::Sudo;
    }
    DockerMode::Direct
}

/// 同步版本机权限探测（`docker_perm::PROBE_SCRIPT`）：与启动预检 / 部署 step6 同一份事实来源
#[cfg(not(target_os = "windows"))]
fn probe_perm_local_sync(
) -> Option<(crate::common::utils::docker_perm::DockerPerm, crate::common::utils::docker_perm::DockerPermState)> {
    use crate::common::utils::docker_perm as dp;
    let out = std::process::Command::new("sh")
        .arg("-c")
        .arg(dp::PROBE_SCRIPT)
        .output()
        .ok()?;
    let p = dp::parse_probe(&String::from_utf8_lossy(&out.stdout));
    let s = dp::classify(&p);
    Some((p, s))
}

#[cfg(not(target_os = "windows"))]
fn current_user() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_default()
}

/// sg shim 脚本内容（纯函数，便于单测）：
/// ① 会话已带该组 → 直接 docker；② 已包装过 → 直接 docker（两道护栏，绝不递归）；
/// ③ 否则经 `sg` 取得组身份后重入自身（`sg -c` 固定走 /bin/sh，参数在 bash 侧用 `%q` 转义）
#[cfg_attr(target_os = "windows", allow(dead_code))]
pub(crate) fn sg_shim_body(group: &str) -> String {
    let grp = crate::common::ssh::sh_quote(group);
    format!(
        "#!/bin/bash\n\
# ADM-BE 自动生成（请勿手改）：当前登录会话未带上 docker socket 属组时，\n\
# 借 sg 取得该组身份后执行 docker —— 无需注销重新登录。\n\
# 注：仅在探测脚本实测 `sg {group} -c \"docker info\"` 成功时才会生成本文件。\n\
if id -nG 2>/dev/null | grep -qw {grp}; then\n\
  exec docker \"$@\"\n\
fi\n\
if [ \"${{ADM_BE_SG_WRAPPED:-0}}\" != \"1\" ]; then\n\
  export ADM_BE_SG_WRAPPED=1\n\
  exec sg {grp} -c \"$0$(printf ' %q' \"$@\")\"\n\
fi\n\
exec docker \"$@\"\n",
        group = group,
        grp = grp
    )
}

/// 生成（或复用）sg 组身份 shim：`<data>/com.adm.be/bin/adm-docker`。
/// 参数经 `%q` 转义后交给 sg 内部的 /bin/sh，调用方仍按普通 `docker <args>` 使用。
#[cfg(not(target_os = "windows"))]
fn ensure_sg_shim(group: &str) -> Option<std::path::PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    // shim 用 bash 的 `printf %q` 做参数转义；无 bash 时不做 shim（宁可走失败提示，也不生成坏命令）
    let has_bash = std::process::Command::new("sh")
        .args(["-c", "command -v bash >/dev/null 2>&1"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !has_bash {
        crate::dbg_log!("sg shim: bash 不存在，跳过生成（改走直接 docker）");
        return None;
    }
    let dir = dirs::data_local_dir()?.join("com.adm.be").join("bin");
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join("adm-docker");
    let body = sg_shim_body(group);
    let same = std::fs::read_to_string(&path).map(|s| s == body).unwrap_or(false);
    if !same {
        std::fs::write(&path, body).ok()?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).ok()?;
    }
    Some(path)
}

/// 创建一个 docker 命令（统一入口）：免密 sudo → 会话已带组 → sg shim。
pub fn docker_cmd() -> std::process::Command {
    match docker_mode() {
        DockerMode::Sudo => {
            let mut c = std::process::Command::new("sudo");
            c.arg("-n").arg("docker");
            c
        }
        DockerMode::Direct => std::process::Command::new("docker"),
        DockerMode::Shim(path) => std::process::Command::new(path),
    }
}

/// 创建一个 docker 命令（tokio 版）。
pub fn docker_cmd_tokio() -> tokio::process::Command {
    match docker_mode() {
        DockerMode::Sudo => {
            let mut c = tokio::process::Command::new("sudo");
            c.arg("-n").arg("docker");
            c
        }
        DockerMode::Direct => tokio::process::Command::new("docker"),
        DockerMode::Shim(path) => tokio::process::Command::new(path),
    }
}

/// docker 命令行（shell 形式）：供需要经 shell 包装执行的场景使用（如 `script -c` 给 docker 分配 PTY）。
pub fn docker_shell_prefix() -> &'static str {
    static PREFIX: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    PREFIX.get_or_init(|| match docker_mode() {
        DockerMode::Sudo => "sudo -n docker".to_string(),
        DockerMode::Direct => "docker".to_string(),
        DockerMode::Shim(path) => crate::common::ssh::sh_quote(&path),
    })
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

/// 本机 docker 权限探测（`docker_perm::PROBE_SCRIPT` + 分类）
#[cfg(not(target_os = "windows"))]
pub async fn probe_local_docker_perm(
) -> Option<(crate::common::utils::docker_perm::DockerPerm, crate::common::utils::docker_perm::DockerPermState)> {
    use crate::common::utils::docker_perm as dp;
    let out = tokio::process::Command::new("sh")
        .args(["-c", dp::PROBE_SCRIPT])
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let p = dp::parse_probe(&String::from_utf8_lossy(&out.stdout));
    let state = dp::classify(&p);
    Some((p, state))
}

/// pkexec 修复结果
#[cfg(not(target_os = "windows"))]
#[derive(Debug, Clone, PartialEq)]
pub enum PkexecFix {
    /// 修复脚本执行成功（组/socket 已按 socket 实际属组对齐）
    Ok,
    /// 用户取消/拒绝密码框（pkexec 126/127）
    Cancelled,
    /// 无 pkexec（无桌面 agent / 未安装）→ 调用方可降级为终端命令提示
    Unavailable,
    /// 其他失败（含超时、脚本报错）
    Failed(String),
}

/// 弹系统原生密码框（pkexec）以 root 执行 docker 权限修复脚本：
/// `usermod -aG <socket 实际属组> <user>` + socket 属组/权限对齐。
/// 不重启 docker（重启会杀掉运行中的模型容器），socket 属组变更立即生效。
#[cfg(not(target_os = "windows"))]
pub async fn pkexec_fix_docker_permission(
    user: &str,
    p: &crate::common::utils::docker_perm::DockerPerm,
) -> PkexecFix {
    use crate::common::utils::docker_perm as dp;
    if !dp::valid_user(user) {
        return PkexecFix::Failed(format!("非法用户名：{}", user));
    }
    let script = dp::fix_script(user, p);
    let pkexec = tokio::process::Command::new("pkexec")
        .args(["sh", "-c", &script])
        .kill_on_drop(true)
        .output();
    // kill_on_drop：超时后杀掉挂起的 pkexec，避免孤儿进程
    match tokio::time::timeout(std::time::Duration::from_secs(120), pkexec).await {
        Ok(Ok(o)) if o.status.success() => {
            invalidate_docker_mode();
            PkexecFix::Ok
        }
        Ok(Ok(o)) => {
            let code = o.status.code().unwrap_or(-1);
            let err_raw = String::from_utf8_lossy(&o.stderr).to_string();
            let err = err_raw.to_lowercase();
            let cancelled = code == 126
                || (code == 127
                    && (err.contains("not authorized") || err.contains("incident has been reported")));
            if cancelled {
                PkexecFix::Cancelled
            } else if err.contains("no such file") || err.contains("not found") {
                PkexecFix::Unavailable
            } else {
                PkexecFix::Failed(format!("退出码 {}：{}", code, err_raw.trim()))
            }
        }
        Ok(Err(e)) => {
            crate::dbg_log!("pkexec spawn failed: {}", e);
            PkexecFix::Unavailable
        }
        Err(_) => PkexecFix::Failed("pkexec 超时（120s）".to_string()),
    }
}

/// Linux 启动权限预检 + 自动修复（幂等，由 lib.rs setup() 延迟 3 秒调用）：
/// 1. 应用已能免 sudo 使用 docker（免密 sudo / 会话已带组 / `sg` 补组身份 shim）→ 静默返回，
///    **不再提示注销**——旧实现把「账号在组」当成唯一判据，权限其实可用时也反复要求注销；
/// 2. docker CLI 缺失 / daemon 未运行 / 权限正常 → 无修复意义，静默返回；
/// 3. 仅「账号不在 socket 属组」或「会话在组内仍被拒（socket 属组不一致）」才弹 pkexec 修复；
///    成功后复探一次确认「免注销即可用」，emit "sudo-permission-fixed"
///    （payload 带 group / verified / needLogout，前端按需提示，不再一律要求注销）。
#[cfg(not(target_os = "windows"))]
pub async fn startup_ensure_docker_permission(app: tauri::AppHandle) {
    use crate::common::utils::docker_perm as dp;
    use tauri::Emitter;

    // 1) 应用已能免 sudo 使用 docker → 无需任何干预（也不打扰用户）
    //    模式解析会跑阻塞探测（sh/stat/sudo/docker info/sg），故放阻塞线程池
    let mode = tokio::task::spawn_blocking(docker_mode)
        .await
        .unwrap_or(DockerMode::Direct);
    match mode {
        DockerMode::Sudo | DockerMode::Shim(_) => {
            crate::dbg_log!("startup permission check: docker 可用（sudo 兜底 / sg 组身份），跳过");
            return;
        }
        DockerMode::Direct => {}
    }

    let Some((p, state)) = probe_local_docker_perm().await else {
        crate::dbg_log!("startup permission check: 权限探测失败，跳过");
        return;
    };
    match state {
        // 权限已正常 / 免密 sudo 兜底 → 无需修复
        dp::DockerPermState::Ready | dp::DockerPermState::SudoFallback => return,
        // 会话未带组身份：只有 sg 取组身份 / 免密 sudo 真可用时才算「无需处理」；
        // 两者都不可用则继续往下尝试修复（chgrp/chmod/setfacl 仍可能修好 socket 权限/掩码问题）
        dp::DockerPermState::SessionStale if p.sg_ok || p.sudo_nopass => return,
        dp::DockerPermState::CliMissing => {
            crate::dbg_log!("startup permission check: docker CLI 不存在，跳过");
            return;
        }
        dp::DockerPermState::DaemonDown => {
            crate::dbg_log!("startup permission check: docker daemon 未运行，跳过");
            return;
        }
        _ => {}
    }

    let user = if p.user.trim().is_empty() {
        current_user()
    } else {
        p.user.clone()
    };
    if !dp::valid_user(&user) {
        crate::dbg_log!("startup permission check: 用户名缺失或非法，跳过 pkexec 修复");
        return;
    }

    crate::dbg_log!("startup permission check: state={:?}，弹 pkexec 修复", state);
    match pkexec_fix_docker_permission(&user, &p).await {
        PkexecFix::Ok => {
            // 复核：修复后「本会话即可用」（sg 取组身份 / 免密 sudo）则无需注销
            let (verified, need_logout) = match probe_local_docker_perm().await {
                Some((p2, s2)) => {
                    let ok = dp::can_apply_without_logout(&p2, s2);
                    (ok, !ok)
                }
                None => (false, true),
            };
            let _ = app.emit(
                "sudo-permission-fixed",
                serde_json::json!({
                    "user": user,
                    "group": p.effective_group(),
                    "socket": p.socket_path(),
                    "verified": verified,
                    "needLogout": need_logout,
                }),
            );
        }
        PkexecFix::Cancelled => {}
        PkexecFix::Unavailable => {
            crate::dbg_log!("pkexec 不可用，跳过（设置页「权限修复」可手动重试）");
        }
        PkexecFix::Failed(e) => {
            crate::dbg_log!("startup pkexec 修复失败: {}", e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sg_shim_has_two_recursion_guards() {
        let b = sg_shim_body("docker");
        // ① 会话已带该组 → 直接 docker（sg 生效后重入自身时命中）
        assert!(b.contains("if id -nG 2>/dev/null | grep -qw 'docker'; then"));
        assert!(b.contains("exec docker \"$@\""));
        // ② 已包装标记（sg 会保留调用方环境）→ 直接 docker，任何情况下都不再起 sg
        assert!(b.contains("ADM_BE_SG_WRAPPED"));
        // ③ 真正调用 sg，并把自身路径 + 已转义参数交给 sg 的 /bin/sh
        assert!(b.contains("exec sg 'docker' -c \"$0$(printf ' %q' \"$@\")\""));
        assert!(b.starts_with("#!/bin/bash"));
    }

    #[test]
    fn sg_shim_quotes_group_name() {
        let b = sg_shim_body("do'ck");
        assert!(b.contains("exec sg 'do'\\''ck' -c"));
        assert!(b.contains("grep -qw 'do'\\''ck'"));
    }
}
