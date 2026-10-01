// ===== DGX-Spark 双机直连 · 一键从零部署（A 控制机 / B 对等节点）=====
//
// 流程（7 步，每步幂等：先检测 → 已满足跳过；失败自动回滚）：
//   1. 免密 SSH：A 本机生成 ed25519 → 密码控制 B 生成 ed25519 → 公钥互拷 → 验证 A→B 免密
//   2. 光口探测：A/B 各自 ibdev2netdev，确认两侧插口方向一致，得到 Up 网卡
//   3. 固定 IP：/etc/netplan/40-cx7.yaml（A=192.168.177.11/24，B=192.168.177.12/24）
//               + nmcli managed no + NetworkManager 重启 + systemd-networkd 重启
//               （⚠ 绝不执行 netplan apply——DGX-Spark 已知坑）
//   4. 连通检测：双向 ping + 双向免密 ssh（走光口 IP）；失败回滚 Step3
//   5. Docker：检测 A/B，未装执行官方一键脚本（下载走设置页代理）
//   6. 组权限：当前用户加入 docker 组，免 sudo 执行 docker
//   7. 写回多机互联节点表（rank0=本机 / rank1=B），启用多机模式并保存
//
// 事件：`dgx-zero-deploy`，payload { step: 0-7, phase, detail }
//   phase: start / run / done / error / rollback / rollback_done
// 安全：密码仅内存；所有事件 detail 统一经 redact() 打码；命令经 sh_quote 防注入。

use crate::common::error::AppError;
use crate::common::ssh::sh_quote;
use crate::common::ssh_pw::{redact, ssh_pw_run, ssh_pw_run_stdin};
use crate::common::types::NodeInfo;
use crate::common::utils::docker_perm as dp;
use std::time::Duration;
use tauri::Emitter;

const EVENT: &str = "dgx-zero-deploy";
const IP_A: &str = "192.168.177.11";
const IP_B: &str = "192.168.177.12";
const NETPLAN_FILE: &str = "/etc/netplan/40-cx7.yaml";

/// 部署上下文：A = 本机（控制机），B = 远端（对等节点，用表单地址直连）
struct Ctx {
    app: tauri::AppHandle,
    b_addr: String,
    b_port: u16,
    b_user: String,
    b_pass: String,
    a_user: String,
    a_pass: String,
    /// Step2 产物：A/B 各自 Up 光口网卡名（netplan 用）
    iface_a: String,
    iface_b: String,
    /// 本部署是否真的应用过 netplan（false = 用户原有已配置/未应用，回滚时不动）
    applied_a: std::sync::atomic::AtomicBool,
    applied_b: std::sync::atomic::AtomicBool,
    /// Step6：本次是否真的改了账号组（只有真改了才提示「注销重登」，否则不再打扰用户）
    logout_hint: std::sync::atomic::AtomicBool,
}

impl Ctx {
    fn emit(&self, step: u8, phase: &str, detail: &str) {
        let _ = self.app.emit(EVENT, serde_json::json!({
            "step": step,
            "phase": phase,
            "detail": redact(detail, &[&self.a_pass, &self.b_pass]),
        }));
    }
}

// ===== 执行辅助 =====

/// A 本机执行（非 sudo），返回 (status_success, stdout, stderr)
async fn local_run(cmd: &str, timeout: Duration) -> Result<(bool, String, String), AppError> {
    local_run_stdin(cmd, None, timeout).await
}

/// 同 `local_run`，但可把 `stdin_data` 写入子进程 stdin（如 `sudo -S` 的密码）——
/// 密码因此不进命令行参数（否则本机 `ps` 能看到）。
async fn local_run_stdin(
    cmd: &str,
    stdin_data: Option<&[u8]>,
    timeout: Duration,
) -> Result<(bool, String, String), AppError> {
    let mut child = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| AppError::msg(format!("本机执行失败（sh 不可用？）: {}", e)))?;
    if let Some(data) = stdin_data {
        if let Some(mut si) = child.stdin.take() {
            // 写完即 drop（EOF），sudo -S 才不会被卡住
            let _ = tokio::io::AsyncWriteExt::write_all(&mut si, data).await;
        }
    }
    let out = tokio::time::timeout(timeout, async {
        let mut stdout = String::new();
        let mut stderr = String::new();
        if let Some(mut so) = child.stdout.take() {
            let _ = tokio::io::AsyncReadExt::read_to_string(&mut so, &mut stdout).await;
        }
        if let Some(mut se) = child.stderr.take() {
            let _ = tokio::io::AsyncReadExt::read_to_string(&mut se, &mut stderr).await;
        }
        let status = child.wait().await.map_err(|e| AppError::msg(format!("本机执行等待失败: {}", e)))?;
        Ok::<_, AppError>((status.success(), stdout.trim().to_string(), stderr.trim().to_string()))
    })
    .await;
    match out {
        Ok(r) => r,
        Err(_) => {
            let _ = child.kill();
            Err(AppError::msg(format!("本机执行超时（{}s）", timeout.as_secs())))
        }
    }
}

/// B 远端执行（密码 SSH）
async fn remote_run(c: &Ctx, cmd: &str, timeout: Duration) -> Result<(bool, String, String), AppError> {
    ssh_pw_run(&c.b_addr, c.b_port, &c.b_user, &c.b_pass, cmd, timeout).await
}

/// **以 root 执行脚本（密码走 stdin，不进命令行/进程 argv）**：
/// A 本机经 `sh -c "sudo -S -p '' sh -c <脚本>"` + stdin 写密码；
/// B 远端经 SSH 通道 stdin 送密码（`ssh_pw_run_stdin`）。
async fn run_sudo(
    c: &Ctx,
    is_a: bool,
    script: &str,
    timeout: Duration,
) -> Result<(bool, String, String), AppError> {
    let pass = if is_a { &c.a_pass } else { &c.b_pass };
    let cmd = format!("sudo -S -p '' sh -c {}", sh_quote(script));
    let mut stdin = pass.as_bytes().to_vec();
    stdin.push(b'\n');
    if is_a {
        local_run_stdin(&cmd, Some(&stdin), timeout).await
    } else {
        ssh_pw_run_stdin(
            &c.b_addr,
            c.b_port,
            &c.b_user,
            &c.b_pass,
            &cmd,
            Some(&stdin),
            timeout,
        )
        .await
    }
}

