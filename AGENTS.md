# ADM-BE — Agent 指南

## 开发命令（始终使用 `pnpm`，不要用 `npm`/`yarn`）
- `pnpm tauri dev` — 热重载开发模式
- `pnpm tauri build` — 生产构建
- `pnpm tauri clean` — 清理构建产物
- `pnpm tauri:build:windows` / `:macos` / `:linux` — 跨平台构建
- `pnpm typecheck` — 前端类型检查（tsc --noEmit，只检查不产出，改完前端必跑）

> 注意：`pnpm typecheck` 依赖 typescript 平台二进制包；若报 "Executable not found"（pnpm 软链断裂），需重建 `node_modules/.pnpm/typescript@<ver>/node_modules/@typescript/typescript-win32-x64` 的 junction 指向 `.pnpm/@typescript+typescript-win32-x64@<ver>` 目录。

## 架构
- **Tauri 2.11.2** + Rust 后端 + **原生 HTML/CSS/JS**（无框架、无打包工具）。
  所有前端源码在 `src/` 目录下，作为 `frontendDist` 原样提供。
- **推理引擎 = vLLM（Docker 部署，`eugr/spark-vllm` fork）**：llama.cpp 与 SGLang 方案均已移除。模型以 **HF safetensors 目录**（`models/<model_id>/`，含 `.done` 完成标记）下载，`start_model` 检测目录模型后走 `start_vllm_docker`：`docker run --gpus all --shm-size <配置> -p <port>:<port> -v <模型目录>:/models/<model_id>:ro <镜像> vllm serve ...`。**镜像由远程 model.json 的 `vllm_image` 字段唯一指定**（每模型独立配置，本地不再保留硬编码兜底；缺字段直接拒绝启动）。容器内 host 固定 `0.0.0.0`（端口映射必需），端口沿用设置页（默认 8000）。就绪信号为 stdout `Application startup complete` / `Starting vLLM API server` / `Uvicorn running on`。停止/退出走 `docker stop/rm` + 杀进程兜底。完整流程见 `doc/vllm-deployment.md`。
- **单窗口 SPA（单页应用）** + hash 路由：
  - `index.html`（外壳）含 `#view-root` 容器、底部硬件栏与导航。
  - 3 个视图（`model_list` / `model_image` / `settings`）各自为独立 **ES 模块**（`src/views/*.js`），默认导出 `{ template, mount(root, params), unmount() }`。模型运行后「查看模型」直接用系统浏览器打开 WebUI（`window.openUrl`），不再有 chat 视图。
  - `index.html` 通过动态 `import()` 异步加载视图模块，把 `template`（含 `<style>` 的 HTML 字符串）注入 `#view-root`，调用 `mount`/`unmount` 管理生命周期。
- **机型适配**：首页顶部「适配机型」下拉（选项从 `model_support_devices` 收集去重，localStorage 记住选择，存 `S().currentDeviceFilter`），模型列表按机型过滤（空数组 = 全机型可用）；机型选择随 `start_model` 的 `device` 参数传入 Rust，当前适配 `dgx-spark-128G`。
- CSS/JS **内联**在每个视图模块的 `template` 字符串或模块函数内，保持零运行时依赖。
- **样式隔离约定**：全局 reset（`* {}`）与 `body` 样式只由 `index.html` 壳层提供，视图内不得重复定义；视图选择器带视图前缀（`settings-*` 等）；视图内元素 id 不得与壳层 id（`app` / `view-root` 等）重复。
- **类型检查**：`jsconfig.json` 开启 `checkJs`，全局类型声明在 `src/types.d.ts`；历史视图暂以 `// @ts-nocheck` 豁免，新代码不得新增此标记（用 JSDoc 注解）。未配置 linter、formatter 或测试框架。

## IPC 注意事项（重要）
- SPA 运行在 Tauri 主窗口内，**直接**调用 `window.__TAURI__.core.invoke` / `.event.listen`，无需 `postMessage` 代理。
- `index.html` 初始化时把 `window.__adm_invoke` / `window.__adm_listen` 暴露给所有视图模块；视图模块通过这两个全局引用调用 IPC。
- **事件监听必须在 `index.html` 全局层注册（`init()` 中一次注册、永不释放）**，不得在视图模块 `mount`/`unmount` 中注册/释放。原因：视图切换时 `unmount` 会释放监听，导致切页期间事件丢失、状态不同步。全局监听只更新 `window.__adm_state`，再通过 `currentView.handleTauriEvent(type, payload)` 转发给当前视图做 DOM 更新（视图未挂载时静默跳过）。视图模块默认导出需包含 `handleTauriEvent` 方法。
- **共享状态** `window.__adm_state`（systemInfo / runningModelId / modelList / currentDeviceFilter 等）跨视图共享，切换不丢。
- 视图 `mount` 时 `listen()` 保存 unlisten 句柄，`unmount` 时统一调用以防事件重复绑定（泄漏）。
- 子页面 → 父窗口导航：使用 `location.hash = "#/list"` 等 hash 路由。

