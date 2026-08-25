# DGX-Spark 双机直连 · 一键从零部署 —— 开发计划

> 状态：**已实现（v1）**——实现说明：密码 SSH 基建采用 russh（ring 后端，`common/ssh_pw.rs`）；部署成功自动写回多机互联节点表（决策 ② 采纳）；MTU 未设置（决策 ⑦ 不采纳）。代码：`src-tauri/src/pages/dgx_deploy.rs` / `src-tauri/src/common/ssh_pw.rs` / `src/views/settings.js`（`panel-multinode` 顶部卡片）。
> 关联背景：设置页「DGX直连配置」（`panel-multinode`）已有多机互联手动配置能力（光口 IP 需用户手工配好）。本文档新增 **一键从零部署**，覆盖「买线 → 插线 → 免密 SSH → 固定 IP → 连通检测 → Docker → 权限」全流程。

---

## 1. 背景与目标

### 1.1 现状痛点
- 多机互联要求用户**手工**配置光口固定 IP（`/etc/netplan/40-cx7.yaml`）、手工打通 SSH 免密、手工装 Docker，门槛高、易踩 DGX-Spark 的坑（`netplan apply` 会报错）。
- 现有 `ssh_run`（`common/ssh.rs`）只支持**密钥认证**（BatchMode），无法承载首次带密码的部署流程。

### 1.2 本功能目标
在设置页「DGX直连配置」Tab **最顶部**新增「DGX-Spark 双机直连 · 一键从零部署」（A = 控制机，B = 对等节点），一键完成：

| 步骤 | 内容 |
|------|------|
| 0. 准备提示 | 展示直连线选购与插线规范（静态文案，非自动步骤） |
| 1. 免密 SSH | A 生成 ed25519 密钥 → 用密码控制 B 生成密钥 → 互相拷贝公钥 |
| 2. 光口探测 | `ibdev2netdev` 分别探测 A/B，确认插口方向一致，得到 Up 网卡名 |
| 3. 固定 IP | 写 `/etc/netplan/40-cx7.yaml`（A=192.168.177.11/24，B=192.168.177.12/24）+ `nmcli managed no` + systemd-networkd 重启（**禁用 netplan apply**） |
| 4. 连通检测 | 双向 ping + 双向免密 ssh（走光口 IP），失败弹错误提示 |
| 5. Docker | 检测 A/B 是否安装；未装执行官方一键脚本 |
| 6. 权限 | 当前用户加入 docker 组（免 sudo 执行 docker） |

**全程每步幂等**（已满足则跳过，可重复执行）；**任一步失败 → 自动回滚 → 弹出错误提示**。

### 1.3 角色定义（重要）
- **A = 本机（ADM-BE 运行所在设备）**：所有本地操作走本地 shell；所有 B 的操作均**从 A 发起**（通过 SSH 密码）。
- **B = 对等节点**：用户在表单填写其**当前可达地址**（管理网 IP / 主机名，DGX Spark 出厂自带的 RJ45 管理口地址）、SSH 端口、用户名、密码。
- 密码语义：A 密码 = 本机 `sudo -S` 密码；B 密码 = SSH 认证密码 + 远端 `sudo -S` 密码（同一字段，与用户需求一致）。

---

## 2. 总体设计

### 2.1 UI 结构（`src/views/settings.js`，`panel-multinode` 内最顶部）

```
┌─ DGX-Spark 双机直连 · 一键从零部署 ──────────────────────────────┐
│ [准备提示卡]                                                      │
│  ● 直连线型号：200G QSFP56 DAC 直连铜缆（★不是 QSFP112 400G，     │
│    不要买错；买 1 条即可，插 2 条受内存带宽限制提升不大且更复杂）  │
│  ● 插线要求：DGX-Spark 有 2 个 QSFP 口，A/B 必须插【同一方向】    │
│    （如都插左边），否则自动配置会检测到插口不一致并报错           │
│                                                                    │
│ [A 控制机（本机）] 用户名 [__]  密码 [****] [👁]                  │
│ [B 对等节点]      地址 [__]  端口 [22]  用户名 [__]  密码 [****]  │
│                                                                    │
│ [▶ 开始一键部署]  [步骤进度条/步骤列表 7 项，逐步点亮]             │
│ [运行日志滚动区（等宽字体，按 [步骤N] 前缀，可复制）]              │
└────────────────────────────────────────────────────────────────────┘
```

