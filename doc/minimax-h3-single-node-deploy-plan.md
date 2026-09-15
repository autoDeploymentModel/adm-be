# MiniMax-H3 单机部署方案（DGX Spark GB10 · ARM64 · Docker + ComfyUI 主线 / SGLang 备选）

> 目标机型：DGX Spark（GB10，128 GB 统一内存，ARM64 + CUDA 13，sm121）
> **主方案**：Docker + ComfyUI，权重用 **NVFP4-AWQ 编码器 + 量化 DiT**，**首次打开由 ComfyUI 自动下载**
> **入口**：底部导航**「视频生成」Tab** —— 镜像构建、权重下载、启动/停止/状态/日志/首次指引全部在该页内（首页模型列表**不出现** ComfyUI 卡片）
> **备选**：需要 `/v1/videos` 生产 API 时启用 SGLang（附录 C）；两者**内存互斥**，不可同时运行
> 状态：本方案为**设计稿**；附录 A 列出的 ADM-BE 改动（`engine: comfyui`、视频生成页）**尚未实现**

---

## 1. 交付结论（先看这段）

| 项目 | 结论 |
|---|---|
| 入口 | **底部导航「视频生成」页**（新增 Tab，路由 `#/video`）：服务状态 + 启动 / 停止 / 重启 / 打开 WebUI / 日志 / 首次指引；数据来源于远程 `model.json` 的 ComfyUI 条目（见 §5、附录 B） |
| 引擎 | ComfyUI 自 **v0.30.0** 起**原生支持 MiniMax-H3**（核心节点 + 官方模板，无需第三方插件），推理全程在 ComfyUI 内完成 |
| 权重获取 | **应用内一键下载**（统一下载清单 `model_download_files` 里 `Comfy-Org/MiniMax-H3/<路径或通配>` 条目，默认 FL2VA+Ref2VA 量化集 **≈67 GB**，走 hf-mirror/代理、断点续传、`.done` 标记）；亦可在 ComfyUI 首次打开模板时用弹窗自助下载（官方默认集 ≈44 GB，仅 FL2VA）；离线预置脚本见 §4.2 |
| 权重组合 | 文本编码器 `qwen3vl_32b_minimax_h3_nvfp4_awq`（15.69 GB）+ DiT `minimax_h3_fl2va_pruned_int8_convrot`（20.97 GB）+ 双 VAE（5.82 GB）+ turbo 8step LoRA（1.96 GB）；NVFP4 DiT 官方**不存在**（社区 w4a8 见 §7） |
| 输出能力 | 768p / 24 FPS / 4–15 s，**H.264 视频 + 32 kHz 立体声音频**（原生音视频同出） |
| 磁盘 / 内存 | ≈67 GB（FL2VA+Ref2VA 量化集；仅 FL2VA ≈44 GB，vs SGLang BF16 288 GB）/ 驻留 ≈44 GB（vs SGLang ≈108 GB） |
| 服务化 | ComfyUI HTTP API：`POST /prompt` → `GET /history/{id}` → `GET /view`；就绪探针 `GET /system_stats`（§8） |
| 离线 | 镜像本地构建；离线场景用预置脚本 + `HF_HUB_OFFLINE=1` + `--disable-api-nodes`（§10） |
| 多模态 IR | 本地开源部分 = H3-Base（FL2VA 文/首尾帧、Ref2VA 图/视频/音频参考）；云端 `H3-Context-IR` / `H3-Regenerate-2K` 未开源 → 全离线 768p 上限，用官方三段式提示词替代 |

### 1.1 形态选择与互斥

| 形态 | 说明 |
|---|---|
| **只部署 ComfyUI（本方案）** | 底部「视频生成」页启动/停止；不需要 SGLang 镜像、288 GB HF 权重、模型列表条目；权重由 ComfyUI 自动下载 |
| 追加 SGLang（附录 C） | 需要 `/v1/videos` 异步任务 API / BF16 50 步 lossless 时启用，仍以首页模型卡片形式存在 |
| 两者并存 | 必须互斥切换（≈44 GB + ≈108 GB > 121 GiB 可用内存）：「视频生成」页启动时提示停止 SGLang 模型，或 `h3-switch.sh comfyui|sglang` |

---

## 2. 架构与数据流

```
┌────────────────────────── ADM-BE（DGX Spark 本机） ──────────────────────────┐
│ 底部导航：首页 │ 视频生成 │ 测试 │ 设置                                       │
│      │                                                                       │
│      └─「视频生成」页 #/video（src/views/video.js）                           │
│           状态卡 ◀── model-started / model-stopped / model-log 事件           │
│           启动/停止 ──▶ start_model / stop_model（engine=comfyui 分支）       │
│                           │                                                   │
│                           ▼                                                   │
│              docker run … adm-comfyui-<model_id>（示例 …-MiniMax-H3-ComfyUI） │
│              镜像 adm-comfyui-h3:nvfp4-20260915（本地构建，comfy-kitchen[cublas]）│
│              命令 python3 main.py --listen 0.0.0.0 --port 8188               │
│                          --disable-auto-launch --disable-api-nodes           │
│              挂载 <data>/comfyui/{models,output,input,user} + media:ro        │
│              就绪探活 GET /system_stats == 200 → model-started               │
│           打开 WebUI ──▶ 系统浏览器 http://127.0.0.1:8188                    │
│           停止 ──▶ docker stop/rm adm-comfyui-h3（纳入 cleanup_processes）    │
└──────────────────────────────────────────────────────────────────────────────┘
                                    │
            浏览器 / API 客户端 ─────┴─► ComfyUI 容器（127.0.0.1:8188）
                                          ├─ 首次打开模板：弹窗自动下载权重 ≈44GB
                                          ├─ 工作流：模板库 → Video → MiniMax H3 T2V/I2V/R2V
                                          └─ API：/prompt /history /view /upload/image /system_stats
```

首页模型列表保持现状（只渲染 vLLM / SGLang 模型卡片）；ComfyUI 条目带 `hidden_in_list: true`，仅作为「视频生成」页的配置来源与后端引擎入口。

---

## 3. 前置条件

| 项 | 要求 | 说明 |
|---|---|---|
| 机型 | DGX Spark / GB10（`model_support_devices: ["dgx-spark-128G"]`） | 128 GB 统一内存，Blackwell（NVFP4 硬件前提） |
| 系统 | ARM64 Linux + NVIDIA 驱动 + CUDA 13（DGX OS） | |
| Docker | Docker Engine + NVIDIA Container Toolkit（`--gpus all` 可用） | 应用启动前已有 CLI/daemon/GPU 预检 |
| 磁盘 | 权重 44 GB + 镜像 ~25 GB + 输出/素材；建议 ≥1 TB NVMe | 比 SGLang 方案（288 GB）宽裕得多 |
| 网络 | 首次准备需联网：应用内下载权重（hf-mirror/代理，推荐）或 ComfyUI 弹窗直连 huggingface.co；镜像需联网构建一次；纯离线见 §4.2 路径 C / §10 | |
| 端口 | 8188（ComfyUI 默认，页面内可配置），不与 SGLang 服务端口（默认 8000）冲突 | 两者不同时运行 |

