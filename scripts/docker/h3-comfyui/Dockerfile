# ADM-BE 专用：ComfyUI（MiniMax-H3）镜像——**方案主线运行时**
# 方案文档：doc/minimax-h3-single-node-deploy-plan.md（§1–§12）
# 目标平台：DGX Spark / GB10（ARM64 + CUDA 13，sm121）
#
# 说明：
# - ComfyUI 官方未发布 ARM64 镜像（社区镜像多为 x86_64）；本 Dockerfile 自建。
# - H3 路径：**NVFP4-AWQ 文本编码器 + 量化 DiT**（默认 int8_convrot 剪枝，可选社区 w4a8）。
# - NVFP4 / int8_convrot 内核来自核心包 comfy-kitchen；**NVFP4 需 `[cublas]` extra**。
# - 权重默认由 ComfyUI **首次打开模板时自动下载**（§4.2 模式 A）；纯离线用 download-comfy-h3.sh 预置。
# - 三个 pip 层用 BuildKit cache mount 缓存 wheel（`--mount=type=cache,target=/root/.cache/pip`，不进镜像层）：
#   弱网构建失败后重试不必重下 2 GB+；需 BuildKit（Docker 23+ 默认；classic builder 不支持该语法）。
#
# ⚠️ 本文件是「Dockerfile 单一真源」：ADM-BE 应用内「视频生成」页的「构建镜像」按钮
#    通过 include_str! 内嵌本文件（src-tauri/src/pages/comfyui.rs），运行时写到
#    <data>/build/comfyui/Dockerfile 再执行 docker build。修改本文件后需重新编译应用。
#
# 构建（需联网一次）：
#   docker build -t adm-comfyui-h3:nvfp4-20260915 scripts/docker/h3-comfyui
# 可选参数（--build-arg）：
#   BASE=nvidia/cuda:13.0.0-runtime-ubuntu24.04   # 基础镜像；Docker Hub 不可达时用加速器前缀（如 docker.1ms.run/nvidia/cuda:...）
#   COMFYUI_REF=v0.30.0                           # ComfyUI 版本（H3 需 ≥ 0.30.0）
#   COMFYUI_REPO=https://github.com/comfyanonymous/ComfyUI
#   COMFYUI_REPO_FALLBACKS="<空格分隔的备用源>"           # 克隆失败时依次回退（缺省含 gitee / gitcode / ghfast / gh-proxy）
#   COMFYUI_REF_SHA=<commit>                             # 非空时校验克隆结果 commit（防第三方镜像篡改/滞后）；留空跳过
#   TORCH_INDEX_URL=https://download.pytorch.org/whl/cu130   # PyTorch wheel 索引（须能给出 aarch64 + cu13 轮子）
#   PIP_INDEX_URL=                                # PyPI 源；留空 = 官方。应用内「构建镜像」会实测各源吞吐择优注入国内镜像
#   SKIP_RUNTIME_CHECK=0                          # 1 = 跳过镜像内自检（跨架构 QEMU 构建时用；到 DGX 实机再验）
#   HTTP_PROXY / HTTPS_PROXY / ALL_PROXY / NO_PROXY（大小写均可）——应用「设置 → 代理」开启后自动注入
#
# 网络说明（重要）：Docker Hub 不通（registry-1.docker.io / auth.docker.io i/o timeout）时，BuildKit
# 解析 FROM（含取匿名 token）由 daemon 发起——不读 daemon.json 的 registry-mirrors、也不吃客户端代理
# （daemon.json `proxies` 才生效）；应用侧「构建镜像」会先用 docker manifest inspect（同样**不带**客户端
# 代理，避免误判官方源可达）探测候选源（已配置加速器 → 官方源 → 内置加速器），把可达前缀写进 BASE；
# 客户端代理只经 --build-arg 注入 RUN 步骤（apt/pip/git）。
# github.com 被 TLS 重置（GnuTLS recv error -110）同理：应用侧会探测 git 源可达性并把可达源排在前面。

