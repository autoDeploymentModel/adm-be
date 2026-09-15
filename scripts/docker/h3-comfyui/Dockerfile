# ADM-BE 专用：ComfyUI（MiniMax-H3）镜像——**方案主线运行时**
# 方案文档：doc/minimax-h3-single-node-deploy-plan.md（§1–§12；备选 SGLang 见附录 C）
# 目标平台：DGX Spark / GB10（ARM64 + CUDA 13，sm121）
#
# 说明：
# - ComfyUI 官方未发布 ARM64 镜像（社区镜像多为 x86_64）；本 Dockerfile 自建。
# - H3 路径：**NVFP4-AWQ 文本编码器 + 量化 DiT**（默认 int8_convrot 剪枝，可选社区 w4a8）。
# - NVFP4 / int8_convrot 内核来自核心包 comfy-kitchen；**NVFP4 需 `[cublas]` extra**。
# - 权重默认由 ComfyUI **首次打开模板时自动下载**（§4.2 模式 A）；纯离线用 download-comfy-h3.sh 预置。
#
# ⚠️ 本文件是「Dockerfile 单一真源」：ADM-BE 应用内「视频生成」页的「构建镜像」按钮
#    通过 include_str! 内嵌本文件（src-tauri/src/pages/comfyui.rs），运行时写到
#    <data>/build/comfyui/Dockerfile 再执行 docker build。修改本文件后需重新编译应用。
#
# 构建（需联网一次）：
#   docker build -t adm-comfyui-h3:20260915 scripts/docker/h3-comfyui
# 可选参数：
#   --build-arg COMFYUI_REF=v0.30.0          # ComfyUI 版本（H3 需 ≥ 0.30.0）
#   --build-arg BASE=nvidia/cuda:13.0.0-runtime-ubuntu24.04

ARG BASE=nvidia/cuda:13.0.0-runtime-ubuntu24.04
FROM ${BASE}

# H3 支持自 ComfyUI v0.30.0 起；Fun ControlNet Union 模板需 ≥ 0.35.0
ARG COMFYUI_REF=v0.30.0

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
# 若该 index 暂缺 aarch64 产物：改用 https://download.pytorch.org/whl/nightly/cu130
RUN python3 -m pip install --break-system-packages --no-cache-dir \
        torch torchvision torchaudio --index-url https://download.pytorch.org/whl/cu130

# ComfyUI 本体（requirements.txt 内含 comfy-kitchen 基础包）
RUN git clone --depth 1 --branch "${COMFYUI_REF}" https://github.com/comfyanonymous/ComfyUI /opt/ComfyUI \
    && python3 -m pip install --break-system-packages --no-cache-dir -r /opt/ComfyUI/requirements.txt

# NVFP4（Blackwell，本方案默认编码器量化格式）与 int8_convrot 内核：
# 需 comfy-kitchen 的 cublas extra（plain wheel 仅带 eager/triton 路径，量化权重会退化或报错）
RUN python3 -m pip install --break-system-packages --no-cache-dir "comfy-kitchen[cublas]"

# 构建期校验锚点（失败即镜像不可用）：
#   1) torch 能看到 GPU；2) comfy-kitchen 后端可用（H3 的 int8_convrot / NVFP4 权重必需）
RUN python3 -c "import torch; print('torch', torch.__version__, '| cuda available:', torch.cuda.is_available()); \
    import comfy_kitchen as ck; print('comfy-kitchen backends:', ck.list_backends())"

WORKDIR /opt/ComfyUI
EXPOSE 8188
# ENTRYPOINT/CMD 由 compose 提供（--listen 0.0.0.0 --port 8188 --disable-auto-launch）