---

## 4. 一次性准备

### 4.1 构建 ComfyUI ARM64 镜像（离线关键）

**一键脚本（推荐；也可在另一台设备构建后拷回 DGX）**：

```bash
./scripts/docker/h3-comfyui/build-image.sh --save
# 常用参数：
#   --dry-run                      只打印解析结果与将执行的命令，不构建
#   --proxy http://127.0.0.1:1080  构建期代理（本机代理会自动加 --network=host）
#   --pip-index https://pypi.tuna.tsinghua.edu.cn/simple
#   --platform linux/arm64         默认；x86 宿主跨架构需 binfmt/QEMU（脚本会提示安装命令）
#   --force / --no-verify / --skip-runtime-check / --tag <image:tag> / --base <image> / --ref <tag>
# 脚本自动完成：基础镜像源探测（官方 → 加速器）、ComfyUI 源码源回退 + commit 校验、代理注入、
# 构建后自检；--save 额外导出 tar.gz + sha256。拷到 DGX 后导入：
#   gunzip -c adm-comfyui-h3_nvfp4-20260915-linux_arm64.tar.gz | docker load
```

> **在 x86 宿主上构建（Ubuntu 22.04 / WSL2 / x86 Docker Desktop）**：目标 `linux/arm64` 属**跨架构构建**，需 QEMU/binfmt——
> ① 一次性注册模拟器：`sudo docker run --privileged --rm tonistiigi/binfmt --install arm64`（Docker Desktop 自带，可跳过）；验证：`docker run --rm --platform linux/arm64 alpine uname -m` → `aarch64`；
> ② Ubuntu 22.04 发行版自带 `docker.io` 是 20.10（classic builder），脚本会自动补 `DOCKER_BUILDKIT=1`；更省事是装 Docker CE 官方源（23+）；
> ③ 直接跑 `./scripts/docker/h3-comfyui/build-image.sh --save`（自动加 `--platform linux/arm64`）——QEMU 模拟下预计 **1–3 小时**，磁盘预留 ≈30 GB；
> ④ 若镜像内自检在模拟下异常，加 `--skip-runtime-check` 跳过，到 DGX 实机再验。

手工等价命令：

```bash
docker build -t adm-comfyui-h3:nvfp4-20260915 scripts/docker/h3-comfyui
# --build-arg COMFYUI_REF=v0.30.0 （H3 需 ≥0.30.0；Fun ControlNet 模板需 ≥0.35.0）
# --build-arg BASE=nvidia/cuda:13.0.0-runtime-ubuntu24.04   # Docker Hub 不可达时用加速器前缀（如 docker.1ms.run/nvidia/cuda:...）
# --build-arg PIP_INDEX_URL=https://pypi.tuna.tsinghua.edu.cn/simple   # 可选：PyPI 源（应用内构建会按吞吐自动择优；留空 = 官方）
# --build-arg TORCH_INDEX_URL=... / COMFYUI_REPO=...                   # 可选：wheel 源 / ComfyUI 源码仓库
# --build-arg SKIP_RUNTIME_CHECK=1                                    # 可选：跳过镜像内自检（QEMU 跨架构构建时用）
# 代理：HTTP_PROXY / HTTPS_PROXY / ALL_PROXY / NO_PROXY（应用「设置 → 代理」开启后自动注入）
```

> **基础镜像源自动探测（应用内构建）**：BuildKit 解析 `FROM`（含到 `auth.docker.io` 取匿名 token）**由 daemon 发起**——既不读 daemon.json 的 `registry-mirrors`，也不吃客户端代理；国内网络会直接卡在 `registry-1.docker.io / auth.docker.io … i/o timeout`。应用「构建镜像」会先用 `docker manifest inspect`（**不带客户端代理**，与 daemon 侧一致）逐个探测候选源（已配置加速器 → 官方源 → 内置加速器），把可达的那个前缀写进 `BASE`（页内日志显示实际使用的源）；「设置 → 代理」只注入构建内 RUN 步骤（apt/pip/git，本机代理自动加 `--network=host`），想让 daemon 也走代理需在该 Tab 点「保存并重启 Docker」（写 daemon.json `proxies`）。
>
> **源码源自动回退（同类问题）**：`github.com` 在部分网络被 TLS 重置（`GnuTLS recv error (-110)`）；应用会探测 `…/info/refs?service=git-upload-pack` 把可达的 git 源排到最前（官方 → gitee 镜像 → gitcode 镜像 → ghfast / gh-proxy 加速器），Dockerfile 内逐个尝试；克隆完成后用 `COMFYUI_REF_SHA` 校验 commit（`v0.30.0` = `b1693ecb…`，四个备用源与上游一致），不一致直接失败——被篡改/滞后的第三方镜像会被挡住。
>
> **PyPI 源自动择优（pip 层）**：`pypi.org` 的索引页本身很快，但**未托管在 PyTorch 索引的包**（`cuda-bindings`、`cuda-pathfinder`…）会被官方索引回落到 `files.pythonhosted.org`，弱网下长时间停读（实测 8.7 KB/s → `ReadTimeoutError`，整层构建失败，前面的 apt/torch 层白跑）。应用「构建镜像」会实测官方 + 清华 / 阿里 / 腾讯 / 华为云镜像的吞吐（GET `<源>/pip/` 索引页取最快）并把结果写进 `PIP_INDEX_URL`（页内日志显示实测 KB/s）；`torch / torchvision / torchaudio` 仍固定从 `TORCH_INDEX_URL` 取（`+cu130` 本地版本标记在同版本下优先于镜像里的普通轮子，已实测确认），其**依赖**则走选出的 PyPI 源。Dockerfile 侧：所有 pip 安装带 `--timeout 60 --retries 10`；requirements 与 `comfy-kitchen[cublas]` 主源失败会自动回退官方 PyPI；torch 装完校验 `torch.version.cuda` 大版本（与 `TORCH_INDEX_URL` 的 `cuNNN` 对齐），不符则从官方索引重装、仍不符直接失败——避免静默装出 cu12x 轮子。三个 pip 层还带 `--mount=type=cache,target=/root/.cache/pip`（wheel 存 BuildKit 缓存、不进镜像层：弱网失败后重试不必重下 2 GB+），该语法要求 BuildKit——Docker 23+ 默认即是，应用与 `build-image.sh` 都强制 `DOCKER_BUILDKIT=1`。

镜像要点：

| 项 | 要求 | 原因 |
|---|---|---|
| PyTorch | cu130（aarch64 原生 wheel） | GB10 / CUDA 13 |
| ComfyUI | `COMFYUI_REF` 固定 tag | H3 支持自 v0.30.0 起 |
| `comfy-kitchen` | **`pip install "comfy-kitchen[cublas]"`** | NVFP4（Blackwell）与 int8_convrot 内核；plain wheel 不带 cublas 路径 |
| 构建期校验 | `torch.cuda.is_available()` 且 `comfy_kitchen.list_backends()` 含 `cuda` | 避免"装上了但量化跑不了" |
| 其他 | ffmpeg、libgl1、libglib2.0-0、libsndfile1 | H3 音视频输出 |