/// 无反馈 sudo 包装：`echo '<pass>' | sudo -S -p '' <cmd>`（密码经 sh_quote 防注入）
/// 当前各步骤直接内联该模式（密码来源不同），本函数保留为等效模板并有单测锁定格式
#[cfg_attr(not(test), allow(dead_code))]
fn sudoize(pass: &str, cmd: &str) -> String {
    format!("echo {} | sudo -S -p '' {}", sh_quote(pass), cmd)
}

/// 远端文件内容传输：命令串内嵌 base64（无特殊字符，安全；Rust 侧 base64 依赖生成）
fn base64_of(s: &str) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(s)
}

/// 步骤包装：统一 start/done/error 事件（宏：规避 async fn 的 HRTB 限制）
macro_rules! run_step {
    ($c:expr, $step:expr, $name:expr, $f:expr) => {{
        $c.emit($step, "start", $name);
        match $f($c).await {
            Ok(v) => {
                $c.emit($step, "done", "");
                Ok::<_, AppError>(v)
            }
            Err(e) => {
                $c.emit($step, "error", &e.to_string());
                Err::<_, AppError>(e)
            }
        }
    }};
}

// ===== 主命令 =====

#[tauri::command]
pub async fn dgx_deploy_run(
    app: tauri::AppHandle,
    b_addr: String,
    b_port: u16,
    b_user: String,
    b_pass: String,
    a_user: String,
    a_pass: String,
) -> Result<String, AppError> {
    if !cfg!(target_os = "linux") {
        return Err(AppError::msg("该功能需在 DGX Spark（A 机，Linux）上运行".to_string()));
    }
    validate_input(&b_addr, &b_user, &a_user, &a_pass, &b_pass, b_port)?;

    let mut c = Ctx {
        app,
        b_addr: b_addr.trim().to_string(),
        b_port: if b_port == 0 { 22 } else { b_port },
        b_user: b_user.trim().to_string(),
        b_pass,
        a_user: a_user.trim().to_string(),
        a_pass,
        iface_a: String::new(),
        iface_b: String::new(),
        applied_a: std::sync::atomic::AtomicBool::new(false),
        applied_b: std::sync::atomic::AtomicBool::new(false),
        logout_hint: std::sync::atomic::AtomicBool::new(false),
    };

    c.emit(0, "start", "DGX-Spark 双机直连一键部署开始");
    run_step!(&c, 1, "免密 SSH 配置", step1_ssh_key)?;
    let (iface_a, iface_b) = run_step!(&c, 2, "光口探测", step2_iface)?;
    c.iface_a = iface_a;
    c.iface_b = iface_b;
    run_step!(&c, 3, "固定 IP 配置", step3_netplan)?;
    run_step!(&c, 4, "连通性检测", step4_ping)?;
    run_step!(&c, 5, "Docker 检测安装", step5_docker)?;
    run_step!(&c, 6, "Docker 组权限", step6_group)?;
    run_step!(&c, 7, "写回多机互联节点表", step7_writeback)?;
    // 只有本次真的改了账号组（且免注销验证未通过）才提示注销：权限已可用时不再反复要求注销
    let logout_hint = c.logout_hint.load(std::sync::atomic::Ordering::Relaxed);
    c.emit(0, "done", if logout_hint { "全部完成|LOGOUT_HINT" } else { "全部完成|READY" });
    Ok(if logout_hint { "DEPLOY_DONE|LOGOUT_HINT" } else { "DEPLOY_DONE|READY" }.to_string())
}

fn validate_input(
    b_addr: &str,
    b_user: &str,
    a_user: &str,
    a_pass: &str,
    b_pass: &str,
    b_port: u16,
) -> Result<(), AppError> {
    let bad = |s: &str| {
        s.chars()
            .any(|ch| ch.is_whitespace() || "@/\\$`;|&()<>\"':".contains(ch))
    };
    let check = |name: &str, v: &str| -> Result<(), AppError> {
        if v.trim().is_empty() {
            return Err(AppError::msg(format!("{}不能为空", name)));
        }
        if bad(v) {
            return Err(AppError::msg(format!(
                "{}包含非法字符（不能有空格/@/引号/冒号等）",
                name
            )));
        }
        Ok(())
    };
    check("B 设备地址", b_addr)?;
    check("B 用户名", b_user)?;
    check("A 用户名", a_user)?;
    if a_pass.is_empty() || b_pass.is_empty() {
        return Err(AppError::msg("A/B 密码不能为空".to_string()));
    }
    if !(1..=65535).contains(&b_port) {
        return Err(AppError::msg("B SSH 端口需在 1-65535".to_string()));
    }
    Ok(())
}

// ===== Step1 免密 SSH =====