## Rust 后端（`src-tauri/src/`）
| 模块 | 关键命令 |
|--------|-------------|
| `index.rs` | `get_system_info`, `check_update`（远程 `d/update.json`，无 llamacpp 逻辑） |
| `model_list.rs` | `fetch_model_list`, `scan_local_models`, `scan_part_files`, `download_model`（多文件目录下载）, `start_model`（目录模型 → `start_vllm_docker` / 多机 `start_multi_node`）, `stop_model`（docker stop/rm）, `delete_local_model` |
| `settings.rs` | `save_settings`（直接写入 + `sync_all` 刷盘，持久化 `vllm_args` / `multi_node_args`）, `load_settings`, `get_app_version`, `multi_node_probe`（SSH 探活）, `push_image_to_remote`（流式镜像同步） |
| `model_image.rs` | `check_sd_exists`, `download_and_extract_sd`, `start_sd_generation`, `stop_sd`, `save_sd_image_as` |

## 关键注意事项
- **vLLM 镜像**：由远程 model.json `vllm_image` 字段唯一指定（如 `ghcr.io/spark-arena/dgx-vllm-eugr-nightly-b12x:2026082301`），本机不再保留默认兜底。**镜像拉取统一在模型下载阶段完成**——`download_model` 下完模型文件（写 `.done`）后自动调 `pull_docker_image` 拉镜像（已存在则跳过）；`start_vllm_docker` / `start_multi_node` 启动时只做 `check_docker_env`（CLI/daemon/GPU 预检 + `docker image inspect` 校验），镜像缺失直接报「请回到模型列表重新点击下载触发拉取」，**不在启动时拉镜像**。镜像缺失场景：用户 `docker rmi` 误删、镜像被 Docker 清理、模型清单新增镜像未拉。本应用对接的是 `eugr/spark-vllm-docker` 社区 fork（专门针对 DGX Spark / GB10 sm12x 优化），**不要去查官方 vLLM 文档作为参数权威**——fork 含大量自定义（多机模式严格对齐 fork `launch-cluster.sh exec_no_ray_cluster`：head/worker 容器均跑 `sleep infinity`，先给每个 worker 派发 `vllm serve --headless --distributed-executor-backend mp --nnodes N --node-rank R --master-addr <head_ip> --master-port <port>`，最后 head 起完整 `vllm serve ...`（同参数但 node-rank 0、无 --headless）并等待所有 rank join；绝对不能用 `--distributed-executor-backend ray`（fork 镜像 pydantic 校验 nnodes>1 只允许 mp/uni/external_launcher）；`--speculative-config {method:dflash|dspark,...}`、`--attention-config` / `--reasoning-config` / `--compilation-config` JSON 配置、`--load-format instanttensor`、`--dspark-noise-token-id` 等 hf-overrides、`--default-chat-template-kwargs.*` 等），upstream vLLM 不直接兼容这些自定义入口与参数。升级镜像版本只改 model.json；切到 upstream vLLM 时需同步调整 `start_multi_node` 多机启动方式（压测后端由 `benchmark.rs` 启动前探测容器 `vllm bench serve --help` 自动选择，无需手动改）。
- **启动参数权威源**：`https://github.com/eugr/spark-vllm-docker`（README + `recipes/` 配方 + `mods/` 补丁说明 + `examples/` 启动脚本）。
- **模型下载（多文件目录）**：`model_download_files` 清单逐文件 `.part` 续传，下载完成写 `.done`；所有 `huggingface.co` 自动替换为 `hf-mirror.com`（设置页「代理」开启时跳过镜像替换，直接用源链接）。设置页「代理」Tab（`Settings.proxy_url`，如 `http://127.0.0.1:1080`）配置后，模型/SD 文件下载走该代理（reqwest 客户端附 `Proxy`）；docker pull 子进程另注入代理 env 兜底，标准 dockerd 需 daemon.json `proxies` 才生效——设置页「代理」→「保存并重启 Docker」（复用镜像加速面板的提权安装/重启链路，`save_docker_proxy_config`）。
- **vLLM 参数**：设置页「模型启动参数」面板配置 `vllm_args`（shm_size / tp / gpu-memory-utilization / quantization / kv-cache-dtype / distributed-executor-backend / load-format / block-size / tokenizer-mode / 推理 & 工具 parser / trust-remote-code / enable-auto-tool-choice / max-num-seqs / max-num-batched-tokens / `extra_args` 每行 `key=value` 追加 `--key value` / `extra_env` 每行 `KEY=VALUE` 注入 `-e`），仅非空/非默认值拼入命令。**参数权威文档：`https://github.com/eugr/spark-vllm-docker`（fork 的 README / recipes / examples）**——不要去查 `docs.vllm.ai`。设置页下拉选项必须与 fork 当前支持的能力完全一致，新增/变更参数前先看 fork 的最新 recipe 与 mod 列表。
- **多机集群硬规则**（对齐 fork `launch-cluster.sh`）：
  - **不要** 在 `vllm serve` 命令里手动加 `--distributed-executor-backend`、`--nnodes`、`--node-rank`、`--master-addr`、`--master-port`、`--headless`——本应用 `start_multi_node` 已在命令尾部追加这些参数；用户把它们写进 `vllm_flags` / `extra_args` 反而会被后追加的同名参数覆盖导致行为异常（vLLM 后值生效）。
  - fork 多机为 no-Ray 后端（`--distributed-executor-backend mp` + worker `--headless`）；本应用严格走 fork no-Ray 模式，**不要用 `--distributed-executor-backend ray`**（镜像校验 nnodes>1 时 ray 直接崩）。
  - `--tensor-parallel-size / -tp` 由 launch-cluster 自动从 active 节点数派生；本应用多机 TP 固定等于 `n_nodes`，不要让用户在 `vllm_flags` 里覆盖。