> ARM64 风险（务必现场验证）：`comfy-kitchen` 自 0.2.10 起提供 `manylinux_2_28_aarch64` CUDA wheel，但 **sm121 内核覆盖需实测**；不可用时按 §12 回退（fp8_scaled / bf16 / 换基础镜像）。

### 4.2 镜像与权重获取（三种路径，页面内一键）

| 路径 | 适用 | 做法 | 说明 |
|---|---|---|---|
| **A. 应用内一键（默认）** | 有外网/镜像加速 | 「视频生成」页 → 「构建镜像」（应用写入内置 Dockerfile 到 `<data>/build/comfyui/` 并执行 `docker build -t <engine_image>`，日志流式进入页内日志区）→ 「下载权重」（`model_download_files` 中的 `Comfy-Org/MiniMax-H3/<路径或通配>` 条目 ≈67 GB，支持断点续传/停止） | 与模型下载链路同源（hf-mirror/代理、`.done` 标记）；落盘到 `<data>/models/MiniMax-H3-ComfyUI/{diffusion_models,text_encoders,vae,loras,embeddings}` 并整体挂载为 `/opt/ComfyUI/models` |
| B. ComfyUI 弹窗自助 | 首次只想跑 FL2VA | 启动容器 → 打开 WebUI → 打开官方模板 → 跟随弹窗下载（≈44 GB） | 直连 huggingface.co（受限时改用路径 A）；与 A 落盘目录相同，可混用/续下。⚠️ 清单默认带 `--disable-api-nodes`（禁云 API/前端外联），若该参数导致弹窗下载不可用，可临时从 `engine_command` 去掉它或直接用路径 A |
| C. 离线预置 | 纯离线/内网 | `./scripts/docker/h3-comfyui/download-comfy-h3.sh minimal|ref2va|all`（默认 `<data>/models/MiniMax-H3-ComfyUI`，`ADM_COMFYUI_WEIGHTS` 可覆盖）+ `HF_HUB_OFFLINE=1` | 与 A 目录一致；镜像用 `docker build`（或在有网机器构建后 `docker save/load` 导入） |

目录约定（三种路径共用，挂载进容器 `/opt/ComfyUI/models`）：

```
<data>/models/MiniMax-H3-ComfyUI/        → /opt/ComfyUI/models（读写）
├── diffusion_models/   # FL2VA/Ref2VA int8_convrot 剪枝 DiT（各 20.97 GB）
├── text_encoders/      # NVFP4-AWQ 编码器（15.69 GB）
├── vae/                # video fp16（5.21 GB）+ audio fp32（0.61 GB）
├── loras/              # turbo 8step（1.96 GB，官方模板扫描必需）+ ref2v 4step
└── embeddings/         # 风格 embedding（可选，每个 <2 MB）
<data>/comfyui/output/  → /opt/ComfyUI/output（产物）
<data>/comfyui/input/   → /opt/ComfyUI/input（参考素材上传）
<data>/comfyui/user/    → /opt/ComfyUI/user（界面设置/工作流）
<data>/build/comfyui/   # 应用写入的 Dockerfile（构建目录）
```

## 5. 「视频生成」页（底部导航新增 Tab，设计稿）

### 5.1 导航与路由（壳层改动）

| 改动 | 内容 |
|---|---|
| 底部导航 | `src/index.html` 的 `#hardware-bar` 内新增按钮 `id="video-btn"`（文本「视频生成」，`onclick="location.hash='#/video'"`）；建议顺序：`首页 │ 视频生成 │ 测试 │ 设置`，硬件信息仍在右侧 |
| 路由表 | `routes` 增加 `"/video": { load: () => import("./views/video.js"), nav: "video-btn" }` |
| 视图模块 | 新增 `src/views/video.js`，默认导出 `{ template, mount(root, params), unmount(), handleTauriEvent(type, payload) }`；遵守既有约定（样式加 `video-*` 前缀、不重复全局 reset、不注册全局事件、状态读写走 `window.__adm_state`） |
| 首页 | 模型列表**不渲染** ComfyUI 条目（`hidden_in_list: true`）；其余模型卡片行为不变 |
| i18n | `src/i18n.js` 增补本页词条（标题、按钮、状态、指引文案） |

### 5.2 页面布局（wireframe）

```
┌ 视频生成 ───────────────────────────────────────────────────────────────┐
│ ┌ 状态卡 ─────────────────────────────────────────────────────────────┐ │
│ │ ● 运行中       容器 adm-comfyui-<model_id>  镜像 adm-comfyui-h3:nvfp4-… │ │
│ │   地址 http://127.0.0.1:8188      端口 [8188]      运行时长 00:12:31 │ │
│ └─────────────────────────────────────────────────────────────────────┘ │
│ [ 启动 ComfyUI ]  [ 停止 ]  [ 重启 ]        [ 打开 WebUI ]  [ 复制产物路径 ]│
│ ┌ 环境准备（首次使用）─────────────────────────────────────────────────┐ │
│ │ 镜像：未构建 / 已构建 / 构建中…              [ 构建镜像 ]              │ │
│ │ 权重：未下载（首次约 67 GB）/ 下载中 42% / 已下载（xx.x GB） [ 下载权重/停止下载 ] │
│ │ ▓▓▓▓▓▓▓░░░░░░░░░░ 进度条                                              │ │
│ │ 权重目录 <data>/models/MiniMax-H3-ComfyUI   构建目录 <data>/build/comfyui │ │
│ └──────────────────────────────────────────────────────────────────────┘ │
│                                                                         │
│ ┌ 首次使用指引（权重未就绪时高亮）────────────────────────────────────┐ │
│ │ 1. 点击「打开 WebUI」→ 模板库 → Video → MiniMax H3 T2V / I2V / R2V  │ │
│ │ 2. 跟随弹窗下载权重（NVFP4-AWQ 编码器 + 量化 DiT，≈44 GB，仅一次）   │ │
│ │ 3. 首次建议 480P / 5 s 跑通，再上 768p；低延迟可接 turbo 8step LoRA  │ │
│ │ 权重目录 <data>/models/MiniMax-H3-ComfyUI  产物 <data>/comfyui/output │ │
│ └─────────────────────────────────────────────────────────────────────┘ │
│ ┌ 互斥提示（仅当 SGLang 模型运行中时显示）────────────────────────────┐ │
│ │ 当前 SGLang 服务（108 GB）占用统一内存，启动 ComfyUI 前需先停止它    │ │
│ │ [ 停止 SGLang 并启动 ComfyUI ]                                       │ │
│ └─────────────────────────────────────────────────────────────────────┘ │
│ ┌ 日志（model-log 流式，限高滚动，可复制）───────────────────────────┐ │
│ └─────────────────────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────────────────┘
```

### 5.3 状态机与交互

