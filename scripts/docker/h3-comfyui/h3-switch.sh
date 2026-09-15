#!/usr/bin/env bash
# MiniMax-H3 双路径互斥切换：ComfyUI（方案主线） ↔ SGLang（备选，/v1/videos 生产 API）
# 方案文档：doc/minimax-h3-single-node-deploy-plan.md（主线 §1–§12 / 备选附录 C / 互斥 §1.1）
#
# 说明：应用内实现 ComfyUI 卡片后，此脚本主要用于两种路径共存时的切换与排障
# （若只部署 ComfyUI，脚本仍可用：stop_sglang 分支在容器不存在时直接跳过）
#
# 为什么必须互斥：DGX Spark 只有 128GB 统一内存（≈121GiB 可用）。
#   - SGLang 生产服务：BF16 权重驻留 ≈108GB（+ 激活）
#   - ComfyUI：int8_convrot DiT 20.97GB + NVFP4-AWQ 编码器 15.69GB + VAE 5.8GB ≈ 44GB
# 两者同时跑必然触发 OOM/主机内存回收，故一次只跑一个。
#
# 用法：
#   ./h3-switch.sh status     # 查看当前运行状态与内存
#   ./h3-switch.sh comfyui    # 停 SGLang 容器 → 起 ComfyUI（等待就绪）
#   ./h3-switch.sh sglang     # 停 ComfyUI → 提示回 ADM-BE 点「启动」
#   ./h3-switch.sh stop       # 两个都停
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
COMPOSE_FILE="$SCRIPT_DIR/docker-compose.yml"
export ADM_DATA_DIR="${ADM_DATA_DIR:-$HOME/.local/share/com.adm.be}"
# 权重目录（与应用内下载 / compose 挂载一致）
export ADM_COMFYUI_WEIGHTS="${ADM_COMFYUI_WEIGHTS:-$ADM_DATA_DIR/models/MiniMax-H3-ComfyUI}"

SGLANG_CONTAINER="adm-sglang-MiniMax-H3"
# ComfyUI 容器名：应用（「视频生成」页）启动时为 adm-comfyui-<model_id>，
# 手工 compose 路径为 adm-comfyui-h3 → 统一按前缀探测，避免硬编码
comfy_container() {
  if [[ -n "${ADM_COMFYUI_CONTAINER:-}" ]]; then
    echo "$ADM_COMFYUI_CONTAINER"
    return
  fi
  docker ps -a --filter "name=adm-comfyui-" --format '{{.Names}}' 2>/dev/null | head -1
}
COMFY_PORT="${ADM_COMFYUI_PORT:-8188}"
WAIT_SECS="${ADM_WAIT_SECS:-600}"   # ComfyUI 冷加载等待上限（首次加载量化权重偏慢）

log() { printf '[%s] %s\n' "$(date +%H:%M:%S)" "$*"; }

require_docker() {
  command -v docker >/dev/null 2>&1 || { echo "[x] 未找到 docker" >&2; exit 1; }
  docker info >/dev/null 2>&1 || { echo "[x] Docker daemon 不可用（或当前用户无权限）" >&2; exit 1; }
}

container_state() { # $1=name → running|exited|absent
  local name="$1"
  if docker container inspect "$name" >/dev/null 2>&1; then
    docker container inspect -f '{{.State.Status}}' "$name"
  else
    echo "absent"
  fi
}

show_status() {
  echo "SGLang 生产服务容器 ($SGLANG_CONTAINER): $(container_state "$SGLANG_CONTAINER")"
  local c; c="$(comfy_container)"
  echo "ComfyUI 容器 (${c:-未创建}):   $(container_state "${c:-adm-comfyui-h3}")"
  if command -v free >/dev/null 2>&1; then
    echo "--- 内存 ---"
    free -h
  fi
  if [[ "$(container_state "$(comfy_container)")" == "running" ]]; then
    echo "ComfyUI WebUI: http://127.0.0.1:$COMFY_PORT"
  fi
}

stop_sglang() {
  local st; st="$(container_state "$SGLANG_CONTAINER")"
  if [[ "$st" == "absent" ]]; then
    log "SGLang 容器未运行（ADM-BE 未启动模型），跳过"
  else
    log "停止 SGLang 容器 $SGLANG_CONTAINER（释放 ≈108GB 权重占用）"
    docker rm -f "$SGLANG_CONTAINER" >/dev/null
  fi
}

stop_comfyui() {
  local c; c="$(comfy_container)"
  if [[ "$(container_state "${c:-adm-comfyui-h3}")" != "absent" ]]; then
    log "停止 ComfyUI 容器 ${c:-adm-comfyui-h3}"
    docker compose -f "$COMPOSE_FILE" down >/dev/null 2>&1 || docker rm -f "${c:-adm-comfyui-h3}" >/dev/null
  else
    log "ComfyUI 容器未运行，跳过"
  fi
}

wait_comfyui_ready() {
  local url="http://127.0.0.1:$COMFY_PORT/system_stats" waited=0
  log "等待 ComfyUI 就绪（$url，上限 ${WAIT_SECS}s）"
  while (( waited < WAIT_SECS )); do
    if curl -fsS -m 5 "$url" >/dev/null 2>&1; then
      log "ComfyUI 已就绪 → http://127.0.0.1:$COMFY_PORT"
      curl -fsS -m 5 "$url" | head -c 400; echo
      return 0
    fi
    local c; c="$(comfy_container)"
    if [[ "$(container_state "${c:-adm-comfyui-h3}")" != "running" ]]; then
      echo "[x] ComfyUI 容器已退出，请查看日志：docker logs --tail 120 ${c:-adm-comfyui-h3}" >&2
      return 1
    fi
    sleep 5; waited=$((waited + 5))
  done
  echo "[x] 等待超时（${WAIT_SECS}s），模型可能仍在加载。可继续观察：docker logs -f $(comfy_container)" >&2
  return 1
}

start_comfyui() {
  stop_sglang
  mkdir -p "$ADM_COMFYUI_WEIGHTS" "$ADM_DATA_DIR/comfyui/output" \
           "$ADM_DATA_DIR/comfyui/input" "$ADM_DATA_DIR/comfyui/user" "$ADM_DATA_DIR/media"
  if [[ -z "$(find "$ADM_COMFYUI_WEIGHTS" -maxdepth 2 -name '*.safetensors' -print -quit)" ]]; then
    echo "[x] 未发现 ComfyUI 权重：请先执行 ./download-comfy-h3.sh（minimal 或 ref2va）" >&2
    exit 1
  fi
  log "启动 ComfyUI（image=${ADM_COMFYUI_IMAGE:-adm-comfyui-h3:20260915}）"
  docker compose -f "$COMPOSE_FILE" up -d
  wait_comfyui_ready
  log "提示：ComfyUI 内 模板库 → Video → MiniMax H3 T2V / I2V / R2V 即可加载官方工作流"
}

case "${1:-status}" in
  status)  require_docker; show_status ;;
  comfyui) require_docker; start_comfyui ;;
  sglang)
    require_docker
    stop_comfyui
    log "请回到 ADM-BE「模型列表」点击 MiniMax-H3 的「启动」（约 12 分钟就绪后即可用 /v1/videos API）"
    ;;
  stop)    require_docker; stop_comfyui; stop_sglang ;;
  *)       echo "用法: $0 {status|comfyui|sglang|stop}" >&2; exit 1 ;;
esac
