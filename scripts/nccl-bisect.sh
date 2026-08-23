#!/usr/bin/env bash
# NCCL 双机二分实验（节点 0 上执行）：本机 docker exec + 远端 ssh 同时启动，
# 复现 SGLang 卡死路径，判定隔离（nccl 单组 / dual 双组 / NCCL_PORT 注入）。
# 用法: bash nccl-bisect.sh [模式] [端口] [额外env]
#   模式    nccl（默认，实验1/基线） | dual（实验2，复刻 SGLang gloo+NCCL 双组）
#   端口    默认 6465（避开 SGLang 占用的 6464）
#   额外env 如 NCCL_PORT=6467（实验1 注入嫌疑）；多组用空格分隔（会做词分割）
# 远端默认 lucas@192.168.177.12，可 export RANK1=user@ip 覆盖
set -u
MODE="${1:-nccl}"
PORT="${2:-6465}"
EXTRA_ENV="${3:-}"
RANK1="${RANK1:-lucas@192.168.177.12}"
HERE="$(cd "$(dirname "$0")" && pwd)"
SCRIPT="$HERE/nccl_test.py"
TIMEOUT=90

[ -f "$SCRIPT" ] || { echo "!! 找不到 $SCRIPT"; exit 1; }

LOCAL_CID=$(docker ps -q --filter name=adm-sglang | head -1)
if [ -z "$LOCAL_CID" ]; then echo "!! 本机未找到 adm-sglang 容器"; exit 1; fi

echo "== 实验: 模式=$MODE 端口=$PORT 额外env=[${EXTRA_ENV:-无}] 远端=$RANK1 超时=${TIMEOUT}s =="

# 远端命令（$EXTRA_ENV 在本机展开后注入远端 env，注意引号）
REMOTE_CMD="CID=\$(docker ps -q --filter name=adm-sglang | head -1); [ -n \"\$CID\" ] || exit 2; timeout $TIMEOUT docker exec -i \"\$CID\" env $EXTRA_ENV NCCL_DEBUG=TRACE NCCL_MAX_NCHANNELS=8 NCCL_SOCKET_IFNAME=enp1s0f1np1 python3 - 1 $PORT $MODE"

echo "-- 启动远端 rank 1 --"
ssh -o ConnectTimeout=5 -o BatchMode=yes "$RANK1" "$REMOTE_CMD" \
  < "$SCRIPT" > /tmp/nccl_bisect_rank1.log 2>&1 &
REMOTE_PID=$!

echo "-- 启动本机 rank 0 --"
timeout "$TIMEOUT" docker exec -i "$LOCAL_CID" env $EXTRA_ENV NCCL_DEBUG=TRACE \
  NCCL_MAX_NCHANNELS=8 NCCL_SOCKET_IFNAME=enp1s0f1np1 python3 - 0 "$PORT" "$MODE" \
  < "$SCRIPT" > /tmp/nccl_bisect_rank0.log 2>&1 &
LOCAL_PID=$!

wait "$REMOTE_PID" "$LOCAL_PID"
R0=$?
R1=$?

echo
echo "== 结果 =="
echo "rank0 退出码=$R0  rank1 退出码=$R1  （124=超时卡死，2=远端容器缺失）"
for f in rank0 rank1; do
  if grep -q "SUCCESS: tensor(\[3.\]" "/tmp/nccl_bisect_$f.log"; then
    echo "$f: 通过（SUCCESS）"
  else
    echo "$f: 未通过/卡死 —— 尾部 12 行："
    tail -12 "/tmp/nccl_bisect_$f.log"
  fi
done
echo
echo "完整日志: /tmp/nccl_bisect_rank0.log /tmp/nccl_bisect_rank1.log"
echo "判定: 两端 SUCCESS => 该路径正常；任一端卡 Channel 后无 Init COMPLETE => 复现"