```
未启动 ──点「启动 ComfyUI」──▶ 启动中（拉取/校验镜像 → 拉起容器 → 等待探活）
   │                                   │
   │                        GET /system_stats == 200
   │                          ┌── 成功 ──▶ 运行中（按钮：停止 / 重启 / 打开 WebUI）
   │                          └── 失败 ──▶ 启动失败（toast + 日志区高亮，按钮回「启动」）
   └── 若检测到其它模型运行中（SGLang）──▶ 显示互斥提示条（见 5.2 第三块）
```

页面行为约定：

1. **服务与页面解耦**：服务生命周期由后端持有；切到别的页面/关闭窗口不影响运行中的容器（应用退出由 `cleanup_processes` 统一清理）。
2. **状态恢复**：`mount()` 时调用 `get_model_status` + 读取 `window.__adm_state` 恢复状态卡（避免切页后误显示「未启动」）。
3. **事件驱动**：`handleTauriEvent` 处理 `model-started` / `model-stopped` / `model-log` / `model-pull-progress` 更新状态与日志；事件监听仍只在壳层注册。
4. **互斥（已实现）**：其它模型运行中时页内显示黄色互斥横幅（含「停止当前模型并启动 ComfyUI」按钮）；模型列表侧启动其它模型时若 ComfyUI 在运行，提示先去「视频生成」页停止。
5. **首次引导**：权重未就绪（`<data>/models/MiniMax-H3-ComfyUI` 下 diffusion_models/text_encoders 无关键文件）时高亮指引块；就绪后折叠为一行。
6. **端口**：默认 8188，可在页面内修改（改动写入设置；容器重启生效）。
7. **不做内嵌 WebUI**：ComfyUI 前端依赖 WebSocket/剪贴板，且未承诺允许被 iframe 嵌套 → 默认用系统浏览器打开（`window.openUrl`）；如需内嵌作为实验开关另议。
8. **端口持久化（已实现）**：ComfyUI 端口存前端 `localStorage`（键 `adm_comfyui_port`，默认 8188），随 `start_model` 的 `params.port` 下发；与设置页 SGLang 端口互不影响。
9. **产物路径**：页内「复制产物路径」按钮从挂载映射（`comfyui/output:/opt/ComfyUI/output`）推导宿主机路径并复制到剪贴板。
10. **启动前置（已实现）**：`comfyui_setup_status` 提供镜像/权重状态——镜像未构建时「启动 ComfyUI」禁用（tooltip「需先构建镜像」）；权重未下载不阻塞启动（ComfyUI 界面可用，可走弹窗自助下载）。
8. **测试页协同**：`support_video` 模型在「测试」页仍走 API 引导文案，可加「前往『视频生成』页」的跳转提示。

### 5.4 配置来源

- 页面在后端模型清单（`fetch_model_list` 结果 / `S().modelList`）中查找 `engine === "comfyui"` 的条目，读取：`engine_image`、`engine_command`、端口、`vllm_env`、`vllm_extra_mounts`、`model_type` 等。
- 未找到条目 → 页面显示「未配置 ComfyUI 条目（检查远程 model.json）」并禁用启动按钮。
- 条目示例见附录 B。

---

## 6. 启动与首次使用流程

```bash
# 应用内：底部「视频生成」页 → 状态卡变「运行中」
# 等价手工命令（排障用）：
docker run -d --name adm-comfyui-h3 --gpus all --ipc host --shm-size 16g \
  -p 127.0.0.1:8188:8188 \
  -v <data>/models/MiniMax-H3-ComfyUI:/opt/ComfyUI/models \
  -v <data>/comfyui/output:/opt/ComfyUI/output \
  -v <data>/comfyui/input:/opt/ComfyUI/input \
  -v <data>/comfyui/user:/opt/ComfyUI/user \
  -v <data>/media:/data/minimax-h3:ro \
  -e PYTORCH_CUDA_ALLOC_CONF=expandable_segments:True \
  adm-comfyui-h3:nvfp4-20260915 \
  python3 main.py --listen 0.0.0.0 --port 8188 --disable-auto-launch --disable-api-nodes
```

WebUI 内（首次）：

1. 模板库 → **Video → MiniMax H3 T2V / I2V / R2V**（或导入 `video_minimax_h3_t2v.json` / `_r2v.json`）
2. **跟随弹窗自动下载**（官方量化集，含 NVFP4-AWQ 编码器 + int8 剪枝 DiT + 双 VAE + turbo LoRA，≈44 GB）
3. 检查节点选择：UNET = `*_pruned_int8_convrot`、文本编码器 = `*_nvfp4_awq`、VAE = video/audio 各一
4. 首次出片建议 480P / 5 s；跑通后再上 768p
5. 低延迟：接 `LoraLoader` 加载 turbo 8step（或 4step），步数设 8（或 4）

---

## 7. NVFP4 权重组合与放置

| 角色 | 文件（ComfyUI `models/` 下） | 体积 | 来源 | 状态 |
|---|---|---|---|---|
| 文本编码器 | `text_encoders/qwen3vl_32b_minimax_h3_nvfp4_awq.safetensors` | 15.69 GB | Comfy-Org 官方 | **默认（NVFP4 硬件加速项）** |
| DiT（默认） | `diffusion_models/minimax_h3_fl2va_pruned_int8_convrot.safetensors` | 20.97 GB | Comfy-Org 官方 | 官方教程默认；GB10 需验证 int8 内核 |
| DiT（可选/更小） | `diffusion_models/minimax_h3_fl2va_pruned_w4a8_mixed.safetensors` | 12.54 GB | 社区（Kijai experimental） | **进阶、未验证**：需 ComfyUI ≥0.31.0 + comfy-kitchen w4a8 内核 |
| DiT（备选） | `*_pruned_fp8_scaled.safetensors` / `*_pruned_bf16.safetensors` | 20.96 GB / 40.2 GB | Comfy-Org 官方 | int8 内核不可用时的回退 |
| 视频 VAE | `vae/minimax_h3_video_vae_fp16.safetensors` | 5.21 GB | Comfy-Org 官方 | 默认 |
| 音频 VAE | `vae/minimax_h3_audio_vae_fp32.safetensors` | 0.61 GB | Comfy-Org 官方 | 默认（立体声输出） |
| LoRA | `loras/minimax_h3_fl2v_turbo_8step_v1.0_comfyui_bf16.safetensors` | 1.96 GB | Comfy-Org 官方 | **官方模板扫描必需**；同时是主要提速手段 |
| Ref2VA 变体 | `diffusion_models/minimax_h3_ref2va_pruned_int8_convrot.safetensors` + `loras/minimax_h3_ref2v_turbo_4step_v0.1_*` | 20.97 + 1.96 GB | Comfy-Org 官方 | 需要多模态参考时追加 |

**关于"NVFP4 DiT"的实话**：官方 Comfy-Org 仓库**只有 NVFP4 编码器**，DiT 侧量化为 int8_convrot / fp8_scaled / bf16（以及剪枝版）；4-bit 的 DiT 目前来自社区（w4a8 等，如 HF space `ramu23/minimax-h3-i2v-r2v-nvfp4-cn`），**未验证、按需评估**。因此本方案的"NVFP4"落在：① 编码器 NVFP4-AWQ（官方、默认）；② DiT 可选社区 w4a8（进阶实验项）。