- 表单元素规划（id 前缀 `zd-`，避免与壳层/其他面板冲突）：
  - `zd-a-user` / `zd-a-pass`（A 密码框 `type=password` + 显示切换）
  - `zd-b-addr` / `zd-b-port` / `zd-b-user` / `zd-b-pass`
  - `zd-start-btn`（部署中 disabled + 文案「部署中...」）
  - `zd-steps`（步骤列表容器，7 项，`done/active/error/pending` 状态类）
  - `zd-log`（日志区，自动滚底，`max-height` 带滚动条）
- **密码不持久化**：仅模块级内存变量；切页（unmount）后需重填。不写入 `Settings`、不落 localStorage。
- A 用户名自动回填 `get_local_username`（多机互联已有该惯例，`settings.js:893`）。

### 2.2 事件协议（沿用 AGENTS.md 全局监听规则）

- 新命令：`dgx_deploy_run(app, b_addr, b_port, b_user, b_pass, a_user, a_pass)`
- 新事件：`dgx-zero-deploy`（**必须**在 `src/index.html` `init()` 中注册一次、永不释放，转发给 `currentView.handleTauriEvent`；`settings.js` 的 `handleTauriEvent` 增加该事件分支，只做 DOM 更新）
- payload：`{ step: 0-6, phase: "start"|"run"|"done"|"error"|"rollback"|"rollback_done", detail: string }`
  - `start`：该步骤开始（点亮 active）
  - `run`：步骤内日志行（追加到日志区）
  - `done`：步骤成功（点亮 done）
  - `error` + `rollback`：失败，进入回滚（步骤标红，日志区追加回滚动作）
  - `rollback_done`：回滚完成，前端弹错误提示（复用现有错误弹窗样式）
- 前端按钮态：运行中禁点；`error` 后恢复可点（可修复后重跑，幂等）。

### 2.3 Rust 端模块划分

| 模块 | 内容 |
|------|------|
| `src-tauri/src/common/ssh_pw.rs`（**新增**） | 密码 SSH 执行基建（见 §3 选型）：`ssh_pw_run(host, port, user, pass, cmd, timeout, stdin) -> Result<(bool, String, String), AppError>`，支持向远端喂 stdin（sudo -S 用），输出打码防泄漏 |
| `src-tauri/src/pages/dgx_deploy.rs`（**新增**） | 7 步编排器 + 回滚 + 事件发射（复用 `settings.rs` 的 `pipe_emit` 模式） |
| `src-tauri/src/pages/mod.rs` / `lib.rs` | 注册 `dgx_deploy_run` 命令 |
| `src/views/settings.js` | 新卡片 UI + 表单 + 步骤条 + 日志区 + `handleTauriEvent` 分支 |
| `src/index.html` | `init()` 注册 `dgx-zero-deploy` 监听并转发 |
| `src/i18n.js` | EN 字典补 ~20 条新文案（原文即 key，漏译自动回退中文） |
| `src-tauri/Cargo.toml` | 新增依赖（见 §3 选型） |

不修改 `types.rs` / `Settings`（密码不持久化，无新配置字段）。

---

## 3. 密码 SSH 基建选型（待审核决策点 ①）

现有 `ssh_run` 走系统 `ssh` 客户端 + `BatchMode=yes`，**无法**承载密码认证（OpenSSH 密码输入需要 tty，管道喂不进）。两个候选：

| 方案 | 优点 | 缺点 |
|------|------|------|
| **A. russh（推荐）** | 纯 Rust、async（契合现有 tokio）、无系统依赖、密码不暴露在 `ps` 进程列表、ed25519 原生支持 | 编译 +1~2 分钟（Cargo 原生依赖）；需封装 ~150 行 exec/输入输出读写 |
| B. sshpass | 改动最小（`ssh_run` 加 `sshpass -p <pass>` 前缀即可，零新 crate） | 需 `apt install sshpass`（自举安装本身要 sudo 密码，用户已提供，可行）；密码可通过 `ps` 被同机用户看到；A 机若为 ARM Ubuntu 也能装 |

