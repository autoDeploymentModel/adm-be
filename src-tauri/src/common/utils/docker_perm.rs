//! docker 权限统一探测 / 修复（本机 `sh -c` 与远端 SSH 复用同一脚本）。
//!
//! 背景：docker 的 socket（默认 /var/run/docker.sock）属 `root:docker` 0660，只有
//! 「docker 组成员」且「该组身份已进入当前登录会话」的进程才能免 sudo 访问。历史实现
//! 只判断「账号是否在 docker 组」，并把「本来就在组里」和「本次刚加入组」当成同一件事，
//! 于是权限其实已经可用时仍反复提示注销；而真正让「注销也无效」的两类场景又识别不出来：
//!   1. socket 的属组与账号被加入的组不是同一个（同名不同 GID、docker 重装过、socket 被
//!      重建时属组变了）——注销一万次也没用，必须按 socket 实际属组修正；
//!   2. 账号已在组内、但当前登录会话没带上组身份（应用不是从该会话启动、desktop 只锁屏
//!      未真正注销、systemd 用户服务 linger 等）——用 `sg <组>` 即可当场取得组身份，也无需注销。
//!
//! 本模块负责采集事实（`PROBE_SCRIPT` + `parse_probe`）、给出确定性结论（`classify`）、
//! 生成修复脚本（`fix_script`，经 `echo pass | sudo -S sh -c ...` 以 root 执行）与
//! 面向用户的一句话说明（`describe`）。调用方（启动预检 / DGX 一键部署 / 设置页权限修复 /
//! 多机远端探活）只按结论决定「是否提示注销」，不再自行拼判断。

// 本体面向 Linux（Windows 构建里部分仅 Linux 可达的判断函数用不上）
#![cfg_attr(target_os = "windows", allow(dead_code))]

use crate::common::ssh::sh_quote;

/// 探测脚本（POSIX sh，无 bashism）：本机 `sh -c` 与远端 SSH 执行同一份，输出 KEY=VALUE 行。
///
/// - `ACCOUNT_GROUPS`：`id -nG <user>`，账号数据库视角（重新登录后会带上）
/// - `SESSION_GROUPS`：`id -nG`，当前进程会话视角（决定本进程能否免 sudo）
/// - `SUDO_NOPASS`：`sudo -n true` 是否可用（可用则 docker 命令走 `sudo -n docker` 兜底）
/// - `SG_OK`：`sg <socket 属组> -c docker info` 是否免密成功（= 无需注销即可取得组身份）
pub const PROBE_SCRIPT: &str = r#"u=$(id -un 2>/dev/null || echo '')
echo "USER=$u"
if command -v docker >/dev/null 2>&1; then echo "CLI=1"; else echo "CLI=0"; fi
sock=''
case "${DOCKER_HOST:-}" in
  unix://*) sock="${DOCKER_HOST#unix://}" ;;
esac
if [ -z "$sock" ]; then
  for p in /var/run/docker.sock /run/docker.sock; do
    if [ -S "$p" ]; then sock="$p"; break; fi
  done
fi
echo "SOCKET=$sock"
grp=''
if [ -n "$sock" ]; then
  grp=$(stat -c %G "$sock" 2>/dev/null || echo '')
  echo "SOCK_GROUP=$grp"
  echo "SOCK_GID=$(stat -c %g "$sock" 2>/dev/null || echo '')"
fi
if [ -n "$u" ]; then echo "ACCOUNT_GROUPS=$(id -nG "$u" 2>/dev/null | tr ' ' ',')"; fi
echo "SESSION_GROUPS=$(id -nG 2>/dev/null | tr ' ' ',')"
if sudo -n true >/dev/null 2>&1; then echo "SUDO_NOPASS=1"; else echo "SUDO_NOPASS=0"; fi
if command -v docker >/dev/null 2>&1; then
  if docker info >/dev/null 2>&1; then
    echo "DAEMON=ok"
  elif docker info 2>&1 | grep -qi "permission denied"; then
    echo "DAEMON=perm"
  else
    echo "DAEMON=down"
  fi