async fn step1_ssh_key(c: &Ctx) -> Result<(), AppError> {
    // A 本机：准备 .ssh 目录 + 生成 ed25519（已存在跳过）+ 读公钥
    c.emit(1, "run", "A：检查/生成 ed25519 密钥");
    let (ok, out, err) = local_run(
        "mkdir -p ~/.ssh && chmod 700 ~/.ssh && \
         [ -f ~/.ssh/id_ed25519 ] || ssh-keygen -t ed25519 -N '' -C 'adm-be@dgx-a' -f ~/.ssh/id_ed25519; \
         chmod 600 ~/.ssh/id_ed25519 ~/.ssh/id_ed25519.pub 2>/dev/null; cat ~/.ssh/id_ed25519.pub",
        Duration::from_secs(30),
    )
    .await?;
    if !ok {
        return Err(AppError::msg(format!(
            "A 密钥准备失败（本机未装 openssh-client？）: {}",
            err
        )));
    }
    let pub_a = out.trim().lines().last().unwrap_or("").trim().to_string();
    if pub_a.is_empty() || !pub_a.starts_with("ssh-ed25519") {
        return Err(AppError::msg("读取 A 公钥失败".to_string()));
    }

    // B：准备 .ssh + 生成密钥 + 追加 pubA + 回传 pubB
    c.emit(1, "run", "B：准备密钥并接收 A 公钥");
    let b_setup = format!(
        "mkdir -p ~/.ssh && chmod 700 ~/.ssh && \
         [ -f ~/.ssh/id_ed25519 ] || ssh-keygen -t ed25519 -N '' -C 'adm-be@dgx-b' -f ~/.ssh/id_ed25519; \
         chmod 600 ~/.ssh/id_ed25519 ~/.ssh/id_ed25519.pub 2>/dev/null; \
         touch ~/.ssh/authorized_keys && chmod 600 ~/.ssh/authorized_keys; \
         grep -qF {} ~/.ssh/authorized_keys 2>/dev/null || echo {} >> ~/.ssh/authorized_keys; \
         cat ~/.ssh/id_ed25519.pub",
        sh_quote(&pub_a),
        sh_quote(&pub_a)
    );
    let (ok, out, err) = remote_run(c, &b_setup, Duration::from_secs(60)).await?;
    if !ok {
        return Err(AppError::msg(format!("B 密钥准备失败: {}", err)));
    }
    let pub_b = out.trim().lines().last().unwrap_or("").trim().to_string();
    if pub_b.is_empty() || !pub_b.starts_with("ssh-ed25519") {
        return Err(AppError::msg("读取 B 公钥失败".to_string()));
    }

    // A：备份 authorized_keys 并追加 pubB
    c.emit(1, "run", "A：录入 B 公钥（自动备份 authorized_keys）");
    let (ok, _, err) = local_run(
        &format!(
            "cp ~/.ssh/authorized_keys ~/.ssh/authorized_keys.adm-bak-$(date +%s) 2>/dev/null; \
             touch ~/.ssh/authorized_keys && chmod 600 ~/.ssh/authorized_keys; \
             grep -qF {pb} ~/.ssh/authorized_keys 2>/dev/null || echo {pb} >> ~/.ssh/authorized_keys",
            pb = sh_quote(&pub_b)
        ),
        Duration::from_secs(30),
    )
    .await?;
    if !ok {
        return Err(AppError::msg(format!("A 录入公钥失败: {}", err)));
    }

    // 验证 A→B 免密（用 B 表单地址；accept-new：光口/IP 首次连接不因 host key 校验失败）
    c.emit(1, "run", "验证 A→B 免密 SSH");
    let (ok, out, err) = local_run(
        &format!(
            "ssh -o BatchMode=yes -o StrictHostKeyChecking=accept-new -o ConnectTimeout=5 {}@{} 'echo SSH_OK_AB'",
            sh_quote(&c.b_user),
            sh_quote(&c.b_addr)
        ),
        Duration::from_secs(20),
    )
    .await?;
    if !ok || !out.contains("SSH_OK_AB") {
        c.emit(1, "rollback", "验证失败，回滚公钥互拷");
        let _ = local_run(
            &format!(
                "grep -vF {pb} ~/.ssh/authorized_keys > ~/.ssh/authorized_keys.tmp && mv ~/.ssh/authorized_keys.tmp ~/.ssh/authorized_keys",
                pb = sh_quote(&pub_b)
            ),
            Duration::from_secs(20),
        )
        .await;
        let _ = remote_run(
            c,
            &format!(
                "grep -vF {pa} ~/.ssh/authorized_keys > ~/.ssh/authorized_keys.tmp && mv ~/.ssh/authorized_keys.tmp ~/.ssh/authorized_keys",
                pa = sh_quote(&pub_a)
            ),
            Duration::from_secs(30),
        )
        .await;
        let reason = err.split('\n').next().unwrap_or("").trim();
        let reason = if reason.is_empty() { out } else { reason.to_string() };
        return Err(AppError::msg(format!("A→B 免密验证失败: {}", reason)));
    }
    Ok(())
}

// ===== Step2 光口探测 =====

/// 解析 ibdev2netdev 输出与 fallback（`==> iface (Up)` 行）输出，返回 Up 口 (rocep, iface) 列表
fn parse_up_ifaces(out: &str) -> Vec<(String, String)> {
    let mut res = Vec::new();
    for line in out.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (rocep, right) = match line.find("==>") {
            Some(p) => {
                let left = line[..p].trim();
                let rocep = left.split_whitespace().next().unwrap_or("").to_string();
                (rocep, line[p + 3..].trim().to_string())
            }
            None => (String::new(), line.to_string()),
        };
        // right: "enp1s0f1np1 (Up)" / "enp1s0f1np1 (Down)" / "enp1s0f1np1"
        let (iface, state) = match right.find(" (") {
            Some(p) => (&right[..p], right[p + 2..].trim_end_matches(')').trim().to_string()),
            None => (right.as_str(), String::new()),
        };
        if state == "Up" && !iface.is_empty() {
            res.push((rocep, iface.to_string()));
        }
    }
    res
}

/// 两侧 Up 口集合是否一致（顺序无关）：
/// DGX Spark 插 1 根线时同一物理口会呈现为 2 个 Up（如 rocep1s0f1 / roceP2p1s0f1
/// 镜像），所以不做数量判定，只要求 A、B 两侧的 Up 集合完全相同 = 插口方向一致。
fn same_up_set(a: &[(String, String)], b: &[(String, String)]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut a2 = a.to_vec();
    let mut b2 = b.to_vec();
    a2.sort();
    b2.sort();
    a2 == b2
}