**建议：方案 A（russh）**。理由：密码安全（部署流程密码是核心资产）、省去自举安装环节、async 模型与 `run_pipe_with_progress` 一致。若审核倾向最小改动，可降级方案 B，接口层已抽象，切换成本低（`ssh_pw_run` 内部实现二选一）。

`ssh_pw_run` 关键实现点：
- 连接参数：`ConnectTimeout=8s`、`StrictHostKeyChecking=accept-new`（首连接受并缓存内存，不落盘，语义与现有 `ssh_run` 一致）、server alive。
- exec 远端命令，stdout/stderr 读满后返回（超时强杀会话）。
- 支持可选 stdin 喂入（`echo '<pass>' | sudo -S -p '' <cmd>` 由**远端 shell** 完成，Rust 仅传命令字符串；密码经 `sh_quote` 转义——注意 `sudo -S` 从 stdin 读密码，`-p ''` 抑制交互提示，实现「无反馈模式」）。
- 密码打码：任何日志/事件 detail 中出现密码原文时替换为 `******`（工具函数 + 断言，防 `dbg_log!` 泄漏）。

---

## 4. 步骤编排详细设计（核心）

> 通用原则：**每步先检测目标状态，已满足直接跳过并标 done**（幂等、可重跑）；**每步开始时先备份将被修改的远端状态**（authorized_keys 备份、netplan 备份），失败时按回滚矩阵恢复。

### Step 1 免密 SSH（A ↔ B 互通）
```
A 本机（非 sudo，当前用户）：
  1. [ -f ~/.ssh/id_ed25519 ] || ssh-keygen -t ed25519 -N '' -C 'adm-be@<A>' -f ~/.ssh/id_ed25519
  2. chmod 700 ~/.ssh; chmod 600 ~/.ssh/id_ed25519 ~/.ssh/id_ed25519.pub
  3. 读取 ~/.ssh/id_ed25519.pub → pubA

→ B（ssh_pw_run，密码认证）：
  4. mkdir -p ~/.ssh && chmod 700 ~/.ssh
  5. [ -f ~/.ssh/id_ed25519 ] || ssh-keygen -t ed25519 -N '' -C 'adm-be@<B>' -f ~/.ssh/id_ed25519
  6. grep -qF '<pubA>' ~/.ssh/authorized_keys 2>/dev/null || echo '<pubA>' >> ~/.ssh/authorized_keys
  7. chmod 600 ~/.ssh/authorized_keys
  8. cat ~/.ssh/id_ed25519.pub → pubB

A 本机：
  9. 备份 authorized_keys（cp 到 .adm-bak-<ts>，回滚用）
  10. grep -qF '<pubB>' ~/.ssh/authorized_keys 2>/dev/null || echo '<pubB>' >> ~/.ssh/authorized_keys

验证（B 用表单地址）：
  11. ssh -o BatchMode=yes -o ConnectTimeout=5 <b_user>@<b_addr> 'echo SSH_OK'  → 成功即 A→B 免密打通
  12. B→A 免密验证【延后到 Step 4 用光口 IP 做】（原因：光口未配时 B 无路由回 A；公钥互拷在 9/10 已完成）
```
- 回滚：恢复 A 的 authorized_keys 备份（Step 1 失败时，B 侧已写入的 pubA 一并按 `grep -vF` 移除；B 生成的密钥保留——幂等，下次重跑复用）。

### Step 2 光口探测（ibdev2netdev）
- A 本机与 B 各自执行 `ibdev2netdev`，解析每行 `rocepX port 1 ==> enpY (Up|Down)`：
  - 取 `(Up)` 行集合：A→{ifaceA, rocepA}，B→{ifaceB, rocepB}
  - 兜底：命令不存在时（驱动未加载/工具缺失），改用 `ip -br link` + `/sys/class/net/<if>/device/driver` 含 `mlx5_core` + `operstate=UP` 识别光口