fi
if [ -n "$grp" ]; then
  if sg "$grp" -c "docker info >/dev/null 2>&1" >/dev/null 2>&1; then echo "SG_OK=1"; else echo "SG_OK=0"; fi
fi
echo "PROBE_END""#;

/// docker 权限事实（探测结果）
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DockerPerm {
    pub user: String,
    pub cli: bool,
    pub socket: String,
    /// socket 实际属组名（免 sudo 访问所需的组就是它，而不是写死的 "docker"）
    pub socket_group: String,
    pub socket_gid: String,
    /// 账号数据库视角（重新登录后当前进程会带上）
    pub account_groups: Vec<String>,
    /// 当前会话视角
    pub session_groups: Vec<String>,
    pub sudo_nopass: bool,
    /// ok / perm / down（perm = socket 权限拒绝）
    pub daemon: String,
    /// `sg <socket 属组> -c docker info` 是否免密成功（无需注销即可取得组身份）
    pub sg_ok: bool,
}

/// 权限结论（调用方据此决定是否提示注销 / 是否需要修复）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockerPermState {
    /// 当前会话即可免 sudo 使用 docker
    Ready,
    /// 当前会话不可用，但免密 sudo 可用（应用走 `sudo -n docker`，无需任何修复）
    SudoFallback,
    /// 账号已在 socket 属组，当前会话未带上组身份（`sg` 可当场生效，无需注销）
    SessionStale,
    /// 账号不在 socket 属组（需 usermod；修完即可 `sg` 生效，注销仅对终端便利）
    NotInGroup,
    /// 会话已在组内仍被拒：socket 属组/权限与账号所在组不一致（注销无效，需修 socket）
    SocketMismatch,
    /// docker CLI 不存在
    CliMissing,
    /// docker daemon 未运行
    DaemonDown,
    /// 其他未知状态
    Unknown,
}

impl DockerPerm {
    /// 当前进程会话是否已具备 socket 属组身份
    pub fn session_in_group(&self) -> bool {
        let g = self.effective_group();
        !g.is_empty() && self.session_groups.iter().any(|x| x == &g)
    }

    /// 账号数据库里是否已在 socket 属组
    pub fn account_in_group(&self) -> bool {
        let g = self.effective_group();
        !g.is_empty() && self.account_groups.iter().any(|x| x == &g)
    }

    /// 需要免 sudo 访问时应对齐的组：socket 实际属组；socket 缺失时退回 "docker"
    pub fn effective_group(&self) -> String {
        if self.socket_group.trim().is_empty() {
            "docker".to_string()
        } else {
            self.socket_group.trim().to_string()
        }
    }

    /// socket 属组与账号所在组不一致（同名不同 GID 等）时的提示比较基准
    pub fn socket_path(&self) -> String {
        if self.socket.trim().is_empty() {
            "/var/run/docker.sock".to_string()
        } else {
            self.socket.trim().to_string()
        }
    }
}

/// 解析探测脚本输出
pub fn parse_probe(out: &str) -> DockerPerm {
    let mut p = DockerPerm::default();
    for line in out.lines() {
        let line = line.trim();
        let Some((k, v)) = line.split_once('=') else { continue };
        let v = v.trim();
        match k {
            "USER" => p.user = v.to_string(),
            "CLI" => p.cli = v == "1",
            "SOCKET" => p.socket = v.to_string(),
            "SOCK_GROUP" => p.socket_group = v.to_string(),
            "SOCK_GID" => p.socket_gid = v.to_string(),
            "ACCOUNT_GROUPS" => p.account_groups = split_groups(v),
            "SESSION_GROUPS" => p.session_groups = split_groups(v),
            "SUDO_NOPASS" => p.sudo_nopass = v == "1",
            "DAEMON" => p.daemon = v.to_string(),
            "SG_OK" => p.sg_ok = v == "1",
            _ => {}
        }
    }
    p
}

