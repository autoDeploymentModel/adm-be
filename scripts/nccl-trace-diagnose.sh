#!/usr/bin/env bash
# NCCL 多机卡死诊断脚本：在节点 0（运行 ADM 的机器）上执行。
# 用法: bash nccl-trace-diagnose.sh [远端IP] [远端用户] [互连网卡]
# 示例: bash nccl-trace-diagnose.sh 192.168.177.12 lucas enp1s0f1np1
set -u
RANK1_IP="${1:-192.168.177.12}"
RANK1_USER="${2:-lucas}"
IFACE="${3:-enp1s0f1np1}"

CID=$(docker ps -q --filter name=adm-sglang | head -1)
if [ -z "$CID" ]; then
  echo "!! 未找到运行中的 adm-sglang 容器（可能是多机/未启动）"
  docker ps -a --filter name=adm-sglang
  exit 1
fi

echo "==== 1) 容器 NCCL/GLOO 环境变量（确认 NCCL_DEBUG=TRACE 是否生效）===="
docker inspect "$CID" --format '{{.Config.Env}}' | tr ',' '\n' | grep -iE "NCCL|GLOO" || echo "(无 NCCL env —— 参数未生效，检查是否填在「额外环境变量」且已重启)"

echo
echo "==== 2) 容器镜像与启动参数 ===="
docker inspect "$CID" --format 'image={{.Config.Image}}'
docker inspect "$CID" --format '{{.Config.Cmd}}' | tr ' ' '\n' | grep -E "disable-cuda-graph|nccl-port|node-rank" || echo "(未匹配到多机参数?)"
echo "-- 多机参数实际值（nccl-port/node-rank/nnodes/tp/dist-init-addr）--"
docker inspect "$CID" --format '{{.Config.Cmd}}' | grep -oE -- '--(nccl-port|node-rank|nnodes|tp|dist-init-addr)[ =][0-9.:]+' | sort -u || echo "(无)"

echo
echo "==== 2.1) 容器网络模式（多机必须 NetworkMode=host，否则跨机 TCP 黑洞卡死）===="
docker inspect "$CID" --format 'NetworkMode={{.HostConfig.NetworkMode}}'

echo
echo "==== 2.2) RoCE 形态与 gdrdrv 匹配（官方 2×Spark 配方：--device /dev/infiniband + memlock + IPC_LOCK）===="
if docker inspect "$CID" --format '{{.Config.Cmd}}' | grep -q "infiniband"; then
  echo "容器: 已挂 /dev/infiniband（RoCE 形态）"
else
  echo "容器: 未挂 /dev/infiniband（纯 Socket 形态——官方 DGX Spark recipe 用的是 RoCE）"
fi
if [ -d /dev/infiniband ]; then echo "主机 IB 设备: $(ls /dev/infiniband/ 2>/dev/null | tr '\n' ' ')"; else echo "主机 IB 设备: 无 /dev/infiniband"; fi
GH=$(docker inspect "$CID" --format '{{range .Config.Env}}{{println .}}{{end}}' | grep '^GDRCOPY_HOME=' || echo "GDRCOPY_HOME=未设")
echo "容器 ${GH}"
echo "主机 gdrdrv 源码: $(ls -d /usr/src/gdrdrv* 2>/dev/null | tr '\n' ' ' || echo 无)"

echo
echo "==== 3) 日志落盘 /tmp/sg_trace.log ===="
docker logs "$CID" > /tmp/sg_trace.log 2>&1
wc -l /tmp/sg_trace.log
echo "-- 运行时 NCCL 版本（Config.Env 的 NCCL_VERSION 是构建时烘焙值，以日志为准）--"
grep -m1 "NCCL version" /tmp/sg_trace.log || echo "(日志中未见 NCCL version 行)"

echo
echo "==== 4) NCCL socket 层关键行（后 40 行；仅 NCCL_DEBUG=TRACE 时可见 net 行）===="
DBGLVL=$(docker inspect "$CID" --format '{{.Config.Env}}' | tr ',' '\n' | grep -oE "NCCL_DEBUG=[A-Z]+\|NCCL_DEBUG=[a-z]+" | head -1 | cut -d= -f2)
if [ "${DBGLVL:-INFO}" = TRACE ]; then
  echo "(NCCL_DEBUG=TRACE：Listening/Connecting 判据可用)"
else
  echo "(注意：当前 NCCL_DEBUG=${DBGLVL:-未设}，Listening/Connecting 行被过滤，第 8 步判据不适用，以 ss/流量为准)"
fi
grep -nE "NET/Socket|Listening|Connecting|connect to|Init COMPLETE|Failed to initialize" /tmp/sg_trace.log | tail -40

echo
echo "==== 3.1) 通道数与卡点时长（日志带时间戳行 vs 当前时间）===="
CH=$(grep -cE "Channel [0-9]+/0 :" /tmp/sg_trace.log)
echo "channel 建连打点行数: ${CH:-0}（<16 通道建连未展开；>16 且无 Init COMPLETE = 建连中途卡）"
LAST_TS=$(grep -oE "\[20[0-9]{2}-[0-9]{2}-[0-9]{2} [0-9]{2}:[0-9]{2}:[0-9]{2}" /tmp/sg_trace.log | tail -1)
echo "日志最后时间戳: ${LAST_TS:-（无墙钟行，NCCL 行不带时间）}    当前时间: $(date '+%Y-%m-%d %H:%M:%S')"
if grep -q "Uvicorn running" /tmp/sg_trace.log; then echo "Uvicorn running: 已出现（服务就绪）"; else echo "Uvicorn running: 未出现"; fi
if grep -q "Init COMPLETE" /tmp/sg_trace.log; then echo "NCCL Init COMPLETE: 已出现"; else echo "NCCL Init COMPLETE: 未出现"; fi