- **判定规则**（对应需求「up 就代表是插入的，如果两边不一致要提示」）：
  | 情形 | 处理 |
  |------|------|
  | 任一侧 Up 数 = 0 | 报错「<A/B> 未检测到已插好的光口，请检查直连线是否插紧/识别」，中止（无回滚） |
  | 任一侧 Up 数 ≥ 2 | **正常**：DGX Spark 插 1 根线时同一物理口呈现 2 个 Up 镜像（如 `rocep1s0f1`/`roceP2p1s0f1`），不做数量判定 |
  | 两侧各 1 个但 rocep 名不一致 | 报错「插口方向不一致：A 插的是 <rocepA>，B 插的是 <rocepB>。请保持 A、B 插同一方向的口（如都插左边）」，中止（无回滚） |
  | 两侧各 1+ 个且排序后集合一致 | 通过，输出 iface（取每侧首个 Up 网卡，与手工流程一致；另一条为镜像/备用） |
- 输出事件 `detail`：`A: rocep1s0f1 → enp1s0f1np1 (Up)`、`B: ...` 供日志展示。

### Step 3 固定 IP 配置（严格按用户提供流程，禁用 netplan apply）
```
生成 YAML（indent=4 空格，模板固定，仅替换 网卡名/地址）：
  network:
      version: 2
      renderer: networkd
      ethernets:
          <iface>:
              dhcp4: no
              addresses: [192.168.177.1X/24]        # A=11, B=12

每台设备依次执行（A 本机 sudo -S 喂 A 密码；B 经 ssh_pw_run + 远端 sudo -S 喂 B 密码）：
  1. [ -f /etc/netplan/40-cx7.yaml ] && sudo cp /etc/netplan/40-cx7.yaml /etc/netplan/40-cx7.yaml.adm-bak-<ts>
  2. 写新文件（sudo tee 覆盖；内容先本地生成再整体传输，避免逐行转义出错）
  3. sudo nmcli device set <iface> managed no
  4. sudo systemctl restart NetworkManager
  5. sudo systemctl daemon-reload
  6. sudo systemctl enable --now systemd-networkd
  7. sudo systemctl restart systemd-networkd
  8. 校验: ip -4 addr show <iface> | grep 192.168.177.1X/24 → 未出现则失败回滚
```
- **特别说明（写进日志文案与文档）**：绝不执行 `netplan apply`（DGX-Spark 已知坑，报错）。配置生效靠**写入后先 `netplan generate`（仅生成 systemd-networkd 配置，不 apply、不接管接口）+ 重启 systemd-networkd**——只写 yaml 重启不生效（networkd 只读 `/run/systemd/network/` 生成文件）；回滚同样先 generate 再重启服务。
- 回滚：`sudo cp 40-cx7.yaml.adm-bak-<ts> 40-cx7.yaml`（无备份则删文件）+ `sudo nmcli device set <iface> managed yes` + `sudo systemctl restart NetworkManager` + 重启 systemd-networkd。B 侧回滚同样经 ssh 执行（B 的 SSH 走管理网地址，不受光口配置影响，回滚必然可达）。

### Step 4 连通检测（走光口）
```
A 本机：ping -c 3 -W 2 192.168.177.12                    → 丢包/全超时 → 失败
A 本机：ssh -o BatchMode=yes -o ConnectTimeout=5 192.168.177.12 'echo OK'   → A→B 免密（光口）验证
经 B（ssh_pw_run 192.168.177.12）：
        ssh -o BatchMode=yes -o ConnectTimeout=5 192.168.177.11 'echo OK'   → B→A 免密（光口）验证
```
- 失败 → 提示「A/B 光口直连不通」+ 自动回滚 Step 3 的网络配置（双向恢复）。

