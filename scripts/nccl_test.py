#!/usr/bin/env python3
"""NCCL 双机隔离测试（在 SGLang 容器内运行，宿主无 torch）。

用法（容器未停止、6464 被占时换端口 6465，两端一致；不必停容器）：

  rank 0 (本机 192.168.177.11)：
    CID=$(docker ps -q --filter name=adm-sglang | head -1)
    docker exec -i "$CID" env NCCL_DEBUG=TRACE NCCL_MAX_NCHANNELS=8 \
      NCCL_SOCKET_IFNAME=enp1s0f1np1 python3 - 0 6465 [模式] < scripts/nccl_test.py

  rank 1 (远端 192.168.177.12) 同时：
    ssh lucas@192.168.177.12 'CID=$(docker ps -q --filter name=adm-sglang | head -1); \
      docker exec -i "$CID" env NCCL_DEBUG=TRACE NCCL_MAX_NCHANNELS=8 \
      NCCL_SOCKET_IFNAME=enp1s0f1np1 python3 - 1 6465 [模式]' < scripts/nccl_test.py

模式：
  nccl （默认）  单一 NCCL 组（已跑通：Init COMPLETE + SUCCESS）
  dual           复刻 SGLang 序列：先 gloo 默认组，再 new_group(backend=nccl)+all_reduce
                 —— 用于定位 SGLang 卡死是否来自双进程组/NCCL 组创建路径
                 单 NCCL 组已通而 dual 卡 => 锁死 SGLang 组初始化；dual 也通 => 问题在
                 SGLang 启动器注入的其它环境/参数（如 NCCL_PORT，可用 env NCCL_PORT=x 复现）

判定：
  "Init COMPLETE" + "SUCCESS: tensor([3.], device='cuda:0')" => 该路径正常
  卡在 Channel 打点后、无 Init COMPLETE、无流量 => 该路径存在与 SGLang 相同死锁
"""
import sys
import torch
import torch.distributed as dist

RANK = int(sys.argv[1]) if len(sys.argv) > 1 else 0
PORT = int(sys.argv[2]) if len(sys.argv) > 2 else 6464
MODE = sys.argv[3] if len(sys.argv) > 3 else "nccl"
WORLD_SIZE = 2
INIT_METHOD = f"tcp://192.168.177.11:{PORT}"

print(f"[Rank {RANK}] 初始化（模式={MODE}）...", flush=True)
dist.init_process_group(
    backend="gloo" if MODE == "dual" else "nccl",
    init_method=INIT_METHOD,
    rank=RANK,
    world_size=WORLD_SIZE,
)

if MODE == "dual":
    print(f"[Rank {RANK}] gloo 组就绪，创建 NCCL 组...", flush=True)
    nccl_group = dist.new_group(backend="nccl")
    t = torch.tensor([float(RANK + 1)]).cuda()
    print(f"[Rank {RANK}] NCCL 组就绪，开始 all-reduce...", flush=True)
    dist.all_reduce(t, group=nccl_group)
    print(f"[Rank {RANK}] SUCCESS: {t}", flush=True)
else:
    print(f"[Rank {RANK}] NCCL 初始化完成，开始 all-reduce...", flush=True)
    t = torch.tensor([float(RANK + 1)]).cuda()
    dist.all_reduce(t)
    print(f"[Rank {RANK}] SUCCESS: {t}", flush=True)

dist.destroy_process_group()
print(f"[Rank {RANK}] 完成", flush=True)