/// 返回 (iface_a, iface_b)
async fn step2_iface(c: &Ctx) -> Result<(String, String), AppError> {
    // 探测命令：优先 ibdev2netdev，缺失时用 sysfs fallback（mlx5_core + operstate=up）
    let probe_cmd = "if command -v ibdev2netdev >/dev/null 2>&1; then ibdev2netdev 2>/dev/null; else \
         for d in /sys/class/net/*; do n=$(basename \"$d\"); drv=$(readlink \"$d/device/driver\" 2>/dev/null); drv=${drv##*/}; \
         st=$(cat \"$d/operstate\" 2>/dev/null); if [ \"$drv\" = mlx5_core ] && [ \"$st\" = up ]; then echo \"==> $n (Up)\"; fi; done; fi";

    c.emit(2, "run", "A：ibdev2netdev 探测光口");
    let (ok, out, err) = local_run(probe_cmd, Duration::from_secs(30)).await?;
    if !ok {
        return Err(AppError::msg(format!("A 光口探测失败: {}", err)));
    }
    let list_a = parse_up_ifaces(&out);

    c.emit(2, "run", "B：ibdev2netdev 探测光口");
    let (ok, out, err) = remote_run(c, probe_cmd, Duration::from_secs(30)).await?;
    if !ok {
        return Err(AppError::msg(format!("B 光口探测失败: {}", err)));
    }
    let list_b = parse_up_ifaces(&out);

    // 探测结果完整列出（便于日志核对插口方向）
    let fmt_list = |l: &[(String, String)]| -> String {
        l.iter()
            .map(|(r, i)| format!("{} → {} (Up)", if r.is_empty() { i.clone() } else { r.clone() }, i))
            .collect::<Vec<_>>()
            .join("；")
    };
    if !list_a.is_empty() {
        c.emit(2, "run", &format!("A 检测到 {} 个 Up 光口：{}", list_a.len(), fmt_list(&list_a)));
    }
    if !list_b.is_empty() {
        c.emit(2, "run", &format!("B 检测到 {} 个 Up 光口：{}", list_b.len(), fmt_list(&list_b)));
    }

    // 判定：任一侧为 0 → 线没插好；两侧集合不一致 → 插口方向不一致
    if list_a.is_empty() {
        return Err(AppError::msg(
            "A 未检测到已插好的光口（Up 数 = 0），请检查直连线是否插紧/被系统识别".to_string(),
        ));
    }
    if list_b.is_empty() {
        return Err(AppError::msg(
            "B 未检测到已插好的光口（Up 数 = 0），请检查直连线是否插紧/被系统识别".to_string(),
        ));
    }
    if list_a.len() != list_b.len() {
        return Err(AppError::msg(format!(
            "A/B 检测结果不一致：A 检测到 {} 个 Up 光口，B 检测到 {} 个。请确认 A/B 都插好直连线，且插同一方向的口（如都插左边）",
            list_a.len(),
            list_b.len()
        )));
    }
    if !same_up_set(&list_a, &list_b) {
        return Err(AppError::msg(format!(
            "插口方向不一致：A 检测到 [{}]，B 检测到 [{}]。请保持 A、B 插同一方向的口（如都插左边）",
            fmt_list(&list_a),
            fmt_list(&list_b)
        )));
    }

    // 两侧 Up 集合一致 → 通过；netplan 只配置首个网卡（与手工流程一致，另一条是镜像/备用）
    let iface_a = list_a[0].1.clone();
    let iface_b = list_b[0].1.clone();
    Ok((iface_a, iface_b))
}

// ===== Step3 固定 IP =====

/// 生成 netplan yaml。addr 接受纯 IP（IP_A/IP_B 常量，如 192.168.177.11）或
/// 已带 /prefixlength 的写法（容错：避免误传已带 prefix 的地址产生 //24）。
fn netplan_yaml(iface: &str, addr: &str) -> String {
    let addr_with_prefix = if addr.contains('/') {
        addr.to_string()
    } else {
        format!("{}/24", addr)
    };
    format!(
        "network:\n  version: 2\n  renderer: networkd\n  ethernets:\n    {}:\n      dhcp4: no\n      addresses: [{}]\n",
        iface, addr_with_prefix
    )
}

