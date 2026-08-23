# DGX Spark 多机互联 — 开发计划（与实际代码同步）

> **文档约定**：本文档与实际代码状态同步维护——代码每有改动，本文档随改随更
> （AGENTS.md 已约定「当新的功能发生变化时候即时更新文档」）。
> **AGENTS.md 只记录关键注意事项**，细节均在本文档与 `doc/sglang-deployment.md` 中。

---

## 0. 实施状态

| 阶段 | 内容 | 状态 |
|---|---|---|
| P1 | 数据结构 + 设置页 Tab 表单 + i18n + 持久化 | ✅ 完成 |
| P2 | ssh.rs + 探活命令 + 节点表探活 UI | ✅ 完成 |
| P3 | 参数拼装 + 多机启动/停止链路 + model-log 事件 | ✅ 完成 |
| P4 | 启动前置校验体系 + 文档同步 | ✅ 完成 |
| P5 | 真机调优（NVFP4/FP8、NCCL env、超时重试） | ⬜ 待真机验证（需 2 台 DGX Spark） |

**实现记录（与设计文档的偏差，以代码为准）**：
- **未做 `build_sglang_args` 重构**（单机零改动原则）：改为新增独立纯函数 `build_multi_node_args`（`model_list.rs`），单机 `start_sglang_docker` 完全未动
- **取消 `multi_node_probe_all` 命令**：前端「全部测试」用 `Promise.all` 并行调用 `multi_node_probe`
- **回滚语义细化**：远端预检/启动/快速就绪（30s）任一失败 → 停止已启动远端 + 报错；本机 spawn 失败 → 停止全部远端；**运行期**远端容器异常退出 → 监控线程（10s 轮询）仅转发日志告警，不自动级联停止
- **多机停止识别**：`stop_model` / `cleanup_processes` 通过容器名后缀 `-rank-0` 识别多机模式，SSH 逐台下发 `docker stop/rm`（20s/3s 超时）
- **多机参数放命令最后**：`--tp N --nnodes N --node-rank R --dist-init-addr` 置于 `sglang_flags` 之后，确保跨节点 TP 拓扑不被覆盖（与"sglang_flags 优先级最高"在单机语义下的合理例外）
- **远端网卡识别**：`iface` 为全局单值（注入所有节点 `-e NCCL_SOCKET_IFNAME`，各节点需同名网卡——同款 Spark 硬件通常一致）；探活时回传远端网卡列表（`IFACES:` 段）合并进设置页下拉，可手动输入；迭代二可做 per-node 网卡
- **远端启动方式**：SSH `nohup docker run ... > /tmp/adm_sglang_<model>_rank_<R>.log 2>&1 &`，日志落盘便于故障排查
- **直连节点 IP 语义（2026-08 调整）**：节点清单 IP = **光口 IP**（ConnectX-7 QSFP 上配置的互连地址），非 RJ45 局域网 IP；`dist-init-addr` 亦用该地址。设置页 Tab 已更名「DGX直连配置」，节点清单更名「直连节点」并**固定恰好 2 台**（本机 + 1 台直连节点，UI 截断 + 后端 `validate_multi_node` 收紧 `len == 2`），「主节点（本机）」引导区（管理网卡/IP 自动回填、ConnectX 下拉）已移除；`get_local_network_info` / `ensure_ssh_key` 命令保留待后续阶段复用
- **测试连通**：探活检查远端 nvidia-smi / docker daemon / **镜像存在性（`docker image inspect` 本机当前镜像）** / 模型目录；不再探测远端网卡（SSH 可达即互联已通）
- **镜像一键推送（新增命令 `push_image_to_remote`）**：独立操作行「同步镜像到直连节点」= **流式管道** `docker save | [pv -s] | gzip -1 | ssh 'gunzip | docker load'`（不落盘、复用光口带宽；有 pv 且取到镜像 size 时带百分比）；远端校验 `LOAD_OK`；效率事件 `image-push-progress`
- **模型一键同步（新增命令 `sync_model_to_remote`）**：独立操作行「同步模型到直连节点」= **先监测远端 `.done`（已同步则跳过）**，未同步则全量同步本机主模型（正在运行的模型优先，无则第一个完整模型）；rsync 增量（`--info=progress2 --no-inc-recursive`，无 rsync 回退 scp -r），校验远端 `.done`；进度事件 `model-sync-progress`；两者均已在 index.html 全局事件注册并转设置页 `handleTauriEvent`。**测试连通按钮已移到节点行内**（原独立按钮删除）；**模型目录默认策略（2026-08）**：本机行自动填 `<data_dir>/models`（新命令 `get_app_data_dir`），直连节点留空自动 `/home/<SSH用户>/models/<模型ID>`（`effective_remote_model_dir` 兜底，启动/同步/探活统一生效，校验不再强制必填）
- **审查修复（2026-08）**：① `~/` 路径单引号包裹后波浪号不展开 → 新增 `quote_remote_path`（`~` 前缀保持引号外）并全局替换远端路径转义，默认目录改绝对路径；② 容器名（含 model_id，上游 model.json 非信任输入）在远端 shell 命令（start/stop/docker ps filter）中统一 `sh_quote` 防注入；③ `ssh_run` / `run_pipe_with_progress` 超时强制 kill 子进程（tokio future drop 不杀进程，曾会泄漏孤儿 ssh/sh）；④ 远端运行期监控线程在模型停止后静默退出（检查 `running_container`），消除停止过程的误报；⑤ 同步失败错误信息附带 stderr 尾段（4KB）便于诊断

