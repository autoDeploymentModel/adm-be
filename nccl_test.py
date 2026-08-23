#!/usr/bin/env python3
"""NCCL all-reduce 测试：在两台机器上同时运行，验证跨机 NCCL TCP 通信是否正常。

用法：
  rank 0 (192.168.177.11) 上执行：
    NCCL_DEBUG=INFO NCCL_SOCKET_IFNAME=enp1s0f1np1 python3 nccl_test.py 0

  rank 1 (192.168.177.12) 上同时执行：
    NCCL_DEBUG=INFO NCCL_SOCKET_IFNAME=enp1s0f1np1 python3 nccl_test.py 1

成功输出：SUCCESS: tensor([3.], device='cuda:0')
"""
import sys
import torch
import torch.distributed as dist

RANK = int(sys.argv[1]) if len(sys.argv) > 1 else 0
WORLD_SIZE = 2
INIT_METHOD = "tcp://192.168.177.11:6464"

print(f"[Rank {RANK}] 初始化 NCCL...", flush=True)
dist.init_process_group(
    backend="nccl",
    init_method=INIT_METHOD,
    rank=RANK,
    world_size=WORLD_SIZE,
)

print(f"[Rank {RANK}] NCCL 初始化完成，开始 all-reduce...", flush=True)
t = torch.tensor([float(RANK + 1)]).cuda()
dist.all_reduce(t)
print(f"[Rank {RANK}] SUCCESS: {t}", flush=True)

dist.destroy_process_group()
print(f"[Rank {RANK}] 完成", flush=True)