fn split_groups(v: &str) -> Vec<String> {
    v.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// 由探测事实给出结论（顺序即优先级：CLI → daemon 可达 → 权限细节）
pub fn classify(p: &DockerPerm) -> DockerPermState {
    if !p.cli {
        return DockerPermState::CliMissing;
    }
    match p.daemon.as_str() {
        "ok" => DockerPermState::Ready,
        "down" => DockerPermState::DaemonDown,
        "perm" => {
            if p.session_in_group() {
                // 已在组内仍被拒 → 不是「会话没刷新」，注销无效
                DockerPermState::SocketMismatch
            } else if p.account_in_group() {
                DockerPermState::SessionStale
            } else if p.sudo_nopass {
                DockerPermState::SudoFallback
            } else {
                DockerPermState::NotInGroup
            }
        }
        // 探测不到 daemon 状态（如 CLI 存在但 info 因其他原因失败）：免密 sudo 仍可兜底
        _ if p.sudo_nopass => DockerPermState::SudoFallback,
        _ => DockerPermState::Unknown,
    }
}

/// 是否需要 root 修复（usermod / socket 属组）
pub fn needs_fix(state: DockerPermState) -> bool {
    matches!(state, DockerPermState::NotInGroup | DockerPermState::SocketMismatch | DockerPermState::Unknown)
}

/// 修复后是否「本该无需注销」：必须存在真实的免注销机制。
///
/// - `Ready`：当前会话直接可用；
/// - `SudoFallback`：应用走 `sudo -n docker`；
/// - 其余（含 `SessionStale`）：只有当 `sg <socket 属组>` 能取到组身份、或免密 sudo 可兜底时才为真——
///   仅凭「账号已在组」不足以判定可用（`sg` 缺失 / 组密码且用户 passwd 为空时 sg 会在非交互下失败）。
pub fn can_apply_without_logout(p: &DockerPerm, state: DockerPermState) -> bool {
    match state {
        DockerPermState::Ready => true,
        DockerPermState::SudoFallback
        | DockerPermState::SessionStale
        | DockerPermState::NotInGroup
        | DockerPermState::SocketMismatch
        | DockerPermState::Unknown => p.sg_ok || p.sudo_nopass,
        DockerPermState::CliMissing | DockerPermState::DaemonDown => false,
    }
}

/// 生成 root 修复脚本（不含密码；调用方以 `echo <pass> | sudo -S -p '' sh -c <脚本>` 执行）。
///
/// - 目标组 = socket 实际属组（缺失时退回 `docker`）；组不存在则按 socket GID 建组/解析同名 GID 组
/// - `usermod -aG <目标组> <用户>`（幂等）
/// - socket 属组按 **账号解析出的数值 GID** 对齐并固化 0660（覆盖「同名组重复条目 / GID 不一致」
///   这一「注销也不生效」的场景）；setfacl 可用时补一条用户 rw ACL 兜底
/// - 不重启 docker：重启会杀掉运行中的模型容器，socket 属组变更本身立即生效
pub fn fix_script(user: &str, p: &DockerPerm) -> String {
    let grp = p.effective_group();
    let sock = p.socket_path();
    let gid = p.socket_gid.trim().to_string();
    format!(
        "set -e\n\
u={user}\ngrp={grp}\nsock={sock}\ngid={gid}\n\
if ! getent group \"$grp\" >/dev/null 2>&1; then\n\
  if [ -n \"$gid\" ]; then\n\
    if getent group \"$gid\" >/dev/null 2>&1; then grp=$(getent group \"$gid\" | cut -d: -f1); else groupadd -g \"$gid\" \"$grp\" >/dev/null 2>&1 || grp=docker; fi\n\
  fi\n\
fi\n\
[ -n \"$grp\" ] || grp=docker\n\
usermod -aG \"$grp\" \"$u\"\n\
if [ -S \"$sock\" ]; then\n\
  want=$(getent group \"$grp\" 2>/dev/null | head -1 | cut -d: -f3)\n\
  if [ -n \"$want\" ]; then chgrp \"$want\" \"$sock\" 2>/dev/null || true; fi\n\
  chmod 660 \"$sock\" 2>/dev/null || true\n\
  if command -v setfacl >/dev/null 2>&1; then setfacl -m \"u:$u:rw\" \"$sock\" >/dev/null 2>&1 || true; fi\n\
fi\n\
echo GROUP_OK\n",
        user = sh_quote(user),
        grp = sh_quote(&grp),
        sock = sh_quote(&sock),
        gid = sh_quote(&gid),
    )
}

/// 修复后的免注销验证命令：以 socket 属组身份跑一次 docker info
pub fn verify_script(p: &DockerPerm) -> String {
    let grp = p.effective_group();
    format!(
        "if sg {grp} -c \"docker info >/dev/null 2>&1\" >/dev/null 2>&1; then echo VERIFY_OK; else echo VERIFY_FAIL; fi",
        grp = sh_quote(&grp)
    )
}

/// 用户名白名单（进 root 脚本前必须校验，防注入）
pub fn valid_user(user: &str) -> bool {
    let u = user.trim();
    !u.is_empty() && u.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
}

/// 面向用户的一句话说明（部署日志 / 设置页 / 探活提示复用）
pub fn describe(p: &DockerPerm, state: DockerPermState) -> String {
    let grp = p.effective_group();
    let sock = p.socket_path();
    match state {
        DockerPermState::Ready => format!("docker 权限正常（会话已在 {} 组，免 sudo 可用）", grp),
        DockerPermState::SudoFallback => "免密 sudo 可用，本应用以 sudo 执行 docker（无需加组）".to_string(),
        DockerPermState::SessionStale => format!(
            "账号已在 {} 组，但当前登录会话未带上该组身份：ADM-BE 会自动以组身份调用 docker（无需注销）；如需在终端里也免 sudo，可注销重新登录",
            grp
        ),
        DockerPermState::NotInGroup => {
            format!("账号不在 docker socket 的属组（{}）内，需加入该组", grp)
        }
        DockerPermState::SocketMismatch => format!(
            "当前会话已在 {} 组内仍被拒绝：{} 的属组/权限与账号所在组不一致（注销不会修复，已按实际属组对齐）",
            grp, sock
        ),
        DockerPermState::CliMissing => "未检测到 docker CLI".to_string(),
        DockerPermState::DaemonDown => "docker daemon 未运行或不可访问".to_string(),
        DockerPermState::Unknown => format!("docker 权限状态未知（socket={}，属组={}）", sock, grp),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(extra: &[(&str, &str)]) -> DockerPerm {
        let mut lines = vec![
            "USER=user".to_string(),
            "CLI=1".to_string(),
            "SOCKET=/var/run/docker.sock".to_string(),
            "SOCK_GROUP=docker".to_string(),
            "SOCK_GID=998".to_string(),
            "ACCOUNT_GROUPS=user,sudo".to_string(),
            "SESSION_GROUPS=user,sudo".to_string(),
            "SUDO_NOPASS=0".to_string(),
            "DAEMON=perm".to_string(),
            "SG_OK=0".to_string(),
            "PROBE_END".to_string(),
        ];
        for (k, v) in extra {
            lines.push(format!("{}={}", k, v));
        }
        parse_probe(&lines.join("\n"))
    }

    #[test]
    fn parse_basic() {
        let p = probe(&[]);
        assert!(p.cli);
        assert_eq!(p.socket_group, "docker");
        assert_eq!(p.account_groups, vec!["user", "sudo"]);
        assert_eq!(p.daemon, "perm");
    }

    #[test]
    fn classify_missing_cli() {
        let p = probe(&[("CLI", "0")]);
        assert_eq!(classify(&p), DockerPermState::CliMissing);
    }

    #[test]
    fn classify_ready_and_daemon_down() {
        assert_eq!(classify(&probe(&[("DAEMON", "ok")])), DockerPermState::Ready);
        assert_eq!(classify(&probe(&[("DAEMON", "down")])), DockerPermState::DaemonDown);
    }

    #[test]
    fn classify_not_in_group_then_sudo_fallback() {
        assert_eq!(classify(&probe(&[])), DockerPermState::NotInGroup);
        assert_eq!(
            classify(&probe(&[("SUDO_NOPASS", "1")])),
            DockerPermState::SudoFallback
        );
    }

    #[test]
    fn classify_session_stale_when_account_has_group() {
        let p = probe(&[("ACCOUNT_GROUPS", "user,sudo,docker")]);
        assert_eq!(classify(&p), DockerPermState::SessionStale);
        // 会话未带组身份时，能否免注销完全取决于 sg/sudo 是否真可用
        assert!(!can_apply_without_logout(&p, classify(&p)));
    }

    #[test]
    fn can_apply_without_logout_requires_real_mechanism() {
        let stale_sg = probe(&[("ACCOUNT_GROUPS", "user,sudo,docker"), ("SG_OK", "1")]);
        assert!(can_apply_without_logout(
            &stale_sg,
            classify(&stale_sg)
        ));

        let stale_sudo = probe(&[("ACCOUNT_GROUPS", "user,sudo,docker"), ("SUDO_NOPASS", "1")]);
        assert!(can_apply_without_logout(
            &stale_sudo,
            classify(&stale_sudo)
        ));

        // socket 属组不一致且既无 sg 也无 sudo → 必须注销
        let mismatch = probe(&[("SESSION_GROUPS", "user,docker")]);
        assert!(!can_apply_without_logout(&mismatch, classify(&mismatch)));

        // daemon 不可用/CLI 缺失永远不算「无需注销」
        assert!(!can_apply_without_logout(
            &probe(&[("DAEMON", "down")]),
            DockerPermState::DaemonDown
        ));
        assert!(!can_apply_without_logout(
            &probe(&[("CLI", "0")]),
            DockerPermState::CliMissing
        ));

        // daemon 已可用 → 与机制无关
        assert!(can_apply_without_logout(
            &probe(&[("DAEMON", "ok")]),
            DockerPermState::Ready
        ));
    }

    #[test]
    fn classify_socket_mismatch_when_session_has_group() {
        // 会话已在 docker 组仍被拒绝 → 注销无效，需修 socket
        let p = probe(&[("SESSION_GROUPS", "user,docker")]);
        assert_eq!(classify(&p), DockerPermState::SocketMismatch);
        assert!(needs_fix(classify(&p)));
    }

    #[test]
    fn fix_script_uses_socket_group_and_is_injection_safe() {
        let p = probe(&[("SOCK_GROUP", "docker-x")]);
        let s = fix_script("user", &p);
        assert!(s.contains("usermod -aG \"$grp\" \"$u\""));
        assert!(s.contains("'docker-x'"));
        assert!(s.contains("chgrp \"$want\" \"$sock\""));
    }

    #[test]
    fn fix_script_falls_back_to_docker_group_when_socket_unknown() {
        let p = parse_probe("USER=user\nCLI=1\nDAEMON=perm\nPROBE_END");
        let s = fix_script("user", &p);
        assert!(s.contains("'docker'"));
        assert!(s.contains("'/var/run/docker.sock'"));
    }

    #[test]
    fn valid_user_whitelist() {
        assert!(valid_user("nvidia"));
        assert!(valid_user("adm-be.user_1"));
        assert!(!valid_user(""));
        assert!(!valid_user("a b"));
        assert!(!valid_user("a;rm -rf /"));
        assert!(!valid_user("a'b"));
    }

    #[test]
    fn describe_mentions_group() {
        let p = probe(&[("SOCK_GROUP", "dockergrp")]);
        let d = describe(&p, DockerPermState::NotInGroup);
        assert!(d.contains("dockergrp"));
    }
}
