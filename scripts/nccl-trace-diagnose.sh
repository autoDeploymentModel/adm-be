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

echo
echo "==== 2.1) 容器网络模式（多机必须 NetworkMode=host，否则跨机 TCP 黑洞卡死）===="
docker inspect "$CID" --format 'NetworkMode={{.HostConfig.NetworkMode}}'

echo
echo "==== 3) 日志落盘 /tmp/sg_trace.log ===="
docker logs "$CID" > /tmp/sg_trace.log 2>&1
wc -l /tmp/sg_trace.log

echo
echo "==== 4) NCCL socket 层关键行（后 40 行）===="
grep -nE "NET/Socket|Listening|Connecting|connect to|Init COMPLETE|Failed to initialize" /tmp/sg_trace.log | tail -40

echo
echo "==== 5) 卡点现场：日志最后 25 行 ===="
tail -25 /tmp/sg_trace.log

echo
echo "==== 6) 本机直连网卡流量（对比两次输出字节数）===="
ip -s link show "$IFACE" | grep -A1 -E "RX:|TX:"
sleep 5
ip -s link show "$IFACE" | grep -A1 -E "RX:|TX:"
echo "-- 网卡 link/speed --"
ethtool "$IFACE" 2>/dev/null | grep -E "Speed|Link detected" || echo "(ethtool 不可用)"
echo "-- 本机非回环 TCP 状态（SYN-SENT 堆积=对端端口被拦；无 LISTEN=没绑上）--"
ss -tna | grep -vE "127.0.0.1|::1" | tail -12

echo
echo "==== 7) 远端节点状态（日志尾部 + 网卡流量）===="
ssh -o ConnectTimeout=5 -o BatchMode=yes "$RANK1_USER@$RANK1_IP" \
  'CID=$(docker ps -q --filter name=adm-sglang | head -1)
   if [ -n "$CID" ]; then echo "[远端容器日志尾部]"; docker logs "$CID" 2>&1 | tail -8; echo "[远端网络模式]"; docker inspect "$CID" --format "NetworkMode={{.HostConfig.NetworkMode}}"; fi
   echo "[远端网卡流量]"; ip -s link show '"$IFACE"' | grep -A1 -E "RX:|TX:"
   echo "[远端 link/speed]"; ethtool '"$IFACE"' 2>/dev/null | grep -E "Speed|Link detected" || echo "(ethtool 不可用)"
   echo "[远端非回环 TCP 状态]"; ss -tna | grep -vE "127.0.0.1|::1" | tail -12' \
  || echo "(远端 SSH 失败——检查免密与 IP)"

echo
echo "==== 8) 结论速查 ===="
echo "第 4 步出现 \"NET/Socket : Listening on\" 且第 6/7 步字节数增长 => 数据面正常，等 Uvicorn"
echo "第 4 步无 Listening/Connecting 或只有一侧 => NCCL socket 卡死，按序排查："
echo "  1) 2.1 是否 host？非 host 必卡，改 --network host"
echo "  2) 6/7 的 ss 是否 SYN-SENT 堆积？有 => nccl 端口被防火墙/NAT 拦（6464 能通不代表数据口通）"
echo "  3) ethtool 无 Link detected / speed 异常 => 光口链路问题"
echo "  4) 以上全正常 => 跑 nccl_test.py 双机隔离；仍卡=网络栈，通过=换回 v0.5.18 镜像对比"
echo "  贴 2.1/4/5/6/7 输出"