/// 单台设备应用 netplan（is_a=true 走本机 sudo，false 走 B 远端 sudo）。
/// 内部任一步失败 → 立即回滚这台设备（删除配置 + 恢复网络服务）并返回 Err。
async fn apply_netplan(c: &Ctx, is_a: bool, iface: &str, addr: &str) -> Result<(), AppError> {
    let who = if is_a { "A" } else { "B" };
    let pass = if is_a { &c.a_pass } else { &c.b_pass };

    // 1) 直接写入新配置（不做备份：40-cx7.yaml 通常不存在或由本流程唯一管理，
    //    回滚 = 删除文件 / 重新生成），base64 传输 → /tmp → sudo cp，
    //    避开管道与 sudo stdin 冲突
    //    显式 chmod 600：netplan / NVIDIA DGX Spark 均要求 0600；cp 覆盖已存在文件时
    //    会保留目标旧权限（首次部署则由 umask 决定），必须手动固化
    c.emit(3, "run", &format!("{}：写入固定 IP {}（{}）", who, addr, iface));
    let write = format!(
        "echo {} | base64 -d > /tmp/adm-netplan-40.yaml && \
         echo '{}' | sudo -S -p '' cp /tmp/adm-netplan-40.yaml {} && \
         echo '{}' | sudo -S -p '' chmod 600 {} && \
         rm -f /tmp/adm-netplan-40.yaml",
        sh_quote(&base64_of(&netplan_yaml(iface, addr))),
        sh_quote(pass),
        NETPLAN_FILE,
        sh_quote(pass),
        NETPLAN_FILE
    );
    let (ok, _, err) = if is_a {
        local_run(&write, Duration::from_secs(30)).await?
    } else {
        remote_run(c, &write, Duration::from_secs(30)).await?
    };
    if !ok {
        restore_one(c, is_a, iface).await;
        return Err(AppError::msg(format!("{} 写入 netplan 失败: {}", who, err)));
    }

    // 2.5) netplan generate：仅生成 systemd-networkd 配置（/run/systemd/network/），不 apply。
    //      ⚠ 关键：systemd-networkd 重启不会自动重读 40-cx7.yaml，必须 generate 后才生效；
    //      generate 只写文件不接管接口，不会触发 DGX-Spark 的 netplan apply 报错坑。
    c.emit(3, "run", &format!("{}：netplan generate 生成 networkd 配置", who));
    let gen = format!(
        "echo '{}' | sudo -S -p '' netplan generate 2>&1",
        sh_quote(pass)
    );
    let (ok, out, err) = if is_a {
        local_run(&gen, Duration::from_secs(30)).await?
    } else {
        remote_run(c, &gen, Duration::from_secs(30)).await?
    };
    if !ok {
        restore_one(c, is_a, iface).await;
        let detail = if out.trim().is_empty() { err } else { out.trim().to_string() };
        return Err(AppError::msg(format!(
            "{} netplan generate 失败（yaml 与现有 netplan 配置冲突？）: {}",
            who, detail
        )));
    }

    // 3) 网卡移交 networkd（nmcli managed no）+ 重启 NetworkManager
    c.emit(3, "run", &format!("{}：nmcli 移交网卡并重启 NetworkManager", who));
    let nm = format!(
        "echo '{}' | sudo -S -p '' nmcli device set {} managed no && \
         echo '{}' | sudo -S -p '' systemctl restart NetworkManager",
        sh_quote(pass),
        sh_quote(iface),
        sh_quote(pass)
    );
    let (ok, _, err) = if is_a {
        local_run(&nm, Duration::from_secs(90)).await?
    } else {
        remote_run(c, &nm, Duration::from_secs(90)).await?
    };
    if !ok {
        restore_one(c, is_a, iface).await;
        return Err(AppError::msg(format!("{} 网卡移交失败: {}", who, err)));
    }

    // 4) daemon-reload + 启用并重启 systemd-networkd + 校验 IP 生效
    c.emit(3, "run", &format!("{}：重启 systemd-networkd 生效", who));
    let netd = format!(
        "echo '{}' | sudo -S -p '' systemctl daemon-reload; \
         echo '{}' | sudo -S -p '' systemctl enable --now systemd-networkd; \
         echo '{}' | sudo -S -p '' systemctl restart systemd-networkd; sleep 3; \
         if ip -4 addr show {} | grep -q \"{}/24\"; then echo NETD_OK; else echo NETD_FAIL; ip -4 addr show {} | head -5; fi",
        sh_quote(pass),
        sh_quote(pass),
        sh_quote(pass),
        sh_quote(iface),
        addr,
        sh_quote(iface)
    );
    let (ok, out, err) = if is_a {
        local_run(&netd, Duration::from_secs(90)).await?
    } else {
        remote_run(c, &netd, Duration::from_secs(90)).await?
    };
    if !ok {
        restore_one(c, is_a, iface).await;
        return Err(AppError::msg(format!("{} 重启 systemd-networkd 失败: {}", who, err)));
    }
    if !out.contains("NETD_OK") {
        restore_one(c, is_a, iface).await;
        return Err(AppError::msg(format!(
            "{} 固定 IP 未生效（{} 上未出现 {}/24）\n{}",
            who, iface, addr, out
        )));
    }
    Ok(())
}

/// 单台设备回滚网络配置（删除本流程写入的 netplan 文件 + nmcli managed yes + 重启 NM/networkd）
async fn restore_one(c: &Ctx, is_a: bool, iface: &str) {
    let pass = if is_a { &c.a_pass } else { &c.b_pass };
    let script = format!(
        "echo '{p}' | sudo -S -p '' rm -f {f}; \
         echo '{p}' | sudo -S -p '' netplan generate || true; \
         echo '{p}' | sudo -S -p '' nmcli device set {ifc} managed yes || true; \
         echo '{p}' | sudo -S -p '' systemctl restart NetworkManager || true; \
         echo '{p}' | sudo -S -p '' systemctl daemon-reload || true; \
         echo '{p}' | sudo -S -p '' systemctl restart systemd-networkd || true",
        f = NETPLAN_FILE,
        p = sh_quote(pass),
        ifc = iface
    );
    let _ = if is_a {
        local_run(&script, Duration::from_secs(90)).await
    } else {
        remote_run(c, &script, Duration::from_secs(90)).await
    };
}

/// 先判断后执行：检测单台设备 netplan 是否已含目标配置。
/// 已配置 → 返回 true（跳过整个应用流程，避免重复重启网络服务造成抖动）。
async fn need_netplan(c: &Ctx, is_a: bool, iface: &str, addr: &str) -> Result<bool, AppError> {
    let who = if is_a { "A" } else { "B" };
    let cmd = format!(
        "if [ -f {f} ] && grep -q \"{ifc}:\" {f} && grep -q \"{addr}/24\" {f}; then echo NETPLAN_READY; else echo NETPLAN_NEED; fi",
        f = NETPLAN_FILE,
        ifc = iface,
        addr = addr
    );
    let (ok, out, err) = if is_a {
        local_run(&cmd, Duration::from_secs(30)).await?
    } else {
        remote_run(c, &cmd, Duration::from_secs(30)).await?
    };
    if !ok {
        return Err(AppError::msg(format!("{} netplan 检测失败: {}", who, err)));
    }
    Ok(out.contains("NETPLAN_READY"))
}