### Step 5 Docker 检测/安装
```
每台设备：
  1. 检测: docker info --format '{{.ServerVersion}}'（先 sudo -S，失败再直接 docker）
     → 已装（含 ServerVersion）→ 跳过
  2. 未装 → curl -fsSL https://get.docker.com -o /tmp/get-docker.sh
     （若 Settings.proxy_url 非空，追加 -x <proxy>，复用设置页代理）
     → sudo -S sh /tmp/get-docker.sh（架构自适应：DGX-Spark 为 ARM64 自动装 arm 包）
     → sudo -S systemctl enable --now docker
     → 复检 docker info 确认
```
- 回滚：**不卸载**（官方脚本无干净卸载路径，中途失败可能残留半装状态）——失败时保留现状，错误提示明确「请手动重跑官方脚本或检查网络后重试」。此为**有损回滚**例外，在 UI 日志中明示（见 §7 回滚矩阵）。

### Step 6 docker 组（免 sudo 执行 docker）
```
每台设备: sudo -S usermod -aG docker <user>   （A 用 A 密码、B 用 B 密码）
校验: id -nG <user> | grep -w docker
提示（日志）: docker 组对当前登录会话不生效，重启/重新登录后免 sudo；部署流程内 docker 命令仍走 sudo -S 兜底
```
- 回滚：`sudo -S gpasswd -d <user> docker`。
- 与现有 `fix_docker_permission`（settings.rs:382，pkexec 方案）不冲突：本流程为远端/无桌面场景的自动方案。

### 完成后（可选联动，待审核决策点 ②）
部署成功 → 自动把多机互联节点表写回：rank0 = { ip: `192.168.177.11`, ssh_user: A 用户名, is_self: true }、rank1 = { ip: `192.168.177.12`, ssh_user: B 用户名, ssh_port: 表单端口 }，`multi_node_args.enabled = true`，保存并刷新节点表 → 用户可直接多机启动模型。**默认建议开启**（一键从零到可用）；如审核认为自动改配置不妥，可改为部署成功后在日志区提示「可前往上方节点表填入光口 IP」。

---

## 5. 关键实现要点

1. **命令注入防护**：所有远端命令经 `sh_quote`（ssh.rs 已有）逐 token 转义；Rust 侧拼装时把 `b_addr / b_user / a_user / iface` 全部按「参数值」路径处理，绝不拼裸字符串进 shell。
2. **密码处理**：前端不持久化；Rust 侧仅存于命令局部变量；事件 payload 与 `dbg_log!` 输出统一经 `redact()` 打码；`ps` 层面由 russh 规避（方案 A）。
3. **事件频率**：日志行为 `run` 逐条推送；进度非百分比制（步骤级粒度），前端以步骤点亮 + 日志行代替进度条（避免过度设计）。
4. **并发与超时**：每步整体超时（本地命令 90s、远端 ssh 120s、docker 安装 10min），超时按失败回滚处理；`spawn + kill` 模式沿用 `ssh_run` 防孤儿进程的写法。
5. **重复执行**：整流程幂等（每步先检测），日志区每次执行前清空。
6. **`sudo -S -p ''` 无反馈**：远程命令统一 `echo '<pass>' | sudo -S -p '' <cmd>` 形态；密码错误时 stderr 含 `incorrect password`/`Sorry, try again` → 步骤失败 → 回滚 + 弹「密码错误，请检查 <A/B> 密码」。

---

## 6. 回滚矩阵

| 步骤 | 失败触发回滚动作 | 备注 |
|------|-----------------|------|
| Step 1 免密 SSH | 恢复 A `~/.ssh/authorized_keys`（备份还原）；从 B `authorized_keys` 移除 pubA（`grep -vF`） | B 侧密钥保留（幂等复用） |
| Step 2 光口探测 | 无（纯探测，无副作用） | 中止即可 |
| Step 3 固定 IP | 双侧恢复 netplan 备份/删除 + `nmcli managed yes` + 重启 NM/networkd | B 经管理网地址回滚，必然可达 |
| Step 4 连通检测 | 同 Step 3 回滚（网络未通即恢复原状） | — |
| Step 5 Docker | **不卸载**，明示半装风险 | 有损回滚例外，人工介入 |
| Step 6 权限 | `gpasswd -d <user> docker` | — |

