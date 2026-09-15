# ADM-BE：MiniMax-H3（SGLang Diffusion）单机离线推理镜像
# **方案定位：备选路径** —— 主线已改为 Docker + ComfyUI（NVFP4、首次自动下载）
# 方案文档：doc/minimax-h3-single-node-deploy-plan.md（备选 SGLang 见附录 C）
# 目标平台：DGX Spark / GB10（ARM64 + CUDA 13，sm121）
#
# 依据 SGLang 官方 cookbook（MiniMax-H3）的 Docker 形态（dockerRunCommand =
# `bash -lc 'pip install -e "/sgl-workspace/sglang/python[diffusion]" && exec sglang serve "$@"' --`），
# 把 diffusion extra 在构建期预装进镜像，使运行期完全不依赖网络（纯离线，且启动不再耗时安装）。
#
# 构建（需联网一次，产物本地 tag 即 ADM-BE model.json 的 engine_image）：
#   docker build -t adm-sglang-h3:20260915 scripts/docker/h3-sglang
# 指定基础镜像 / SGLang revision：
#   docker build -t adm-sglang-h3:20260915 \
#     --build-arg BASE=lmsysorg/sglang:dev \
#     --build-arg SGLANG_REV=15d2cbcc90fc \
#     scripts/docker/h3-sglang

ARG BASE=lmsysorg/sglang:dev
FROM ${BASE}

# SGLang 官方验证过的 revision（GB300 实测 commit）；镜像内没有该 commit 时保留镜像 HEAD
ARG SGLANG_REV=15d2cbcc90fc
# GB10 = sm121；显式声明避免运行期 JIT 误判架构
ARG TORCH_CUDA_ARCH_LIST=12.1a

ENV DEBIAN_FRONTEND=noninteractive \
    PIP_DISABLE_PIP_VERSION_CHECK=1 \
    PYTHONUNBUFFERED=1 \
    TORCH_CUDA_ARCH_LIST=${TORCH_CUDA_ARCH_LIST} \
    HF_HUB_OFFLINE=1 \
    TRANSFORMERS_OFFLINE=1 \
    PYTORCH_CUDA_ALLOC_CONF=expandable_segments:True

# ffmpeg/ffprobe：参考素材预处理与 MP4（H.264 + AAC 立体声）封装所需
RUN apt-get update \
    && apt-get install -y --no-install-recommends ffmpeg libgl1 libglib2.0-0 jq \
    && rm -rf /var/lib/apt/lists/*

# 预装 diffusion extra（H3 属于 SGLang Diffusion 路径，非 LLM 路径）
RUN set -eux; \
    cd /sgl-workspace/sglang; \
    if git rev-parse --verify "${SGLANG_REV}^{commit}" >/dev/null 2>&1; then \
        git checkout "${SGLANG_REV}"; \
    else \
        echo "[warn] revision ${SGLANG_REV} 不在镜像内，保留当前 HEAD"; \
    fi; \
    python3 -m pip install -e "python[diffusion]" --prerelease=allow; \
    python3 -c "import sglang, sglang.multimodal_gen; print('sglang ok:', sglang.__version__)"

# 运行期 ENTRYPOINT 由 ADM-BE 以 `--entrypoint=` 清空后自持 `sglang serve ...`，
# 这里不设置 CMD/ENTRYPOINT，保持镜像入口干净。