---

## 8. API 服务化

| 方法 | 路径 | 作用 |
|---|---|---|
| POST | `/prompt` | 提交工作流（**API 格式** JSON），返回 `prompt_id` |
| GET | `/history/{prompt_id}` | 查询结果（含输出文件清单） |
| GET | `/view?filename=…&type=output` | 下载产物（MP4） |
| POST | `/upload/image` | 上传参考素材（图/视频/音频） |
| GET | `/system_stats` | 就绪/显存状态（页面状态卡探针同源） |
| WS | `/ws?clientId=…` | 执行进度事件（可选） |

调用约定：

1. 工作流 JSON 用 WebUI「Workflow → Export (API)」导出，本地保存后可反复提交；提示词/参数用变量替换。
2. 单卡同一时间只提交 1 个任务；产物统一落 `<data>/comfyui/output/`，业务侧只读共享。
3. 客户端需自行实现轮询/重试（ComfyUI 无 OpenAI 风格的任务对象）。
4. 校验产物：`ffprobe` 应显示 `h264` + `aac` / `2ch` / `32000 Hz`。

---

## 9. 性能与容量

| 项 | 值/预期 | 说明 |
|---|---|---|
| 容器就绪 | 秒级（`/system_stats` 200） | 权重懒加载，首次出片才真正加载（比 SGLang 的 12 min 冷加载体验好） |
| 权重驻留 | ≈44 GB（int8 DiT + NVFP4 编码器 + VAEs） | 统一内存压力远低于 SGLang BF16 ≈108 GB |
| 单请求耗时 | **待实测**；预期优于 SGLang BF16 基线（权重流量降 ~60%，且可用 4/8 步 turbo） | GB10 去噪受 ~273 GB/s 带宽约束，量化直接减流量 |
| 并发 | 1（串行） | 单卡统一内存；批量任务排队 |
| 实测记录表 | 50 步 / turbo 8 步 / turbo 4 步 × 480P·768P，记录耗时、峰值内存、音频轨是否正确 | 部署后补全 |
| 可选调优（ComfyUI 原生参数，已确认存在于 CLI） | `--vram-headroom <GB>`（给系统留显存余量，统一内存机型防抢占）、`--cache-ram <activeGB> <inactiveGB>`（RAM 压力缓存）、`--disable-smart-memory`（更激进卸载）、`--use-sage-attention`（需镜像内装 wheel） | 按 GB10 统一内存特性逐项 A/B |

---

## 10. 离线保障

| 依赖 | 处理 |
|---|---|
| 镜像 | 本地构建（含 torch/comfy-kitchen/ffmpeg），运行期不装包 |
| 权重 | 预置到 `<data>/models/MiniMax-H3-ComfyUI/`（三条路径统一目录）；容器内 `HF_HUB_OFFLINE=1` / `TRANSFORMERS_OFFLINE=1` |
| 云端节点 | `--disable-api-nodes` 禁用 ComfyUI 云 API 节点，并阻止前端外联（已确认存在于 `comfy/cli_args.py`） |
| 前端版本检查 | `--front-end-version` 默认值为 `comfyanonymous/ComfyUI@latest`（ComfyUI CLI 默认），离线启动若卡在版本检查，追加 `--front-end-root <镜像内前端目录>` 固定用镜像内置前端（构建时确认路径，如 `/opt/ComfyUI/web`） |
| ComfyUI-Manager | 新版为**选择启用**（`--enable-manager`，默认关闭）→ 不启用即无 Manager 联网行为 |
| 模板 | 官方模板随 ComfyUI 内置；导出的 API JSON 本地留存 |
| 提示词增强 | `H3-Context-IR` 为云端服务 → 离线用官方《Prompting Guidance》三段式提示词替代 |
| 首次自动下载 | 属联网行为：**纯离线部署必须用模式 B**（§4.2），否则弹窗会失败 |

---

## 11. 运维手册

| 场景 | 操作 |
|---|---|
| 启动/停止 | 「视频生成」页按钮（后端 `start_model` / `stop_model`，容器 `adm-comfyui-<model_id>`）；异常残留：`docker rm -f adm-comfyui-<model_id>` |
| 构建镜像 / 下载权重 | 同页「环境准备」卡：构建日志与下载进度都在页内（`comfyui_setup_status` 显示镜像/权重状态与目录） |
| 日志 | 页内日志区（`model-log` 流式）；容器输出含权重加载与推理进度 |
| 健康检查 | `curl -s -o /dev/null -w '%{http_code}\n' http://127.0.0.1:8188/system_stats` → `200` |
| 端口变更 | 页内修改（默认 8188）→ 重启容器 |
| 镜像更新 | 页内「构建镜像」（镜像已存在会自动跳过）→ 需重建时先 `docker rmi adm-comfyui-h3:nvfp4-20260915` 再点构建 → 页内「重启」 |
| 权重清理 | 删除 `<data>/models/MiniMax-H3-ComfyUI/<类型>/<文件>`（页内「下载权重」会续下；ComfyUI 弹窗亦会补下） |
| 与 SGLang 切换 | 页内互斥提示一键切换，或 `./scripts/docker/h3-comfyui/h3-switch.sh comfyui|sglang|status|stop` |

常见故障：

| 现象 | 原因 | 处理 |
|---|---|---|
| 启动失败：找不到镜像 | 未构建 | 执行 §4.1 构建后再启动 |
| 一直「启动中」 | 探活路径不对/服务未起 | 确认 `engine_ready_path=/system_stats`；看页内日志 |
| 模板弹窗下载失败/极慢 | 直连 huggingface.co 受限 | 改模式 B 预置（§4.2），或容器内试 `HF_ENDPOINT=https://hf-mirror.com` |
| 加载 int8/w4a8 权重报错 | comfy-kitchen 内核不可用（ARM64/sm121 未覆盖） | 换 `fp8_scaled` 或 `bf16` DiT（§12 回退链） |
| 产物没有声音 | 音频 VAE 未接/未解码 | 检查工作流 audio VAE 节点；`ffprobe` 确认 `aac 2ch 32000Hz` |
| 出片黑屏（用 int8 VAE 时） | 版本过低 | int8_convrot VAE 需 ComfyUI ≥0.31.0，否则用 fp16 VAE |
| 启动即 OOM | 与 SGLang 服务同时在跑 | 用页内互斥提示或 `h3-switch.sh` 切换 |
| 构建卡在 `failed to resolve source metadata …` / `failed to fetch anonymous token … auth.docker.io … i/o timeout` | Docker Hub 不可达（BuildKit 解析 `FROM`、取 token 由 **daemon** 发起：不吃客户端代理、不读 `registry-mirrors`） | 应用会自动探测加速器前缀并改写 `BASE`（探测不带客户端代理，页内日志可见）；仍失败时在「设置 → Docker 镜像配置」填加速器（保存并重启 Docker）；配了「设置 → 代理」仍失败 → 点该 Tab 的「保存并重启 Docker」让 daemon 也走代理 |
| 构建报 `unable to access 'https://github.com/comfyanonymous/ComfyUI/' … GnuTLS recv error (-110)` | github.com 被 TLS 重置 | 应用会自动探测并改用可达源（gitee 镜像 / gitcode 镜像 / ghfast、gh-proxy 加速器，页内日志显示实际源）；也可用「设置 → 代理」或手工 `--build-arg COMFYUI_REPO=<源>` |
| 构建报 `ReadTimeoutError … files.pythonhosted.org` / pip 阶段长时间无进展 | PyPI 官方 CDN 弱网停读（索引可达 ≠ 包体可下） | 应用会自动实测并选用最快的 PyPI 镜像（页内日志显示实测 KB/s）；Dockerfile 已带 `--timeout 60 --retries 10` 且主源失败自动回退官方；仍慢就配「设置 → 代理」，或手工 `--pip-index <镜像>`（build-image.sh）/ `--build-arg PIP_INDEX_URL=<镜像>` |