## 1. 背景与目标

ADM-BE 目前只在单台 DGX Spark（GB10，单 GPU，128GB 统一内存）上以 Docker 方式启动 SGLang。
本计划支持 2 台及以上 Spark 组成 SGLang 多节点集群（`TP=N`，N = 节点数）：

- 2× Spark = 256GB 统一内存，可支撑约 405B 参数模型推理
- 设置页新增独立「多机互联」Tab 配置节点清单与互联参数，一键启动/停止集群
- 单机模式（`multi_node_args.enabled = false`）严格走现有代码路径，行为不变

## 2. 已核实的技术要点（官方文档调研结论，P5 真机验证时以此为准）

> 来源：SGLang Server Arguments 文档、SGLang Multi-Node Deployment 文档、SGLang cookbook
>（DGX Spark 多机 recipe）、NVIDIA DGX Spark User Guide、NVIDIA dgx-spark-playbooks。

| 项 | 结论 |
|---|---|
| 跨机互连硬件 | **无 NVLink 跨机**（GB10 内 NVLink-C2C 仅 CPU↔GPU，单 GPU/台）。机间走 **ConnectX-7 以太网**：2× QSFP 200GbE 口（各带 RoCE 设备）+ 1× RJ45 10GbE 管理口（`enP7s7`） |
| 支持拓扑 | 2 机直连（一根 QSFP DAC 即满带宽）；3 机环；4 机需 200GbE 交换机（RoCE） |
| SGLang 多机参数 | `--nnodes N`（各节点相同）+ `--node-rank R`（0..N-1）+ `--dist-init-addr <节点0IP>:<端口>`（**所有节点填同一地址**）；`--nccl-port <端口>` 可选（默认随机） |
| TP 语义 | `--tensor-parallel-size` 为**全集群总 TP 数**（跨节点切分）。2×Spark = `--tp 2` |
| 就绪信号 | 仅**节点 0** stdout 出现 `Uvicorn running on`；远端节点以 SSH 进程存活 + `docker ps` 轮询 |
| 死锁/踩坑 | 多机卡死 → 加 `--disable-cuda-graph`；TP 报 "peer access is not supported" → 加 `--enable-p2p-check` |
| Docker 要求（多机） | 必须 `--network host`（**替代**现有 `-p port:port`）；追加 `--device /dev/infiniband`、`--ulimit memlock=-1:-1`、`--cap-add IPC_LOCK`（RoCE 需要） |
| NCCL 环境变量 | `NCCL_SOCKET_IFNAME` / `GLOO_SOCKET_IFNAME` 指向互连网卡；RoCE 异常时 `NCCL_IB_GID_INDEX=3`、`NCCL_IB_SUBNET_AWARE_ROUTING=1`、`NCCL_NET_PLUGIN=none` |
| 镜像 | 沿用设置页 `sglang_args.image`（默认 `lmsysorg/sglang:v0.5.17`，多架构含 arm64）；多机专用镜像通过同一配置项替换 |

