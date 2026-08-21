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
- **推理引擎 = SGLang（Docker 部署）**：llama.cpp 方案已完全移除。模型以 **HF safetensors 目录**（`models/<model_id>/`，含 `.done` 完成标记）下载，`start_model` 检测目录模型后走 `start_sglang_docker`：`docker run --gpus all --shm-size <配置> -p <port>:<port> -v <模型目录>:/models/<model_id>:ro lmsysorg/sglang:v0.5.17 python3 -m sglang.launch_server ...`，就绪信号为 stdout `Uvicorn running on`。容器内 host 固定 `0.0.0.0`（端口映射必需），端口沿用设置页。停止/退出走 `docker stop/rm` + 杀进程兜底。完整流程见 `doc/sglang-deployment.md`。
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
| `model_list.rs` | `fetch_model_list`, `scan_local_models`, `scan_part_files`, `download_model`（多文件目录下载）, `start_model`（目录模型 → `start_sglang_docker`）, `stop_model`（docker stop/rm）, `delete_local_model` |
| `settings.rs` | `save_settings`（直接写入 + `sync_all` 刷盘，持久化 `sglang_args`）, `load_settings`, `get_app_version` |
| `model_image.rs` | `check_sd_exists`, `download_and_extract_sd`, `start_sd_generation`, `stop_sd`, `save_sd_image_as` |

## 关键注意事项
- **SGLang 镜像固定 tag**：默认 `lmsysorg/sglang:v0.5.17`（v0.5.14+ 支持 GB10；多架构含 arm64 适配 DGX Spark）。`latest` 可变 tag 不用于生产；旧 `spark` tag 已过时。升级只改 `config.json` 的 `sglang_args.image`。
- **NVFP4 在 DGX Spark（GB10）有已知 CUDA 崩溃问题**：优先使用 FP8 权重模型；NVFP4 模型在 Spark 上异常时换 FP8。
- **模型下载（多文件目录）**：`model_download_files` 清单逐文件 `.part` 续传，下载完成写 `.done`；所有 `huggingface.co` 自动替换为 `hf-mirror.com`。
- **SGLang 参数**：设置页「模型启动参数」面板配置 `sglang_args`（image/shm_size/tp/mem-fraction-static/dtype/quantization/kv-cache-dtype/schedule-policy/请求数/chunked-prefill/log 等 + `extra_args` 每行 `key=value` 追加 `--key value`），仅非空/非默认值拼入命令。**官方参数文档（权威，改参数前必查）：`https://docs.sglang.io/docs/advanced_features/server_arguments`**；推测解码文档：`https://docs.sglang.io/docs/advanced_features/speculative_decoding`；DeepSeek MTP 用法：`https://docs.sglang.io/basic_usage/deepseek_v3.html`。设置页下拉选项必须与文档支持的枚举值完全一致，新增/变更参数前先查文档再改代码。
- **MTP 自动启用**：模型目录含 MTP 权重（`model_mtp.safetensors` 等，文件名含 `mtp` 且为 `.safetensors`/`.bin`）时自动追加 `--speculative-algorithm EAGLE --speculative-num-steps 3 --speculative-eagle-topk 1 --speculative-num-draft-tokens 4`（SGLang 中 NEXTN 是 EAGLE 别名，MTP 权重同目录自动加载，无需 `--speculative-draft-model-path`）；`extra_args` 已自定义 `speculative-algorithm` 则跳过。实现位置：`model_list.rs` `start_sglang_docker`。
- **硬件优先级**：`hwinfo` 插件数据覆盖 `sysinfo`。
- **更新流程**：启动后延迟 3 秒 → 应用更新（不再有 llamacpp / VC++ 运行库流程）。
- **窗口关闭**：`cleanup_processes`（托盘退出 / 窗口关闭 / `ExitRequested` 统一入口，幂等）停止 SGLang 容器（`docker stop/rm`）+ 杀 llama-server/sd-cli 残留兜底。
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
- SGLang 部署流程完整文档：`doc/sglang-deployment.md`（架构 / 前置条件 / 镜像策略 / 模型清单格式 / 下载 / 启动命令 / 参数表 / 排查）。
- 模型列表远端配置 `https://adm.tuduoduo.top/b/model.json`（本地示例 `doc/model_list.json`）：新格式 `model_download_files` + `model_support_devices`。
- **已移除功能**：llama.cpp 方案（llamacpp 下载/版本/删除命令、llama-server 启动分支、VC++ 运行库与 LLamaCPP 更新弹窗）、Agent 聊天页（含 admAgent server 集成、多 workspace、微信 Bot/iLink、技能管理）。相关代码均已删除，不要再按旧文档引用。
- **项目结构**：
  - `src/` + `src-tauri/` — Tauri 桌面端（vanilla JS 前端 + Rust 后端）
  - `website/` — 营销网站
  - `scripts/` — 工具脚本（图标生成 / 签名）