ARG BASE=nvidia/cuda:13.0.0-runtime-ubuntu24.04
FROM ${BASE}

# H3 支持自 ComfyUI v0.30.0 起；Fun ControlNet Union 模板需 ≥ 0.35.0
ARG COMFYUI_REF=v0.30.0

# ComfyUI 源码仓库（国内可换成镜像/中转地址）；克隆失败时依次回退到 COMFYUI_REPO_FALLBACKS
ARG COMFYUI_REPO=https://github.com/comfyanonymous/ComfyUI
ARG COMFYUI_REPO_FALLBACKS="https://gitee.com/mirrors/ComfyUI.git https://gitcode.com/gh_mirrors/co/ComfyUI.git https://ghfast.top/https://github.com/comfyanonymous/ComfyUI https://gh-proxy.com/https://github.com/comfyanonymous/ComfyUI"
# v0.30.0 对应 commit（上游 tag → SHA，四个备用源一致）；改 COMFYUI_REF 时同步更新或留空跳过校验
ARG COMFYUI_REF_SHA=b1693ecba9f5b65f8c80ab36b195ab963ec92413
# PyTorch wheel 源（aarch64/cu130 官方索引；可用国内镜像覆盖）
ARG TORCH_INDEX_URL=https://download.pytorch.org/whl/cu130
# PyPI 源（留空 = 官方 https://pypi.org/simple）：torch 的依赖、ComfyUI requirements、comfy-kitchen 都走它。
# 为什么必须可配：官方源对未托管包会回落到 files.pythonhosted.org，弱网下长时间停读（实测 8.7 KB/s → Read timed out）。
# 应用侧「构建镜像」会先实测各源吞吐自动注入（国内一般选中镜像）；下面每个 pip 步骤都带官方源兜底重试。
ARG PIP_INDEX_URL=
# 1 = 跳过镜像内自检（跨架构 QEMU 构建时用；到 DGX 实机再验）
ARG SKIP_RUNTIME_CHECK=0
# 可选：构建期代理（应用「设置 → 代理」开启后自动注入；仅构建期生效，不写入镜像 ENV）
ARG HTTP_PROXY
ARG HTTPS_PROXY
ARG ALL_PROXY
ARG NO_PROXY
ARG http_proxy
ARG https_proxy
ARG all_proxy
ARG no_proxy