## 3. 现状代码基线（实现必须对齐的既有结构）

| 位置 | 现状事实 |
|---|---|
| `src-tauri/src/common/types.rs` | `Settings { launch_params, sglang_args, debug_logging, language }`；`SglangArgs` = image / shm_size / context_length / tensor_parallel_size / mem_fraction_static / dtype / quantization / kv_cache_dtype / schedule_policy / max_running_requests / max_queued_requests / chunked_prefill_size / log_level / log_requests / enable_metrics / reasoning_parser / tool_call_parser / extra_args。全部 `#[serde(default)]`，旧 `config.json` 缺字段可解析 |
| `src/views/settings.js` | 左导航 6 项（顺序）：`launch-params` / `docker-mirror` / `appearance` / `logs` / `version` / `about`；`switchPanel` 用 `data-panel` + `panel-<id>`；保存模式：`saveParams()` = `load_settings` → 只替换 `launch_params` / `sglang_args` → `save_settings`；`autoSave` 监听 id 列表 `setupAutoSave()`；i18n 走 `_t()`（`src/i18n.js` zh/en） |
| `src-tauri/src/pages/model_list.rs` | `start_sglang_docker(app, state, model_id, model_dir, params, device, sglang_version, sglang_flags)`：`check_docker_env`（CLI/daemon/nvidia runtime/镜像缺失自动 pull/端口占用）→ 清理残留容器 → 拼 `docker run`；**模型清单 `sglang_flags` 最后追加、优先级最高**（可覆盖设置页同名参数）；MTP 自动启用跳过条件 = `extra_args` **或** `sglang_flags` 含 `speculative-algorithm`；就绪信号 stdout `Uvicorn running on`；`stop_model`（docker stop/rm + 兜底杀进程）、`cleanup_processes`（应用退出统一入口，幂等） |
| `src-tauri/src/lib.rs` | `invoke_handler(tauri::generate_handler![...])` 注册所有命令；新命令需在此登记 |
| 配置持久化 | `<data_dir>/config.json`，`save_settings`（写文件 + sync_all）/ `load_settings`，命令签名已固定，Settings 扩展字段**无需改 IPC 层** |

## 4. 数据模型设计（P1 改 `types.rs`）

`Settings` 新增（`#[serde(default)]`，旧配置兼容）：

```rust
/// 多机互联配置（DGX Spark 集群）
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct MultiNodeArgs {
    pub enabled: bool,            // 总开关：false = 现有单机路径
    pub nodes: Vec<NodeInfo>,     // 按下标即 rank；[0] 必须是本机 is_self=true
    pub dist_init_port: u16,      // --dist-init-addr 端口，默认 6464，不得与服务端口冲突
    pub nccl_port: u16,           // NCCL 通信端口（0 = 自动）
    pub iface: String,            // 互连网卡（NCCL_SOCKET_IFNAME / GLOO_SOCKET_IFNAME，空 = 自动）
    pub use_roce: bool,           // 启用 RoCE（追加 --device /dev/infiniband 等 flag）
    pub ssh_key_path: String,     // SSH 私钥路径（-i；空 = ssh-agent / 默认 key）
}

pub struct NodeInfo {
    pub ip: String,
    pub ssh_user: String,
    pub ssh_port: u16,            // 默认 22
    pub is_self: bool,            // 本机标记（仅 nodes[0] 可为 true）
    pub model_dir: String,        // 该节点本地模型目录（须已下载好模型）
}
```

## 5. 前端设计（P1 改 `src/views/settings.js`）

