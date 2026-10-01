<div align="center">

# ADM-BE

**Automatic Deployment Model — DGX Spark 专用的大模型一键部署与管理桌面应用**

把 vLLM / SGLang 复杂的容器启动指令做成图形界面：选模型 → 点下载 → 点启动，全程断点续传、国内镜像加速、集群一键直连部署，让 GB10（DGX Spark / 128G 统一内存）的推理部署从「敲命令调参数」变成「点几下」。

![Tauri](https://img.shields.io/badge/Tauri-2.11.2-FFC131?style=flat-square&logo=tauri)
![Rust](https://img.shields.io/badge/Rust-2021_edition-000000?style=flat-square&logo=rust)
![Docker](https://img.shields.io/badge/Engine-vLLM%20%7C%20SGLang%20%7C%20ComfyUI-2496ED?style=flat-square&logo=docker)
![Platform](https://img.shields.io/badge/Platform-Windows%20%7C%20Linux-blue?style=flat-square)
![License](https://img.shields.io/badge/License-MIT-green?style=flat-square)

**📬 联系方式**：项目地址 https://github.com/autoDeploymentModel/adm ｜ 问题反馈 [GitHub Issues](https://github.com/autoDeploymentModel/adm/issues) ｜ 讨论交流 欢迎扫码添加微信

<img src="src-tauri/wx.png" alt="微信" width="180" />

</div>

---

## 这是什么

ADM-BE 是一个 **DGX Spark（GB10 / sm_121）专用**的本地大模型部署工具。所有引擎都跑在 Docker 容器里，模型以 HF safetensors 目录落盘到本机数据目录，镜像与启动参数由远程模型清单统一指定：

- **双推理引擎** — vLLM（默认，走 `eugr/spark-vllm` fork）与 SGLang，按模型清单 `engine` 字段自动分发，同一套图形界面管理
- **一键下载** — 整仓（`hfd.sh`）或逐文件两种清单模式，自动切换 hf-mirror 加速，支持 `.part` / aria2 断点续传与实时速度
- **镜像自动准备** — 模型下完自动拉取引擎镜像（已存在则跳过），启动时只做环境预检，不在启动路径上拉镜像
- **双机 / 多机集群** — DGX 双机光口直连一键部署（免密 SSH → 光口探测 → 固定 IP → 连通性 → Docker → 写回节点表），严格对齐 DGX Spark no-Ray 多机启动流程
- **视频生成** — MiniMax-H3（ComfyUI）单节点部署：镜像与权重分开准备、互斥下载、状态可视，直出 H3 视频
- **内置测试台** — 内嵌 llama-ui 静态页面，经本机兼容层代理到运行中的模型，直接测对话、看 prefill / decode 速度
- **启动即预热** — 引擎就绪后自动跑一组长短不一的请求，把首轮 JIT 编译前移，消除「启动后第一轮卡顿」
- **统一日志** — 模型 / 容器输出按 `[时间] [级别] [标签] 内容` 落盘，设置页「运行日志」可查

轻量高效：Tauri 2 壳 + 原生 HTML/CSS/JS 前端（无框架、无打包工具），Rust 后端负责下载、容器编排、SSH 与本地服务。

---

## 支持的模型

模型清单来自远程 `model.json`，当前线上共 7 个条目（**均为 `dgx-spark-128G` 适配**，下载完自动拉取引擎镜像，点启动即部署）：

| 模型 | 引擎 | 模态 | 体积 | 说明 |
| --- | --- | --- | --- | --- |
| **DeepSeek-V4-Flash-0731** | vLLM（B12x 定制） | 纯文本 | 167 GB | NVFP4 量化 + DSpark 投机解码（5 token），支持对话 / 推理 / 工具调用 |
| **DeepSeek-V4-Flash-0731-CRACK** | vLLM（B12x 定制） | 纯文本 | 167 GB | 同上模型的社区去对齐（abliterated）权重 |
| **GLM-5.3-Flash** | vLLM（EXL3 补丁镜像） | 多模态 | 164 GB | EXL3/TR3 4bpw + MTP-2 投机解码，850K 上下文、fp8 KV；**需 2 台 DGX Spark 组成 TP2 集群** |
| **Qwen3.8-Flash-Next** | vLLM（PLE 补丁镜像） | 多模态 | 133 GB | NVFP4 量化 + MTP-3，600K 上下文（`PLE_FORCE_FP8=1` 必需） |
| **Qwen3.8-Flash-Next-Uncensored** | vLLM（PLE 补丁镜像） | 多模态 | 124 GB | 去对齐版，PLE n-gram 表 FP8 量化，比 RadixArk 版小 47 GiB |
| **Ling-3.0-flash-VL-fp8** | **SGLang** | 多模态 | 124 GB | 124B 总参 / 5.5B 激活，262K 上下文；**需多机部署（2 节点）** |
| **MiniMax-H3-ComfyUI** | **ComfyUI** | 视频生成 | ≈67 GB | NVFP4-AWQ 编码器 + int8 剪枝 DiT（FL2VA / Ref2VA），原生音视频 768p/24fps + 立体声；在「视频生成」页准备与启动，不在模型列表显示 |

对话类模型均支持工具调用与思维链；标注「多模态」的额外支持图像输入。参数随镜像走，不同架构（B12x MLA / NoPE-MLA / qwen3.5-moe 系）的内核与参数**不可跨架构照抄**，具体配方由清单内 `vllm_flags` / `vllm_env` 固定。

> 清单随服务端更新，本表为写入时快照；以应用内「模型列表」实际显示为准。

---

## 界面导航

| 页面 | 路由 | 作用 |
| --- | --- | --- |
| 模型列表 | `#/list` | 适配机型过滤、下载/启动/停止模型、打开 WebUI |
| 视频生成 | `#/video` | MiniMax-H3（ComfyUI）镜像与权重准备、启停、打开 WebUI |
| 设置 | `#/settings` | 模型启动参数 / Docker 镜像配置 / 代理 / DGX直连配置 / 外观主题 / 运行日志 / 版本号 / 关于 |
| 测试 | `#/test` | 内嵌 llama-ui，对运行中的模型做对话与速度测试 |

---

## 核心概念

### 适配机型

首页顶部「适配机型」下拉，选项来自模型清单的 `model_support_devices`（当前为 `dgx-spark-128G`），选择随启动命令传入后端。空数组表示全机型可用。

### 模型清单（远程配置）

模型列表来自远程 `model.json`，本地示例见 `doc/model.json`。除下载清单外，每个模型可携带：

| 字段 | 说明 |
| --- | --- |
| `model_download_files` | 统一下载清单，**仓库条目与完整文件 URL 不可混用** |
| `model_support_devices` | 支持的机型列表 |
| `engine` | `vllm`（缺省）/ `sglang` / `comfyui` |
| `engine_image` | 引擎镜像，为空回退 `vllm_image` |
| `vllm_flags` / `vllm_env` / `vllm_extra_mounts` | 模型专属启动参数 / 环境变量 / 附加挂载，最后追加、优先级最高 |
| `engine_command` / `engine_ready_patterns` / `engine_ready_probe` | 自定义容器入口、就绪关键字、HTTP 就绪探针 |

完整字段与镜像策略见 `doc/vllm-deployment.md`。

### 启动参数

设置页「模型启动参数」配置 shm_size / tp / gpu-memory-utilization / quantization / kv-cache-dtype / load-format 等常用项，以及 `extra_args`（每行 `key=value`）与 `extra_env`（每行 `KEY=VALUE`）兜底。SGLang 只映射安全子集，其余配方交给清单里的 `vllm_flags`。

> ⚠️ 镜像来源决定参数权威：spark-arena/eugr 定制镜像以 [eugr/spark-vllm-docker](https://github.com/eugr/spark-vllm-docker) 为准，官方 vLLM 镜像以 [docs.vllm.ai](https://docs.vllm.ai) 与镜像内 `vllm serve --help` 为准。**多机模式不要手动加 `--distributed-executor-backend` / `--nnodes` / `--node-rank` / `--master-addr`**，这些由应用自动追加，且必须走 no-Ray 后端。

---

## 开发与构建

### 环境准备

| 依赖 | 版本 | 说明 |
| --- | --- | --- |
| [Rust](https://www.rust-lang.org/tools/install) | stable | Tauri 后端编译 |
| [Node.js](https://nodejs.org/) | 22+ | 前端工具链 |
| [pnpm](https://pnpm.io/) | 9+ | 包管理器（**不要使用 npm / yarn**） |
| Docker | 任意近期版本 | 全部推理引擎以容器运行 |

> Windows 还需安装 [Visual Studio C++ 生成工具](https://visualstudio.microsoft.com/visual-cpp-build-tools/)（MSVC）；Linux 需 Tauri 系统依赖（webkit2gtk-4.1、ayatana-appindicator3、librsvg2 等）。

### 开发调试

```bash
pnpm install
pnpm tauri:dev        # 热重载开发模式
pnpm typecheck        # 前端类型检查（tsc --noEmit）
```

前端为原生 HTML/CSS/JS（无框架、无打包工具），源码在 `src/`，视图为独立 ES 模块（`src/views/*.js`）；Rust 后端在 `src-tauri/src/`。

### 生产构建

```bash
pnpm tauri:build            # 当前平台
pnpm tauri:build:windows    # Windows x64（NSIS 安装包）
pnpm tauri:build:linux      # Linux x64（deb）
pnpm release:windows        # Windows 构建 + 自签名一条龙
```

产物位于 `src-tauri/target/<target>/release/bundle/`。

### CI 自动发布

`.github/workflows/build.yml`：推送 `v*` 标签即构建并发布 GitHub Release —— Windows x64（NSIS，自签名）、Ubuntu x64 与 arm64（各 `.deb`）。

```bash
git tag v1.0.7
git push origin v1.0.7
```

---

## 文档

| 文档 | 内容 |
| --- | --- |
| `AGENTS.md` | 架构与全部技术决策的单一真源 |
| `doc/vllm-deployment.md` | vLLM / SGLang 部署全流程、参数表、排查 |
| `doc/dgx-direct-zero-deploy-plan.md` | DGX 双机直连一键部署方案 |
| `doc/minimax-h3-single-node-deploy-plan.md` | MiniMax-H3（ComfyUI）单机视频生成方案 |
| `doc/model.json` | 模型清单示例 |

---

## 许可证

本项目基于 **MIT 许可证** 开源。

```
MIT License

Copyright (c) 2026 ADM

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```
