#!/usr/bin/env bash
# 下载 MiniMax-H3 的 ComfyUI 单文件权重（**模式 B：离线预置**；默认模式 A 为首次打开模板自动下载）
# 方案文档：doc/minimax-h3-single-node-deploy-plan.md §4.2 / §7 / §10
#
# 与 ADM-BE 的 SGLang 权重（HF 原始目录格式）**不共用**：ComfyUI 走单文件量化包，
# 体积更小（最小集 ~44 GB vs 288 GB），适合单机交互/低延迟场景。
#
# 用法：
#   export ADM_DATA_DIR="$HOME/.local/share/com.adm.be"
#   ./download-comfy-h3.sh                # 默认 minimal（fl2va 文/首尾帧 → 音视频）
#   ./download-comfy-h3.sh ref2va         # 追加多模态参考（ref2va）权重
#   ./download-comfy-h3.sh all            # 追加 bf16/fp8 全精度变体与 Fun ControlNet
#
# 说明：
# - 复用 ADM-BE 已下载的 hfd.sh（<data>/hfd.sh）；缺失时自动从 hf-mirror 获取
# - 未配置代理时自动走 https://hf-mirror.com（与 ADM-BE 下载策略一致）
# - 目标目录即 ComfyUI 的 models 目录，按官方类型子目录落盘（diffusion_models/ 等）
set -euo pipefail

PROFILE="${1:-minimal}"
DATA_DIR="${ADM_DATA_DIR:-$HOME/.local/share/com.adm.be}"
REPO="Comfy-Org/MiniMax-H3"
# 权重目录：与应用内「下载权重」/ compose 挂载完全一致（可用 ADM_COMFYUI_WEIGHTS 覆盖）
DEST="${ADM_COMFYUI_WEIGHTS:-$DATA_DIR/models/MiniMax-H3-ComfyUI}"
HFD="$DATA_DIR/hfd.sh"
HF_ENDPOINT="${HF_ENDPOINT:-https://hf-mirror.com}"

mkdir -p "$DEST"

if [[ ! -f "$HFD" ]]; then
  echo "[i] 未找到 $HFD，正在从 $HF_ENDPOINT 获取 hfd.sh"
  curl -fsSL "$HF_ENDPOINT/hfd/hfd.sh" -o "$HFD"
  chmod 755 "$HFD"
fi

# 满足 ComfyUI 官方教程的最小可用集合（fl2va：t2va / i2va / l2va）
INCLUDES=(
  "diffusion_models/minimax_h3_fl2va_pruned_int8_convrot.safetensors"
  "text_encoders/qwen3vl_32b_minimax_h3_nvfp4_awq.safetensors"
  "vae/minimax_h3_video_vae_fp16.safetensors"
  "vae/minimax_h3_audio_vae_fp32.safetensors"
  "loras/minimax_h3_fl2v_turbo_8step_v1.0_comfyui_bf16.safetensors"
  "embeddings/*"
)

case "$PROFILE" in
  minimal) ;;
  ref2va)
    INCLUDES+=(
      "diffusion_models/minimax_h3_ref2va_pruned_int8_convrot.safetensors"
      "loras/minimax_h3_ref2v_turbo_4step_v0.1_comfyui_bf16.safetensors"
    )
    ;;
  all)
    INCLUDES+=(
      "diffusion_models/*"
      "text_encoders/*"
      "vae/*"
      "loras/*"
      "model_patches/*"
      "embeddings/*"
    )
    ;;
  *)
    echo "[x] 未知档位：$PROFILE（可选：minimal | ref2va | all）" >&2
    exit 1
    ;;
esac

echo "[i] 档位：$PROFILE"
echo "[i] 仓库：$REPO  →  $DEST"
echo "[i] 端点：$HF_ENDPOINT（如需直连 huggingface.co 请先 export HF_ENDPOINT=https://huggingface.co）"
printf '[i] include: %s\n' "${INCLUDES[@]}"

HF_ENDPOINT="$HF_ENDPOINT" bash "$HFD" "$REPO" --include "${INCLUDES[@]}" --local-dir "$DEST"

echo
echo "[✓] 完成。目录结构："
find "$DEST" -maxdepth 2 -type f -name '*.safetensors' -printf '  %p  (%s bytes)\n' | sort
echo "[i] 接下来：在应用「视频生成」页启动 ComfyUI，或手工执行"
echo "      docker compose -f $(dirname "$0")/docker-compose.yml up -d"