ENV DEBIAN_FRONTEND=noninteractive \
    PYTHONUNBUFFERED=1 \
    PIP_DISABLE_PIP_VERSION_CHECK=1 \
    HF_HUB_OFFLINE=1 \
    TRANSFORMERS_OFFLINE=1 \
    PYTORCH_CUDA_ALLOC_CONF=expandable_segments:True

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        python3 python3-pip python3-venv git curl ca-certificates \
        ffmpeg libgl1 libglib2.0-0 libsndfile1 \
    && rm -rf /var/lib/apt/lists/*

# PyTorch cu130（ARM64/aarch64 原生 wheel，对齐 GB10/CUDA 13）
# 索引策略（pip 实测行为：同版本时「本地版本标记」优先，故 +cu130 必定胜过镜像里的同版本普通轮子）：
#   --index-url       = PyTorch 官方索引 → torch/torchvision/torchaudio 拿到 aarch64 + cu13 轮子
#   --extra-index-url = PIP_INDEX_URL    → torch 的依赖（cuda-bindings / cuda-pathfinder / triton 等）走镜像，
#                       避开官方索引回落到 files.pythonhosted.org 的慢路径
# --timeout/--retries：默认 15s/5 次对国内弱网不够（一次停读 >15s 就直接 Read timed out 整层失败）
# 装完校验实际 CUDA 大版本：镜像里的同名普通轮子可能是 cu12x，不符则从官方索引重装并复核
# （cu130 → 比 torch.version.cuda 的 13；3 位数去掉末位的规则对 cu126/cu128/cu129/cu130 都成立）
# 若该 index 暂缺 aarch64 产物：TORCH_INDEX_URL 改为 https://download.pytorch.org/whl/nightly/cu130
# cache mount：pip 下载的 wheel 存在 BuildKit 缓存里（不进镜像层），因此网络失败重试不必重下 2 GB+
RUN --mount=type=cache,target=/root/.cache/pip \
    pip_index="${PIP_INDEX_URL:-https://pypi.org/simple}"; unset PIP_INDEX_URL; \
    torch_index="${TORCH_INDEX_URL:-https://download.pytorch.org/whl/cu130}"; \
    expect_cuda="$(printf '%s' "$torch_index" | sed -n 's/.*cu\([0-9][0-9]*\).*/\1/p' | head -n 1)"; \
    case "$expect_cuda" in [0-9][0-9][0-9]) expect_cuda="${expect_cuda%?}" ;; esac; \
    pip_install() { python3 -m pip install --break-system-packages --timeout 60 --retries 10 "$@"; }; \
    cuda_major() { python3 -c "import torch; print((torch.version.cuda or 'none').split('.')[0])"; }; \
    pip_install --index-url "$torch_index" --extra-index-url "$pip_index" torch torchvision torchaudio; \
    if [ "$SKIP_RUNTIME_CHECK" = "1" ]; then \
        echo "[torch] 跳过 CUDA 版本校验（SKIP_RUNTIME_CHECK=1；请在 DGX 实机验证 torch.version.cuda）"; \
    else \
        got_cuda="$(cuda_major)"; \
        if [ -n "$expect_cuda" ] && [ "$got_cuda" != "$expect_cuda" ]; then \
            echo "[torch] 已装轮子是 CUDA ${got_cuda}.x（期望 ${expect_cuda}.x）：从 PyTorch 官方索引重装" >&2; \
            python3 -m pip uninstall -y torch torchvision torchaudio >/dev/null 2>&1 || true; \
            pip_install --index-url "$torch_index" torch torchvision torchaudio; \
            got_cuda="$(cuda_major)"; \
        fi; \
        echo "[torch] 安装完成：torch $(python3 -c 'import torch; print(torch.__version__)') / CUDA ${got_cuda}（期望 ${expect_cuda:-不校验}）"; \
        if [ -n "$expect_cuda" ] && [ "$got_cuda" != "$expect_cuda" ]; then \
            echo "[torch] CUDA 版本不符：期望 cu${expect_cuda}x（torch.version.cuda ${expect_cuda}.x），实得 ${got_cuda}.x——检查 TORCH_INDEX_URL 是否有 aarch64 + cu${expect_cuda}0 轮子" >&2; \
            exit 1; \
        fi; \
    fi