- 导航新增第 3 项（`docker-mirror` 之后、`appearance` 之前）：`data-panel="multinode"`「多机互联」；新增 `<div id="panel-multinode" class="panel">`，复用 `param-row` / `param-group` / `panel-title` 样式，选择器保持 `settings-` 前缀
- Tab 内容：① 总开关（checkbox，开启后显示配置区）② 节点清单表（增删行：IP / SSH 用户 / SSH 端口 / 「本机」复选（仅首行可勾）/ 模型目录；每行「测试连接」+ 顶部「全部测试」）③ 互联参数（引导端口 / NCCL 端口 / 互连网卡 / RoCE 开关 / SSH 私钥路径）④ 拓扑说明（2 直连 / 3 环 / 4 交换机 · 各节点需各下载一份模型）
- 保存：`saveParams()` 中 `s.sglang_args = ...` 旁追加 `s.multi_node_args = getMultiNodeArgsFromForm()`；`setupAutoSave()` 监听新 id；载入侧 `fillMultiNodeArgsForm(settings.multi_node_args)`（`loadSettings` 完成后调用，参照现有 `fillSglangArgsForm`）
- i18n：`src/i18n.js` 补 多机互联相关 zh/en 键（设置页文案全部走 `_t()`）

## 6. 后端设计

### 6.1 新模块 `src-tauri/src/common/ssh.rs`（P2）

```rust
/// 远端执行命令（BatchMode 免交互、防挂起；不存储密码）
pub fn ssh_run(host: &str, user: &str, port: u16, key: Option<&str>, cmd: &str, timeout: Duration)
    -> Result<std::process::Output, AppError>;
```
- `ssh -o BatchMode=yes -o ConnectTimeout=5 -o StrictHostKeyChecking=accept-new -p <port> [-i <key>] <user>@<host> <cmd>`
- 部署机为 Linux（DGX Spark），自带 OpenSSH；Windows 仅开发/探活场景（内置 OpenSSH 客户端）

### 6.2 新 Tauri 命令（`settings.rs`，P2，登记进 `lib.rs` generate_handler）

- `multi_node_probe(ip, user, port, key) -> ProbeResult`：SSH 远端执行 `nvidia-smi` 简版 + `docker info` 简版 + 模型目录存在性 → `{ ok, gpu, docker, modelExists, error }`，结果实时显示在节点表行
- `multi_node_probe_all() -> Vec<ProbeResult>`：遍历节点清单并行探活（tokio join_all）

### 6.3 启动链路（P3 改 `model_list.rs`）

- 拆出纯函数 `build_sglang_args(...) -> Vec<String>`（现有单机拼接逻辑原样平移，便于单测与回归对比）；`start_sglang_docker` 仅保留执行路径
- 入口分支：`multi_node_args.enabled && nodes.len() >= 2` → `start_multi_node(...)`；否则走现有 `start_sglang_docker` 不变
- **节点 0（本机）**：`--network host`（去掉 `-p`，容器内 host 仍 `0.0.0.0`）；`--tp N --nnodes N --node-rank 0 --dist-init-addr <node0IP>:<distInitPort>`；`nccl_port > 0` 时 `--nccl-port`；`use_roce` 时 `--device /dev/infiniband --ulimit memlock=-1:-1 --cap-add IPC_LOCK`；`iface` 非空时 `-e NCCL_SOCKET_IFNAME` / `-e GLOO_SOCKET_IFNAME`；就绪信号不变（stdout `Uvicorn running on`）
- **远端节点 R**：SSH 后台执行同镜像同参数（仅 `--node-rank R`、模型目录用该节点 `model_dir`、容器名 `adm-sglang-<model>-rank-R`）；就绪 = SSH 进程存活 + 节点 0 `docker ps` 轮询双确认；任一台失败 → 停止全部（含已启动节点），`model-log` 事件逐台展示错误
- **沿用既有规则**：`check_docker_env` 预检流程；模型清单 `sglang_flags` 多机模式同样最后追加、优先级最高（含 `speculative-algorithm` 覆盖 MTP 自动启用的判断）；MTP 自动启用规则不变；设置页参数仅非空/非默认值拼接

### 6.4 停止链路（P3）