- **模型清单 `vllm_flags`**：`RemoteModel` 可带 fork 官方推荐启动参数数组（每条 `--key value` 或 `--flag`，如 `--speculative-config {...}` JSON 配置），启动时最后追加、优先级最高（可覆盖设置页同名参数，vLLM 后值生效），经前端 `start_model` 的 `vllmFlags` 透传。示例见 `doc/model.json`（DeepSeek-V4-Flash DGX Spark 配方）。
- **硬件优先级**：`hwinfo` 插件数据覆盖 `sysinfo`。
- **更新流程**：启动后延迟 3 秒 → 应用更新（不再有 llamacpp / VC++ 运行库流程）。
- **窗口关闭**：`cleanup_processes`（托盘退出 / 窗口关闭 / `ExitRequested` 统一入口，幂等）停止 vLLM 容器（`docker stop/rm`，单机容器名 `adm-vllm-<model>`、多机容器名 `adm-vllm-<model>-rank-<R>`）+ 杀 sd-cli 残留兜底。
- **Windows**：`main.rs` 中的 `#![windows_subsystem = "windows"]` + `build.rs` 中的 `/SUBSYSTEM:WINDOWS` 隐藏控制台。
- **调试日志**：`dbg_log!` 宏（`src-tauri/src/common/utils/log.rs`）仅 debug 构建输出到 stderr，release 构建编译为空。

## 构建与发布
- CI：`.github/workflows/build.yml` — 标签触发（`v*`），矩阵构建 3 个平台：
  - Windows x64（`x64-setup.exe` / NSIS）
  - Ubuntu x64 与 arm64（各 `.deb`；arm64 用 `ubuntu-24.04-arm` 原生 runner）
  - Linux 步骤先安装 Tauri 2 系统依赖（webkit2gtk-4.1、ayatana-appindicator3、librsvg2 等）；Windows 自签名，Linux 无签名步骤。
  - deb 打包必需 `bundle.category` / `shortDescription` 字段（已配于 `tauri.conf.json`）。
- 发布：`pnpm tauri:build:<平台>` 然后 `pnpm sign:<平台>`。
- 图标：`python scripts/generate-icons.py` 从 `src-tauri/icons/source.png` 生成。

## 注意事项
- vLLM 部署流程完整文档：`doc/vllm-deployment.md`（架构 / 前置条件 / 镜像策略 / 模型清单格式 / 下载 / 启动命令 / 参数表 / 排查）。当新的功能发生变化时候即时更新文档
- 多机互联（2+ 台 DGX Spark 集群）：v1 已实现（设置页「多机互联」Tab）。**严格对齐 fork `launch-cluster.sh` 的 no-Ray 模式**：关键注意：总开关 + 节点清单（首条必须本机）；多机容器必须 `--network host`（无 `-p`）；容器名 `adm-vllm-<model>-rank-<R>`（据此识别多机停止）；head + worker 容器均跑 `sleep infinity`（fork 的 keepalive），**启动顺序对齐 fork `exec_no_ray_cluster`**——容器全部就绪后，先向每个 worker 派发 `docker exec -d <container> bash -c "vllm serve ... --distributed-executor-backend mp --nnodes N --node-rank R --tensor-parallel-size N --master-addr <head_ip> --master-port <port> --headless"`（后台，日志落盘 `/tmp/adm_vllm_<model>_rank_<R>.log`），最后 head 起完整 `vllm serve ...`（同参数，node-rank 0、无 `--headless`）并等待所有 rank join；**绝对不能用 `--distributed-executor-backend ray`**（fork 镜像 pydantic 校验 `nnodes > 1` 只允许 mp/uni/external_launcher，ray 直接 ValidationError 崩掉）。master 端口（dist_init_port）默认 9090，不得与模型服务端口冲突。细节见 `doc/vllm-deployment.md` 与 `doc/dgx-spark-multinode-plan.md`
- 模型列表远端配置 `https://adm.tuduoduo.top/b/model.json`（本地示例 `doc/model.json`）：新格式 `model_download_files` + `model_support_devices` + `vllm_image` + `vllm_flags`。