echo
echo "==== 5) 卡点现场：日志最后 25 行 ===="
tail -25 /tmp/sg_trace.log

echo
echo "==== 5.1) SGLang worker 进程环境（与裸 nccl_test 对比，抓 SGLang 注入的差异项）===="
WPRCS=$(docker top "$CID" 2>/dev/null | awk 'NR>1 && $0 ~ /python/ {print $2}' | head -5)
if [ -n "$WPRCS" ]; then
  for WPID in $WPRCS; do
    echo "-- python pid=$WPID（容器内）--"
    docker exec -i "$CID" sh -c "tr '\\0' '\\n' < /proc/$WPID/environ" 2>/dev/null \
      | grep -E "^(NCCL|CUDA_VISIBLE|CUMEM|NVLS|GDR|SGLANG|MASTER|RANK|WORLD|LOCAL_|OMPI|PMI|TORCH|NVIDIA)" \
      | sort || echo "(读取失败——容器内无权限或进程已退出)"
  done
else
  echo "(未找到 python 进程)"
fi

echo
echo "==== 6) 本机直连网卡流量（对比两次输出字节数）===="
ip -s link show "$IFACE" | grep -A1 -E "RX:|TX:"
sleep 5
ip -s link show "$IFACE" | grep -A1 -E "RX:|TX:"
echo "-- 网卡 link/speed --"
ethtool "$IFACE" 2>/dev/null | grep -E "Speed|Link detected" || echo "(ethtool 不可用)"
echo "-- 本机非回环 TCP 状态（SYN-SENT 堆积=对端端口被拦；无 LISTEN=没绑上）--"
SS_ALL=$(ss -tna | grep -vE "127.0.0.1|::1")
echo "$SS_ALL" | tail -12

echo
echo "==== 7) 远端节点状态（日志尾部 + 网卡流量）===="
ssh -o ConnectTimeout=5 -o BatchMode=yes "$RANK1_USER@$RANK1_IP" \
  'CID=$(docker ps -q --filter name=adm-sglang | head -1)
   if [ -n "$CID" ]; then echo "[远端 NET/Socket 选卡]"; docker logs "$CID" 2>&1 | grep -E "NET/Socket : Using" | tail -2; echo "[远端容器日志尾部]"; docker logs "$CID" 2>&1 | tail -8; echo "[远端网络模式]"; docker inspect "$CID" --format "NetworkMode={{.HostConfig.NetworkMode}}"; fi
   echo "[远端网卡流量]"; ip -s link show '"$IFACE"' | grep -A1 -E "RX:|TX:"
   echo "[远端 link/speed]"; ethtool '"$IFACE"' 2>/dev/null | grep -E "Speed|Link detected" || echo "(ethtool 不可用)"
   echo "[远端非回环 TCP 状态]"; ss -tna | grep -vE "127.0.0.1|::1" | tail -12' \
  || echo "(远端 SSH 失败——检查免密与 IP)"

echo
echo "==== 8) 自动判定（基于本次采集数据；Listening/Connecting 判据仅 TRACE 可见，INFO 以 ss/流量/Init COMPLETE 为准）===="
SYN=$(echo "$SS_ALL" | grep -c "SYN-SENT" || true)
EST=$(echo "$SS_ALL" | grep -c ESTAB || true)
if grep -q "Uvicorn running" /tmp/sg_trace.log; then
  echo ">>> 服务已就绪：Uvicorn running 已出现，数据面正常，无需排查"
elif grep -q "Init COMPLETE" /tmp/sg_trace.log; then
  echo ">>> NCCL 初始化已完成（Init COMPLETE 出现），卡点在后续阶段（权重加载/调度器）；看 5) 尾部是否还有进展"
else
  echo ">>> 判定：NCCL 未完成初始化（无 Init COMPLETE）"
  if [ "$SYN" -gt 0 ]; then
    echo "  - ss 存在 SYN-SENT x$SYN：对端端口被防火墙/NAT 静默丢弃，查 nccl/数据端口放行"
  else
    echo "  - ss 无 SYN-SENT（本地跨机 ESTAB x${EST}）=> 端口层无碍，卡在 NCCL 内部协商"
  fi
  if grep -q "NET/IB : No device found" /tmp/sg_trace.log && ! docker inspect "$CID" --format '{{.Config.Cmd}}' | grep -q infiniband; then
    echo "  - 纯 Socket 形态（未挂 /dev/infiniband）：建议开 use_roce 走官方 2×Spark 配方"
  fi
  GH=$(docker inspect "$CID" --format '{{range .Config.Env}}{{println .}}{{end}}' | grep '^GDRCOPY_HOME=' | head -1 | cut -d= -f2)
  if [ -n "$GH" ] && ! ls -d /usr/src/gdrdrv* 2>/dev/null | grep -q "${GH##*/}"; then
    echo "  - gdrdrv 版本与容器不一致（容器 $GH，主机无匹配）：GDRCopy 静默失败可能卡，需对齐"
  fi
  docker inspect "$CID" --format '{{.Config.Cmd}}' | grep -q -- '--nccl-port' && echo "  - 仍在传 --nccl-port：官方多机示例从不传它，建议设 0 对比"
  grep -q "CustomAllreduce is disabled" /tmp/sg_trace.log && echo "  - 已进入 SGLang 组初始化：隔离测试通过时用 5.1 对比 worker env（2.29.7 镜像通过 vs 2.28.3/2.30.7 失败）"
fi