async fn step3_netplan(c: &Ctx) -> Result<(), AppError> {
    c.emit(3, "run", "应用前确认：渲染器 networkd，禁用 netplan apply（DGX-Spark 坑）");
    // 先判断后执行：已配置好的设备跳过（不重启网络服务）
    match need_netplan(c, true, &c.iface_a, IP_A).await? {
        true => c.emit(3, "run", &format!("A：{} 已包含 {}，跳过", NETPLAN_FILE, IP_A)),
        false => {
            if let Err(e) = apply_netplan(c, true, &c.iface_a, IP_A).await {
                c.emit(3, "rollback", "A 固定 IP 配置失败，已回滚（删除 40-cx7.yaml 并恢复网络服务）");
                return Err(e);
            }
            c.applied_a.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
    match need_netplan(c, false, &c.iface_b, IP_B).await? {
        true => c.emit(3, "run", &format!("B：{} 已包含 {}，跳过", NETPLAN_FILE, IP_B)),
        false => {
            if let Err(e) = apply_netplan(c, false, &c.iface_b, IP_B).await {
                c.emit(3, "rollback", "B 固定 IP 配置失败，已回滚 B；同步回滚 A 固定 IP 配置");
                // 仅恢复本部署应用过的 A（跳过=用户原有配置，不动）
                if c.applied_a.load(std::sync::atomic::Ordering::Relaxed) {
                    restore_one(c, true, &c.iface_a).await;
                }
                return Err(e);
            }
            c.applied_b.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
    Ok(())
}

// ===== Step4 连通检测 =====

async fn step4_ping(c: &Ctx) -> Result<(), AppError> {
    c.emit(4, "run", &format!("A：ping B（{}）", IP_B));
    let (ok, _, _) = local_run(&format!("ping -c 3 -W 2 {}", IP_B), Duration::from_secs(20)).await?;
    if !ok {
        rollback_both(c).await;
        return Err(AppError::msg(format!("A 无法 ping 通 B 光口 IP（{}），直连未建立", IP_B)));
    }

    c.emit(4, "run", &format!("B：ping A（{}）", IP_A));
    let (ok, _, _) = remote_run(c, &format!("ping -c 3 -W 2 {}", IP_A), Duration::from_secs(30)).await?;
    if !ok {
        rollback_both(c).await;
        return Err(AppError::msg(format!("B 无法 ping 通 A 光口 IP（{}），直连未建立", IP_A)));
    }

    c.emit(4, "run", "A→B 免密 SSH（走光口）");
    let (ok, out, _) = local_run(
        &format!(
            "ssh -o BatchMode=yes -o StrictHostKeyChecking=accept-new -o ConnectTimeout=5 {}@{} 'echo SSH_OK_AB'",
            sh_quote(&c.b_user),
            IP_B
        ),
        Duration::from_secs(20),
    )
    .await?;
    if !ok || !out.contains("SSH_OK_AB") {
        rollback_both(c).await;
        return Err(AppError::msg("A→B 光口免密 SSH 验证失败".to_string()));
    }

    c.emit(4, "run", "B→A 免密 SSH（走光口）");
    let (ok, out, _) = remote_run(
        c,
        &format!(
            "ssh -o BatchMode=yes -o StrictHostKeyChecking=accept-new -o ConnectTimeout=5 {}@{} 'echo SSH_OK_BA'",
            sh_quote(&c.a_user),
            IP_A
        ),
        Duration::from_secs(30),
    )
    .await?;
    if !ok || !out.contains("SSH_OK_BA") {
        rollback_both(c).await;
        return Err(AppError::msg("B→A 光口免密 SSH 验证失败".to_string()));
    }
    c.emit(4, "run", "双向直连正常（双向 ping + 双向免密 SSH）");
    Ok(())
}

async fn rollback_both(c: &Ctx) {
    c.emit(4, "rollback", "直连校验失败，回滚 A/B 固定 IP 配置（删除配置文件并恢复网络服务）");
    // 仅回滚本部署应用过的设备（跳过=用户原有配置，回滚会破坏原状）
    if c.applied_a.load(std::sync::atomic::Ordering::Relaxed) {
        restore_one(c, true, &c.iface_a).await;
    }
    if c.applied_b.load(std::sync::atomic::Ordering::Relaxed) {
        restore_one(c, false, &c.iface_b).await;
    }
    c.emit(4, "rollback_done", "");
}

// ===== Step5 Docker 检测/安装 =====

/// 检测单台设备 docker：
/// Ok(Some(version)) 已可用 / Ok(None) 未安装（二进制缺失）
/// 已安装但 daemon 异常/密码错误 → 尝试 `systemctl enable --now docker` 后复查；
/// 仍异常 → 返回 Err（明确提示，避免误走安装流程）。
async fn detect_docker(c: &Ctx, is_a: bool) -> Result<Option<String>, AppError> {
    let who = if is_a { "A" } else { "B" };
    let pass = if is_a { &c.a_pass } else { &c.b_pass };
    let pass_q = sh_quote(pass);
    let info = format!("echo '{}' | sudo -S -p '' docker info --format '{{{{.ServerVersion}}}}' 2>/dev/null", pass_q);
    let cmd = format!(
        "if command -v docker >/dev/null 2>&1; then \
         v=$({info}); \
         if [ -z \"$v\" ]; then \
           echo '{p}' | sudo -S -p '' systemctl enable --now docker >/dev/null 2>&1 || true; \
           sleep 2; \
           v=$({info}); \
         fi; \
         if [ -n \"$v\" ]; then echo \"$v\"; else echo DOCKER_ERR; fi; \
         else echo DOCKER_MISSING; fi",
        info = info,
        p = pass_q
    );
    let (ok, out, err) = if is_a {
        local_run(&cmd, Duration::from_secs(60)).await?
    } else {
        remote_run(c, &cmd, Duration::from_secs(60)).await?
    };
    if !ok {
        return Err(AppError::msg(format!("{} docker 检测失败: {}", who, err)));
    }
    let v = out.trim();
    if v == "DOCKER_MISSING" || v.is_empty() {
        Ok(None)
    } else if v == "DOCKER_ERR" {
        Err(AppError::msg(format!(
            "{} 已安装 Docker 但 daemon 无法启动（sudo 密码错误或 docker 服务异常），请检查后重试",
            who
        )))
    } else {
        Ok(Some(v.to_string()))
    }
}

/// 安装 docker（官方一键脚本；代理仅作用于脚本下载）
async fn install_docker(c: &Ctx, is_a: bool) -> Result<(), AppError> {
    let who = if is_a { "A" } else { "B" };
    let pass = if is_a { &c.a_pass } else { &c.b_pass };
    let proxy = crate::pages::settings::load_settings(c.app.clone())
        .await
        .map(|s| s.proxy_url.trim().to_string())
        .unwrap_or_default();
    let proxy_opt = if proxy.is_empty() {
        String::new()
    } else {
        format!("-x {}", sh_quote(&proxy))
    };
    c.emit(5, "run", &format!("{}：未检测到可用 Docker，执行官方一键安装", who));
    let cmd = format!(
        "curl -fsSL {px} https://get.docker.com -o /tmp/get-docker.sh && \
         echo '{p}' | sudo -S -p '' sh /tmp/get-docker.sh && \
         echo '{p}' | sudo -S -p '' systemctl enable --now docker && \
         echo '{p}' | sudo -S -p '' docker info --format '{{{{.ServerVersion}}}}'",
        px = proxy_opt,
        p = sh_quote(pass)
    );
    let (ok, out, err) = if is_a {
        local_run(&cmd, Duration::from_secs(600)).await?
    } else {
        remote_run(c, &cmd, Duration::from_secs(600)).await?
    };
    if !ok {
        return Err(AppError::msg(format!(
            "{} docker 安装失败（可能半装状态）: {}",
            who,
            err.lines().last().unwrap_or("")
        )));
    }
    let v = out.trim();
    if v.is_empty() {
        return Err(AppError::msg(format!("{} docker 安装后校验失败", who)));
    }
    c.emit(5, "run", &format!("{}：Docker {} 安装完成", who, v));
    Ok(())
}

async fn step5_docker(c: &Ctx) -> Result<(), AppError> {
    for (is_a, who) in [(true, "A"), (false, "B")] {
        match detect_docker(c, is_a).await? {
            Some(v) => {
                c.emit(5, "run", &format!("{}：Docker 已可用（ServerVersion {}），跳过", who, v));
            }
            None => {
                if let Err(e) = install_docker(c, is_a).await {
                    // 半装不可干净回滚：仅提示（回滚矩阵的有损例外）
                    c.emit(5, "rollback", "Docker 安装失败无法自动回滚（可能半装），请修复后重新执行");
                    return Err(e);
                }
            }
        }
    }
    Ok(())
}

// ===== Step6 docker 组 =====

/// 探测单台设备（A 本机 / B 远端）的 docker 权限事实（本机与远端复用同一探测脚本）
async fn probe_perm(c: &Ctx, is_a: bool, who: &str) -> Result<dp::DockerPerm, AppError> {
    let (ok, out, err) = if is_a {
        local_run(dp::PROBE_SCRIPT, Duration::from_secs(40)).await?
    } else {
        remote_run(c, dp::PROBE_SCRIPT, Duration::from_secs(60)).await?
    };
    if !ok || !out.contains("PROBE_END") {
        return Err(AppError::msg(format!(
            "{} docker 权限探测失败: {}{}",
            who,
            out,
            err
        )));
    }
    Ok(dp::parse_probe(&out))
}

/// 设备上执行一条命令（A 本机 / B 远端）
async fn run_on(c: &Ctx, is_a: bool, cmd: &str, timeout: Duration) -> Result<(bool, String, String), AppError> {
    if is_a {
        local_run(cmd, timeout).await
    } else {
        remote_run(c, cmd, timeout).await
    }
}

/// Step6：按 **docker socket 实际属组** 对齐权限（幂等）。
///
/// 只有真的改了账号组、且「免注销机制不可用」时才置 `logout_hint`——
/// 历史实现无条件提示注销，导致权限早已可用（或本来就正常）时每次部署都要求注销。
async fn step6_group(c: &Ctx) -> Result<(), AppError> {
    let mut logout_hint = false;
    for (is_a, who, user) in [
        (true, "A", c.a_user.clone()),
        (false, "B", c.b_user.clone()),
    ] {
        let p = probe_perm(c, is_a, &who).await?;
        let state = dp::classify(&p);
        c.emit(6, "run", &format!("{}：{}", who, dp::describe(&p, state)));

        match state {
            // 已就绪 / 免密 sudo 兜底 → 无需任何改动
            dp::DockerPermState::Ready | dp::DockerPermState::SudoFallback => continue,
            // 会话未带组身份：仅当 sg 取组身份 / 免密 sudo 真可用时才算「无需注销」
            dp::DockerPermState::SessionStale if p.sg_ok || p.sudo_nopass => continue,
            dp::DockerPermState::CliMissing => {
                return Err(AppError::msg(format!(
                    "{} 未检测到 docker（Step5 应先完成安装）",
                    who
                )));
            }
            dp::DockerPermState::DaemonDown => {
                return Err(AppError::msg(format!(
                    "{} docker daemon 未运行（权限修复无意义），请先修复 daemon 后重试",
                    who
                )));
            }
            // SessionStale（sg/sudo 都不可用）/ NotInGroup / SocketMismatch / Unknown
            // → 走修复 + 复验
            _ => {}
        }

        let already_in_group = p.account_in_group();
        c.emit(
            6,
            "run",
            &format!(
                "{}：按 socket 属组（{}）对齐 {} 的 docker 权限（usermod + socket 属组/权限，不重启 docker）",
                who,
                p.effective_group(),
                user
            ),
        );
        let script = dp::fix_script(&user, &p);
        // 密码经 stdin 送入（不进命令行/进程 argv）
        let (ok, out, err) = run_sudo(c, is_a, &script, Duration::from_secs(60)).await?;
        if !ok || !out.contains("GROUP_OK") {
            let detail = if err.trim().is_empty() { out.clone() } else { err.clone() };
            return Err(AppError::msg(format!("{} docker 权限修复失败: {}", who, detail)));
        }
        // A 为本机：权限变了要作废 docker 调用方式缓存，否则仍按修复前的旧路径调用
        if is_a {
            crate::common::utils::platform::invalidate_docker_mode();
        }

        // 复验：以 socket 属组身份跑一次 `docker info`（成功 = 无需注销即可用）
        let (vok, vout, _) = run_on(c, is_a, &dp::verify_script(&p), Duration::from_secs(40)).await?;
        if vok && vout.contains("VERIFY_OK") {
            c.emit(
                6,
                "run",
                &format!("{}：权限已对齐并验证可免 sudo 使用 —— 当前会话无需注销", who),
            );
            if !already_in_group {
                c.emit(
                    6,
                    "run",
                    &format!(
                        "{}：本次新增了 {} 组成员关系（之后的登录会话/终端会自动带上该组）",
                        who,
                        p.effective_group()
                    ),
                );
            }
        } else {
            c.emit(
                6,
                "run",
                &format!(
                    "{}：已按 socket 属组（{}）对齐，但本会话免 sudo 验证未通过（需注销重登后生效；如仍失败请检查 {} 的属组/权限）",
                    who,
                    p.effective_group(),
                    p.socket_path()
                ),
            );
            // 只有复验未通过才提示注销：权限已可用时不再打扰用户
            logout_hint = true;
        }
    }

    if logout_hint {
        c.emit(
            6,
            "run",
            "提示：本会话尚不能免 sudo 使用 docker，需注销重新登录后重启 ADM-BE 生效。",
        );
    } else {
        c.emit(6, "run", "docker 权限已就绪（当前会话即可免 sudo，无需注销）");
    }
    c.logout_hint
        .store(logout_hint, std::sync::atomic::Ordering::Relaxed);
    Ok(())
}

// ===== Step7 写回多机互联节点表 =====

async fn step7_writeback(c: &Ctx) -> Result<(), AppError> {
    c.emit(7, "run", "写回多机互联节点表（rank0 本机 / rank1 B，启用多机模式）");
    let mut settings = crate::pages::settings::load_settings(c.app.clone()).await?;
    settings.multi_node_args.enabled = true;
    settings.multi_node_args.nodes = vec![
        NodeInfo {
            ip: IP_A.to_string(),
            ssh_user: c.a_user.clone(),
            ssh_port: 22,
            is_self: true,
            model_dir: String::new(),
        },
        NodeInfo {
            ip: IP_B.to_string(),
            ssh_user: c.b_user.clone(),
            ssh_port: c.b_port,
            is_self: false,
            model_dir: String::new(),
        },
    ];
    crate::pages::settings::save_settings(c.app.clone(), settings).await?;
    Ok(())
}

// ===== 单测 =====

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ibdev_common() {
        let out = "rocep1s0f0 port 1 ==> enp1s0f0np0 (Down)\nrocep1s0f1 port 1 ==> enp1s0f1np1 (Up)\nroceP2p1s0f0 port 1 ==> enP2p1s0f0np0 (Down)\nroceP2p1s0f1 port 1 ==> enP2p1s0f1np1 (Up)";
        let list = parse_up_ifaces(out);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0], ("rocep1s0f1".to_string(), "enp1s0f1np1".to_string()));
        assert_eq!(list[1], ("roceP2p1s0f1".to_string(), "enP2p1s0f1np1".to_string()));
    }

    #[test]
    fn parse_ibdev_no_up() {
        assert!(parse_up_ifaces("rocep1s0f0 port 1 ==> enp1s0f0np0 (Down)").is_empty());
        assert!(parse_up_ifaces("").is_empty());
    }

    #[test]
    fn parse_ibdev_fallback_style() {
        let list = parse_up_ifaces("==> enp1s0f1np1 (Up)\n==> enp1s0f0np0 (Down)");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0], (String::new(), "enp1s0f1np1".to_string()));
    }

    #[test]
    fn parse_ibdev_dual_up_single_cable() {
        // 插 1 根线：同一物理口呈现 2 个 Up（用户实测样例），不应误判为插了 2 根
        let out = "rocep1s0f0 port 1 ==> enp1s0f0np0 (Down)\n\
                   rocep1s0f1 port 1 ==> enp1s0f1np1 (Up)\n\
                   roceP2p1s0f0 port 1 ==> enP2p1s0f0np0 (Down)\n\
                   roceP2p1s0f1 port 1 ==> enP2p1s0f1np1 (Up)";
        let list = parse_up_ifaces(out);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0], ("rocep1s0f1".to_string(), "enp1s0f1np1".to_string()));
        assert_eq!(list[1], ("roceP2p1s0f1".to_string(), "enP2p1s0f1np1".to_string()));
        // 两侧同方向插线 → 集合一致（顺序无关）
        assert!(same_up_set(&list, &list));
    }

    #[test]
    fn same_up_set_order_insensitive() {
        let a = vec![
            ("rocep1s0f1".to_string(), "enp1s0f1np1".to_string()),
            ("roceP2p1s0f1".to_string(), "enP2p1s0f1np1".to_string()),
        ];
        let b = vec![
            ("roceP2p1s0f1".to_string(), "enP2p1s0f1np1".to_string()),
            ("rocep1s0f1".to_string(), "enp1s0f1np1".to_string()),
        ];
        assert!(same_up_set(&a, &b));
        // 方向不一致（A 插 f1、B 插 f0）→ 集合不同
        let b_f0 = vec![
            ("rocep1s0f0".to_string(), "enp1s0f0np0".to_string()),
            ("roceP2p1s0f0".to_string(), "enP2p1s0f0np0".to_string()),
        ];
        assert!(!same_up_set(&a, &b_f0));
        // 数量不同 → 不一致
        assert!(!same_up_set(&a, &b[..1].to_vec()));
        // 空集合
        assert!(same_up_set(&[], &[]));
        assert!(!same_up_set(&a, &[]));
    }

    #[test]
    fn netplan_yaml_shape() {
        // 传入纯 IP（与 IP_A/IP_B 常量一致），生成必须带 /24 prefix
        let y = netplan_yaml("enp1s0f1np1", "192.168.177.11");
        assert!(y.contains("    enp1s0f1np1:"));
        assert!(y.contains("addresses: [192.168.177.11/24]"));
        assert!(y.contains("dhcp4: no"));
        assert!(!y.contains("apply"));
    }

    #[test]
    fn netplan_yaml_idempotent_on_already_prefixed() {
        // 已带 prefix 不应产生 //24
        let y = netplan_yaml("enp1s0f1np1", "192.168.177.11/24");
        assert!(y.contains("addresses: [192.168.177.11/24]"));
        assert!(!y.contains("//24"));
    }

    #[test]
    fn redact_secrets() {
        let s = "password 是 'abc123' 与 \"abc123\" 与 abc123";
        let r = redact(s, &["abc123"]);
        assert!(!r.contains("abc123"));
        assert!(r.contains("******"));
    }

    #[test]
    fn sudoize_wraps() {
        let s = sudoize("p'a ss", "ls");
        // 密码单引号被转义，sudo -S -p '' 在管道末端
        assert!(s.starts_with("echo 'p'\\''a ss' | sudo -S -p '' ls"));
    }
}