---

## 12. 风险与已知限制

1. **ARM64 生态**：ComfyUI 官方无 ARM64 镜像（社区多为 x86_64）→ 自建镜像；`comfy-kitchen` aarch64 CUDA wheel 存在（0.2.10+）但 **sm121 内核需实测**；SageAttention 无 ARM64 wheel（跳过，用 turbo LoRA 降步数替代）。
2. **"NVFP4 全量"不成立**：NVFP4 仅覆盖文本编码器（官方）；DiT 为 int8/fp8/bf16（官方）或社区 w4a8（未验证）。页面文案不要写成"全模型 NVFP4"。
3. **首次自动下载依赖外网**：纯离线必须预置；镜像加速对模板内嵌直链是否生效随版本而定。
4. **服务化能力弱于 SGLang**：无 OpenAI 风格异步任务对象、无 BF16 lossless 基线；大批量生产建议切 SGLang（附录 C）。
5. **质量**：turbo 8/4 步与量化会带来画质/音频质量损失；成品建议 50 步或 SGLang 路径。
6. **许可**：MiniMax H3 Community License（含美/欧/英/韩地区额外申请要求），商用前确认。
7. **已实现**：ADM-BE 现支持 `vllm` / `sglang` / `comfyui` 三种引擎与底部「视频生成」页（附录 A 全部落地）。仍待办：把 ComfyUI 条目发布到**远程** `model.json`（本地示例已更新 `doc/model.json`），并构建 `adm-comfyui-h3:nvfp4-20260915` 镜像。

---

## 13. 附录

### A. ADM-BE 改动清单（设计，不含代码）

**状态：A.1–A.11 已实现**（`cargo check` / `pnpm typecheck` 通过）；仅「测试」页跳转提示（A.11 后半句）为可选未做。

| # | 改动 | 实现要点 |
|---|---|---|
| A.1 ✅ | `RemoteModel.engine_ready_path`（默认 `/health`） | ComfyUI 用 `/system_stats`；`spawn_ready_probe` 支持自定义路径 |
| A.2 ✅ | `engine: "comfyui"` 分支 `start_comfyui_docker` | 容器名 `adm-comfyui-<model_id>`（沿用 `adm-<engine>-<model>` 约定）；`--gpus all`、`--ipc host`、`--shm-size`（DGX 16g / 其他 8g）、`-p <port>:<port>`；挂载默认**读写**（`:ro` 条目保持只读）；命令 = `engine_command`（缺省 `python3 main.py`）+ 自动补 `--listen 0.0.0.0` + `--port`，`vllm_flags` 最后追加；跳过设置页 LLM 参数子集；镜像内 `ComfyUI/main.py` 预检 |
| A.3 ✅ | `RemoteModel.hidden_in_list` | 首页 `getFilteredModelList()` 过滤该条目；其它条目行为不变 |
| A.4 ✅ | 底部导航新增 Tab | `src/index.html`：`#video-btn`（「视频生成」→ `#/video`）+ `routes` 注册（位于「首页」之后） |
| A.5 ✅ | 新视图 `src/views/video.js` | 状态卡（状态/地址/端口/镜像/容器/挂载）、启动/停止/重启/打开 WebUI/复制产物路径、首次指引、互斥横幅、日志区（600 行上限、stderr 标红）；遵守视图约定（`video-*` 前缀、事件只在壳层、状态走 `__adm_state`） |
| A.6 ✅ | 后端接口复用 | `start_model` / `stop_model` / `get_model_status` + `model-started` / `model-stopped` / `model-log` / `model-pull-progress`；未新增 API 面 |
| A.7 ✅ | 端口配置 | 前端 `localStorage["adm_comfyui_port"]`（默认 8188）→ `params.port`；与设置页 SGLang 端口隔离 |
| A.8 ✅ | 互斥 | 页内互斥横幅（其它模型运行中时）+ 模型列表侧守卫（ComfyUI 运行中禁止直接启动其它模型）；`exclusive_group` 字段已入模型（当前前端按「已有模型在运行」判断，字段供后续精细化） |
| A.9 ✅ | 生命周期 | 运行中容器名记录在 `AppState.running_container`，`stop_model` / `cleanup_processes` 按记录名 `stop/rm`；ComfyUI 退出线程同时兜底 `docker rm -f` |
| A.10a ✅ | 下载清单统一 | `model_download_files` 单字段承载两种条目：`org/name`（整仓）与 `org/name/<仓库内路径或通配>`（hfd `--include`，同仓库合并去重）；`model_download_includes` 字段已废弃删除 |
| A.10 ✅ | 空清单容忍 | 无 `model_download_files` 时首页不渲染下载按钮；后端 `start_model` 对 `engine: comfyui` 不再要求本地模型目录（先于目录模型判断分发） |
| A.11 ✅/— | i18n | 「视频生成」页文案已入 `src/i18n.js`；「测试」页跳转提示未做（可选） |
| A.12 ✅ | 应用内构建/下载 | `src-tauri/src/pages/comfyui.rs`：`comfyui_setup_status`（镜像是否已构建、权重是否就绪/总字节）+ `build_comfyui_image`（写入内置 Dockerfile → `docker build`，输出流式进页内日志；镜像已存在则跳过）；Dockerfile 单一真源 = `scripts/docker/h3-comfyui/Dockerfile`（`include_str!` 内嵌）；`download_model` 链路复用（完成后 ComfyUI 引擎跳过 registry 拉取校验） |

### B. `model.json` 示例条目