- `stop_model` / `cleanup_processes` 统一走 `stop_multi_node`：节点 0 先停 → 逐台 SSH `docker stop/rm`（幂等）
- SSH 不可达节点：记录告警写日志（远端残留容器需人工清理，写入 `sglang-deployment.md` 排查表）；若远端装有 ADM 可在该机手工停止（远端管理入口列为迭代二）
- 单台 SSH 停止超时上限 30s，超时跳过不阻塞其他节点

### 6.5 兼容性保持

- 单机模式代码路径零改动（`enabled=false` 时 `build_sglang_args` 输出与重构前一致，git diff 验证）
- WebUI「查看模型」仍指向节点 0 端口，URL 不变

## 7. 启动前置校验（start 前逐项，失败 toast + model-log 列出全部原因）

1. 节点数 ≥ 2 且 `nodes[0].is_self`
2. 各节点 IP 可达（ssh probe）
3. 各节点 docker / nvidia-smi 正常
4. 各节点模型目录存在（v1 要求各节点本地各下载一份；SSH/rsync 自动分发 = 迭代二）
5. 引导端口、服务端口未被占用

## 8. 测试计划

- 单测：`build_sglang_args` 多机 2/3/4 节点、RoCE 开/关、nccl_port 有无、`--network host` 且无 `-p` 断言；`enabled=false` 输出与重构前完全一致
- 静态：`pnpm typecheck`
- 真机（2× Spark 直连）：`--tp 2` 跑 NVFP4 70B 与 FP8 模型，记录 TTFT/TPOT，对照 NVIDIA 参考（70B NVFP4 TP2：TTFT 33.4s→21.4s、TPOT 269ms→133ms，约 2×）；NCCL 卡死排查路径：`--disable-cuda-graph` → `NCCL_IB_GID_INDEX=3` → `NCCL_IB_SUBNET_AWARE_ROUTING=1` → `NCCL_NET_PLUGIN=none`
- 3 机环 / 4 机交换机拓扑（硬件允许时）验证启停与就绪判断

## 9. 文档同步清单（代码改动后必改此处）

| 文档 | 同步内容 |
|---|---|
| 本文档 | §0 勾选实施状态；§3 现状基线行号/签名随代码修正；§4-§8 与实际实现对齐 |
| `doc/sglang-deployment.md` | 新增「多机互联」章节（含 §2 技术要点表、启动命令样例、排查路径、容器命名 `adm-sglang-<model>-rank-R`） |
| `AGENTS.md` | **只记关键注意事项**：实现完成后补「多机互联」小节（总开关、`--network host`、容器命名、文档指针），不搬运细节 |

## 10. 分阶段排期

| 阶段 | 内容 | 交付物 |
|---|---|---|
| P1 | types.rs 数据结构 + 设置页 Tab 表单 + i18n + 持久化 | 配置可保存/加载 |
| P2 | ssh.rs + `multi_node_probe` 命令 + 节点表探活 UI | 配置 + 连通性自检 |
| P3 | `build_sglang_args` 重构 + 多机启动/停止链路 + model-log 事件 | 2 机跑通 TP=2 |
| P4 | 启动前置校验体系 + 文档同步（§9 清单执行） | 3/4 机、失败引导 |
| P5 | 真机调优（NVFP4/FP8、NCCL env、超时重试） | 稳定可用 |

## 11. 开放问题（已定案）

1. ~~远端节点是否也装有 ADM？~~ **定案：远端不装 ADM**。远端职责只有"跑容器 + 存模型"，由节点 0 统一 SSH 编排；SSH 不可达的残留容器走文档手工命令兜底；模型分发用 rsync/共享存储（v1 各节点手动下载）。
2. ~~模型分发~~ **v1 定案**：各节点手动各下载一份，启动前逐台校验（含 `.done`）；SSH/rsync 自动分发列为迭代二。
3. ~~互联网卡选择~~ **v1 定案**：默认留空（NCCL 自动发现）；直连/异常时在设置页指定网卡。RoCE 默认关闭，QSFP 直连建议开启。