# ComfyUI 本体（requirements.txt 内含 comfy-kitchen 基础包）
# pip_index：PIP_INDEX_URL 为空时用官方 PyPI；显式 --index-url 优先于环境变量
# 源回退：github.com 在部分网络被 TLS 重置（GnuTLS recv error -110 / i/o timeout）→ 依次尝试备用源
# 完整性：COMFYUI_REF_SHA 非空时校验克隆结果 commit（git 对象按 SHA 校验，可挡住被篡改的镜像）
RUN --mount=type=cache,target=/root/.cache/pip \
    pip_index="${PIP_INDEX_URL:-https://pypi.org/simple}"; unset PIP_INDEX_URL; \
    cloned=""; \
    for repo in "${COMFYUI_REPO}" ${COMFYUI_REPO_FALLBACKS}; do \
        echo "[clone] 尝试 ${repo}（tag ${COMFYUI_REF}）"; \
        rm -rf /opt/ComfyUI; \
        if git clone --depth 1 --branch "${COMFYUI_REF}" "$repo" /opt/ComfyUI; then cloned="$repo"; break; fi; \
    done; \
    if [ -z "$cloned" ]; then \
        echo "[clone] 所有源码源均失败：请检查网络，或用 --build-arg COMFYUI_REPO=<可用源> 指定" >&2; \
        exit 1; \
    fi; \
    if [ -n "${COMFYUI_REF_SHA}" ]; then \
        got="$(git -C /opt/ComfyUI rev-parse HEAD)"; \
        if [ "$got" != "${COMFYUI_REF_SHA}" ]; then \
            echo "[clone] 校验失败：${cloned} 的 ${COMFYUI_REF} = ${got}，期望 ${COMFYUI_REF_SHA}" >&2; \
            echo "[clone] 如确认要跟随新版本，请传 --build-arg COMFYUI_REF_SHA=<sha>（或留空跳过校验）" >&2; \
            exit 1; \
        fi; \
        echo "[clone] 校验通过：${COMFYUI_REF} = ${got}（源 ${cloned}）"; \
    fi; \
    pip_install() { python3 -m pip install --break-system-packages --timeout 60 --retries 10 "$@"; }; \
    ok=0; tried=""; \
    for idx in "$pip_index" "https://pypi.org/simple"; do \
        case " $tried " in *" $idx "*) continue ;; esac; \
        tried="$tried $idx"; \
        echo "[pip] 安装 ComfyUI requirements：${idx}"; \
        if pip_install --index-url "$idx" -r /opt/ComfyUI/requirements.txt; then ok=1; break; fi; \
        echo "[pip] 源 ${idx} 安装失败，改试下一个源" >&2; \
    done; \
    [ "$ok" = "1" ] || { echo "[pip] 所有 PyPI 源均失败：检查网络，或用 --build-arg PIP_INDEX_URL=<可用镜像>" >&2; exit 1; }

# NVFP4（Blackwell，本方案默认编码器量化格式）与 int8_convrot 内核：
# 需 comfy-kitchen 的 cublas extra（plain wheel 仅带 eager/triton 路径，量化权重会退化或报错）
# 源同上：主源失败自动回退官方 PyPI（镜像同步滞后时可能缺新包）
RUN --mount=type=cache,target=/root/.cache/pip \
    pip_index="${PIP_INDEX_URL:-https://pypi.org/simple}"; unset PIP_INDEX_URL; \
    pip_install() { python3 -m pip install --break-system-packages --timeout 60 --retries 10 "$@"; }; \
    ok=0; tried=""; \
    for idx in "$pip_index" "https://pypi.org/simple"; do \
        case " $tried " in *" $idx "*) continue ;; esac; \
        tried="$tried $idx"; \
        echo "[pip] 安装 comfy-kitchen[cublas]：${idx}"; \
        if pip_install --index-url "$idx" "comfy-kitchen[cublas]"; then ok=1; break; fi; \
        echo "[pip] 源 ${idx} 安装失败，改试下一个源" >&2; \
    done; \
    [ "$ok" = "1" ] || { echo "[pip] comfy-kitchen 安装失败（所有 PyPI 源）" >&2; exit 1; }

# 构建期校验锚点（失败即镜像不可用）：
#   1) torch 能看到 GPU；2) comfy-kitchen 后端可用（H3 的 int8_convrot / NVFP4 权重必需）
# 跨架构构建（x86 宿主 QEMU 模拟 arm64）时该检查可能受模拟影响，可加 --build-arg SKIP_RUNTIME_CHECK=1 跳过，到目标机实机再验
RUN if [ "$SKIP_RUNTIME_CHECK" = "1" ]; then \
        echo "[verify] 跳过镜像内自检（SKIP_RUNTIME_CHECK=1）：请在 DGX 实机验证 torch / comfy-kitchen"; \
    else \
        python3 -c "import torch; print('torch', torch.__version__, '| cuda available:', torch.cuda.is_available()); import comfy_kitchen as ck; print('comfy-kitchen backends:', ck.list_backends())"; \
    fi

WORKDIR /opt/ComfyUI
EXPOSE 8188
# ENTRYPOINT/CMD 由 compose 提供（--listen 0.0.0.0 --port 8188 --disable-auto-launch）