```json
[
  {
    "model_id": "MiniMax-H3-ComfyUI",
    "hidden_in_list": true,
    "model_download_files": [
      "Comfy-Org/MiniMax-H3/diffusion_models/minimax_h3_fl2va_pruned_int8_convrot.safetensors",
      "Comfy-Org/MiniMax-H3/diffusion_models/minimax_h3_ref2va_pruned_int8_convrot.safetensors",
      "Comfy-Org/MiniMax-H3/text_encoders/qwen3vl_32b_minimax_h3_nvfp4_awq.safetensors",
      "Comfy-Org/MiniMax-H3/vae/minimax_h3_video_vae_fp16.safetensors",
      "Comfy-Org/MiniMax-H3/vae/minimax_h3_audio_vae_fp32.safetensors",
      "Comfy-Org/MiniMax-H3/loras/minimax_h3_fl2v_turbo_8step_v1.0_comfyui_bf16.safetensors",
      "Comfy-Org/MiniMax-H3/loras/minimax_h3_ref2v_turbo_4step_v0.1_comfyui_bf16.safetensors",
      "Comfy-Org/MiniMax-H3/embeddings/*"
    ],
    "model_support_devices": ["dgx-spark-128G"],
    "engine": "comfyui",
    "engine_image": "adm-comfyui-h3:nvfp4-20260915",
    "engine_command": ["python3", "main.py", "--listen", "0.0.0.0", "--port", "8188",
                       "--disable-auto-launch", "--disable-api-nodes"],
    "engine_ready_probe": true,
    "engine_ready_path": "/system_stats",
    "exclusive_group": "h3",
    "vllm_env": [
      "PYTORCH_CUDA_ALLOC_CONF=expandable_segments:True"
    ],
    "vllm_extra_mounts": [
      "models/MiniMax-H3-ComfyUI:/opt/ComfyUI/models",
      "comfyui/output:/opt/ComfyUI/output",
      "comfyui/input:/opt/ComfyUI/input",
      "comfyui/user:/opt/ComfyUI/user",
      "media:/data/minimax-h3"
    ],
    "model_size": "≈44 GB（权重首次自动下载）",
    "model_type": "视频生成（ComfyUI）",
    "model_description": "MiniMax-H3 via ComfyUI：NVFP4-AWQ 编码器 + 量化 DiT，原生音视频（768p/24fps + 32kHz 立体声），首次打开模板自动下载权重",
    "support_tools": false,
    "support_reasoning": false,
    "support_images": true,
    "support_video": true
  }
]
```

> 说明：`vllm_extra_mounts` 的显式 `host:container` 映射与自动建目录**已实现**；`engine_ready_path` / `hidden_in_list` / `exclusive_group` / `engine: comfyui` / 「视频生成」页为**待实现项**（附录 A）。

### C. 备选：SGLang 生产路径（需要 `/v1/videos` 时）

| 项 | 内容 |
|---|---|
| 镜像 | `docker build -t adm-sglang-h3:20260915 scripts/docker/h3-sglang`（预装 SGLang diffusion extra，固定 revision `15d2cbcc90fc`） |
| 权重 | HF 原始目录（统一下载清单：`MiniMaxAI/MiniMax-H3/FL2VA/*` + `.../Ref2VA/*` + `.../model_index.json` ≈288 GB） |
| 启动命令 | `sglang serve --model-path /models/MiniMax-H3 --host 0.0.0.0 --port 8000 --model-variant fl2va`（应用侧 `engine: sglang` + `engine_command: ["sglang","serve"]`） |
| 就绪 | `GET /health` 200（warmup 完成后；冷加载 ≈12 min） |
| API | `POST /v1/videos` → `GET /v1/videos/{id}` → `GET /v1/videos/{id}/content`；输出 MP4 H.264 24fps + AAC 32kHz |
| 激活模式 | `--model-variant fl2va`（t2va/首尾帧）或 `ref2va`（多模态参考），权重共用、重启切换 |
| 关键约束 | 单机不加 offload / CFG 并行参数；`engine_command` 模式下设置页 LLM 参数子集自动跳过；FP8/NVFP4/AdaLN 旁路在 GB10 未验证 |
| 性能基线 | 冷加载 ≈12 min；文本编码 ≈5.5 min/请求；去噪 ≈12.1 s/step（480P）；warm 请求 ≈12 min；单并发 |
| 互斥 | 与 ComfyUI 不能同时运行（≈108 GB + ≈44 GB > 121 GiB）：页内提示或 `h3-switch.sh` |
| 历史细节 | 完整 flags/参数表/故障排查见 git 历史（本文件 2026-09-15 首版）与 `scripts/docker/h3-sglang/Dockerfile` |

**启用方式**：把下面片段加回（本地/远程）`model.json`，并按 4.1 构建 `adm-sglang-h3:20260915`。
该条目**默认不放进样例清单**（避免误下 288 GB 与误占统一内存）；SGLang 引擎本身仍被清单中其他模型（如 `Ling-3.0-flash-VL-fp8`）使用，代码路径保留。

```json
{
  "model_id": "MiniMax-H3",
  "model_download_files": [
    "MiniMaxAI/MiniMax-H3/model_index.json",
    "MiniMaxAI/MiniMax-H3/FL2VA/*",
    "MiniMaxAI/MiniMax-H3/Ref2VA/*"
  ],
  "model_support_devices": ["dgx-spark-128G"],
  "engine": "sglang",
  "engine_image": "adm-sglang-h3:20260915",
  "engine_command": ["sglang", "serve"],
  "engine_ready_probe": true,
  "vllm_flags": ["--model-variant fl2va"],
  "vllm_env": [
    "PYTORCH_CUDA_ALLOC_CONF=expandable_segments:True",
    "SGLANG_DIFFUSION_MINIMAX_H3_ADALN_GPU_PLANS=64",
    "NCCL_DEBUG=WARN",
    "HF_HUB_OFFLINE=1",
    "TRANSFORMERS_OFFLINE=1"
  ],
  "vllm_extra_mounts": ["media:/data/minimax-h3"],
  "model_size": "288 GB",
  "model_type": "视频生成",
  "model_description": "MiniMax H3 原生音视频生成（768p/24fps + 32kHz 立体声），Docker + SGLang 扩散引擎单机离线服务；--model-variant fl2va 覆盖 t2va/首尾帧；多模态参考输入（图/视频/音频）切 ref2va 后重启；2K 需官方云 API",
  "support_tools": false,
  "support_reasoning": false,
  "support_images": true,
  "support_video": true
}
```

### D. 来源与版本

| 资料 | 位置 |
|---|---|
| ComfyUI H3 教程（版本/文件/目录/工作流） | `https://docs.comfy.org/zh/tutorials/video/minimax/minimax-h3`（及 `-native` / `-multiframe` / `-fun-controlnet`） |
| ComfyUI 单文件权重仓库（官方量化集） | `https://huggingface.co/Comfy-Org/MiniMax-H3` |
| 社区低比特 DiT（w4a8，未验证） | `https://huggingface.co/Kijai/MiniMax-H3-experimental` |
| 量化内核库（int8_convrot / NVFP4 / w4a8） | `https://github.com/Comfy-Org/comfy-kitchen`（PyPI：aarch64 CUDA wheel 自 0.2.10；NVFP4 需 `[cublas]` extra） |
| 官方工作流模板 | `https://github.com/Comfy-Org/workflow_templates`（`video_minimax_h3_t2v.json` / `_r2v.json`） |
| 本仓库脚本 | `scripts/docker/h3-comfyui/`（镜像 / compose / 权重预置 / 互斥切换）、`scripts/docker/h3-sglang/`（备选镜像） |

变更记录：