回滚动作实时通过 `rollback` / `rollback_done` 事件展示在日志区，最终弹窗统一格式：`[步骤N] <错误原因>\n已自动回滚：<动作列表>\n可修复后重新执行一键部署`。

---

## 7. 文件改动清单

| 文件 | 改动 |
|------|------|
| `src-tauri/Cargo.toml` | + `russh`、`russh-keys`（方案 A） |
| `src-tauri/src/common/ssh_pw.rs` | 新增：`ssh_pw_run` + 打码工具 `redact()` |
| `src-tauri/src/pages/dgx_deploy.rs` | 新增：`dgx_deploy_run` 编排器（7 步 + 回滚 + 事件） |
| `src-tauri/src/pages/mod.rs`、`src-tauri/src/lib.rs` | 注册命令（invoke_handler） |
| `src/views/settings.js` | `panel-multinode` 顶部新卡片 UI（模板、表单、步骤条、日志区）；`handleTauriEvent` 增加 `dgx-zero-deploy` 分支（只做 DOM 更新） |
| `src/index.html` | `init()` 注册 `dgx-zero-deploy` 全局监听 → 更新 `__adm_state` → 转发 `currentView.handleTauriEvent` |
| `src/i18n.js` | EN 字典补新文案 |
| `doc/`（本文档） | 落地后同步更新 AGENTS.md「多机互联」小节 |

---

## 8. 测试计划

1. **前端**：`pnpm typecheck` 通过；手动过一遍表单校验（密码为空/地址非法拦截）。
2. **Rust**：`cargo check`（src-tauri 目录）；`dgx_deploy_run` 单测覆盖：ibdev2netdev 输出解析（含 0 Up / 2 Up / 两侧不一致 / 一致 四种样例，均来自用户提供样例文本）、netplan 模板生成、authorized_keys 幂等追加/移除。
3. **手工场景矩阵**（真机验证）：
   - 正常全流程（A/B 各 1 台 DGX-Spark，1 根 QSFP56 DAC）
   - B 不可达 / B 密码错误 / A sudo 密码错误
   - A、B 插口方向不一致（各插一边）→ 报错文案
   - B 未插线 → 报错
   - 已装 Docker 设备（跳过 Step 5）
   - Step 3 中途断电/断网 → 回滚后重跑成功
   - 全流程重跑（幂等）
4. **回归**：多机互联现有「测试连通 / 同步镜像 / 同步模型」不受影响。

---

## 9. 风险与待确认项

| # | 事项 | 建议 | 状态 |
|---|------|------|------|
| ① | 密码 SSH 基建：russh vs sshpass | **russh**（安全/无依赖），接口已抽象可降级 sshpass | **待审核** |
| ② | 部署成功后是否自动写回多机互联节点表（enabled=true） | 默认开启（真正一键） | **待审核** |
| ③ | B 的初始「当前可达地址」由用户填写（管理网 IP） | 合理（DGX-Spark 出厂有管理口 IP）；不支持自动发现 | 拟确认 |
| ④ | netplan 固定文件名 `40-cx7.yaml`，存在则备份覆盖 | 与用户给定流程一致 | 拟确认 |
| ⑤ | 密码不持久化（切页重填） | 安全优先；可在 UI 注明 | 拟确认 |
| ⑥ | Step 5 docker 半装不回滚（有损例外） | 明示用户 | 拟确认 |
| ⑦ | 光口 MTU 是否顺带设 9000 | **默认不加**（与用户指令一致，避免引入新变量） | 待审核 |

---

## 10. 验收标准

- 用户按「准备提示」选购并插好 1 根 QSFP56 DAC（A/B 同一方向）后，填写 A/B 用户名密码 → 点一次「开始一键部署」→ 步骤 1-6 全绿 → 日志区可核对每步命令与结果。
- 全流程失败点均有明确中文错误 + 自动回滚 + 可重跑。
- 部署完成后多机互联可直接使用（节点表已就绪 / 或按决策 ② 提示手动填写）。