| 日期 | 变更 |
|---|---|
| 2026-09-15 | 首版：单机 Docker + SGLang 生产方案（含 model.json 字段、就绪探活、API、量化路线） |
| 2026-09-15 | 合并 ComfyUI sidecar 为附录 C（原独立文档删除） |
| 2026-09-15 | 方案改版：主线切换为 **ComfyUI + NVFP4（首次自动下载）**；新增首页 **ComfyUI 启动卡片**设计 |
| 2026-09-15 | **入口改版（本次）**：首页卡片取消 → **底部导航新增「视频生成」Tab**，ComfyUI 的启动/停止/状态/日志/首次指引全部在 `src/views/video.js`（§5 重写；附录 A 增补导航/视图/`hidden_in_list` 改动；模型列表不再出现 ComfyUI 卡片） |
| 2026-09-15 | **权重目录统一**：应用内下载 / compose 挂载 / `download-comfy-h3.sh` / `h3-switch.sh` 统一到 `<data>/models/MiniMax-H3-ComfyUI`（`ADM_COMFYUI_WEIGHTS` 可覆盖），消除三处路径不一致 |
| 2026-09-15 | **代码审核修订**：权重计数排除 `.aria2` 未完成文件；include 模式匹配对齐 hfd 子串语义（进度基准）；模型列表透传 `engine_ready_path`；视频页在 `model-started` 后刷新环境状态；补充 `--disable-api-nodes` 对弹窗下载的影响、前端版本检查离线兜底与 ComfyUI 原生调优参数说明 |
| 2026-09-15 | **清单瘦身**：样例 `doc/model.json` 移除 SGLang 版 `MiniMax-H3` 条目（避免误下 288 GB），改在附录 C 以「启用片段」形式保留；SGLang 引擎支持不变（其他模型在用） |
| 2026-09-15 | **下载清单统一**：`model_download_includes` 删除，include 模式内联进 `model_download_files` 的仓库条目（`org/name/路径或通配`）；多仓库按条目顺序依次下载、同仓库模式合并去重，进度基准按同一模式过滤 |
| 2026-09-15 | **应用内准备流程**：新增「构建镜像」（内置 Dockerfile + `docker build`，日志流式）与「下载权重」（`Comfy-Org/MiniMax-H3` + include，≈67 GB）按钮与状态显示；`comfyui_setup_status` / `build_comfyui_image` 两个命令；挂载改为 `models/MiniMax-H3-ComfyUI:/opt/ComfyUI/models` |
| 2026-09-15 | **开发落地**：附录 A 全部实现（`engine_ready_path` / `hidden_in_list` / `engine: comfyui` 分发 `start_comfyui_docker` / 挂载读写模式 / 底部「视频生成」Tab + `views/video.js` / 互斥守卫 / i18n）；`doc/model.json` 增补 ComfyUI 条目；`h3-switch.sh` 改为按前缀探测容器 |
| 2026-09-15 | **构建网络加固**：`build_comfyui_image` 构建前用 `docker manifest inspect` 探测基础镜像源（已配置加速器 → 官方源 → 内置加速器）并把可达前缀写进 `--build-arg BASE`，构建期自动注入「设置 → 代理」（本机代理加 `--network=host`）；Dockerfile 新增 `COMFYUI_REPO` / `TORCH_INDEX_URL` / `PIP_INDEX_URL` / 代理构建参数 |
| 2026-09-15 | **源码源回退**：`github.com` 被 TLS 重置时，应用侧探测 `…/info/refs` 并把可达源排前（gitee 镜像 / gitcode 镜像 / ghfast、gh-proxy 加速器），Dockerfile 内依次回退；克隆后用 `COMFYUI_REF_SHA` 校验 commit（`v0.30.0` = `b1693ecb…`，四源与上游一致），新增 `COMFYUI_REPO_FALLBACKS` / `COMFYUI_REF_SHA` 构建参数 |
| 2026-09-15 | **构建探测修正**：基础镜像源探测不再注入客户端代理（BuildKit 的 `FROM`/token 解析由 daemon 发起，客户端代理只对 RUN 步骤生效）——避免「配了本地代理 → 误判官方源可达 → 构建再次卡在 `auth.docker.io`」；命中加速器时页内日志会说明原因与 daemon 代理做法 |
| 2026-09-15 | **一键构建脚本**：新增 `scripts/docker/h3-comfyui/build-image.sh`（与应用侧同款「基础镜像源探测 → 加速器前缀、ComfyUI 源码源回退 + commit 校验、代理注入（本机代理加 `--network=host`）、跨架构 QEMU 提示」，另有 `--save` 导出 tar.gz + sha256、`--dry-run` 预览、构建后自检），可在网络更好的设备上构建后 `docker load` 回 DGX |
| 2026-09-15 | **x86 宿主跨架构构建**：Dockerfile 新增 `SKIP_RUNTIME_CHECK` 参数（QEMU 模拟下可跳过镜像内自检）；`build-image.sh` 自动识别 x86→arm64（`--platform` + binfmt 检测提示 + 旧版 docker 自动 `DOCKER_BUILDKIT=1`）、新增 `--skip-runtime-check`；§4.1 补 Ubuntu 22.04 / WSL2 构建指引 |
| 2026-09-15 | **下载进度修复（断点续传显示 0% 不动）**：hfd 进度改为按**字节**加权折算——已完成文件计全量、进行中文件按已分配块数折真实落盘字节（aria2 `--file-allocation=none` 按段稀疏写盘，文件长度会瞬间逼近全量，只有 `.aria2` 控件存在时不能只看长度），续传时进度从上次中断处继续显示；进度基准在 HF API 不可达时退回 hfd 自带 `.hfd/manifest`；`download-progress` 事件带 `speed`，视频页下载中显示速度；`comfyui_setup_status` 新增 `weights_partial`（目录树里有 `.aria2` 控件即视为未就绪）→ 页面显示「已中断（已下载 xx GB，可继续断点续传）」+「继续下载」 |
| 2026-09-15 | **pip 源加固（构建卡在 torch 依赖）**：实测确认 pip 同版本下「本地版本标记优先」（`2.14.0+cu130` 胜过镜像里的 `2.14.0`），于是 torch 步骤改为 `--index-url TORCH_INDEX_URL --extra-index-url PIP_INDEX_URL`（轮子固定官方、依赖走镜像，不再回落到 `files.pythonhosted.org`）；所有 pip 安装加 `--timeout 60 --retries 10`，requirements / `comfy-kitchen[cublas]` 主源失败自动回退官方 PyPI；torch 装完校验 `torch.version.cuda` 大版本（不符 → 官方索引重装 → 仍不符则构建失败）；三个 pip 层加 BuildKit cache mount（`--mount=type=cache,target=/root/.cache/pip`，wheel 不进镜像层，失败重试不重下 2 GB+），`build-image.sh` 相应强制 `DOCKER_BUILDKIT=1`；应用侧与 `build-image.sh` 均改为**实测各 PyPI 源吞吐择优**（官方 + 清华/阿里/腾讯/华为云，`<源>/pip/` 索引页），日志显示实测 KB/s |
