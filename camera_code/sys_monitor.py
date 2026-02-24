#!/usr/bin/env python3
"""
system_monitor_abs.py

Monitors system + (optionally) a specific process every N seconds.
Outputs BOTH:
  1) Absolute (cumulative since boot) CPU time counters (system)
  2) Per-interval deltas (CPU seconds used over the last interval)
Also outputs:
  - CPU percent (system + process)
  - RAM (percent + absolute MB)
  - Optional per-core absolute counters (can be heavy)

Requires: psutil
  pip install psutil

Examples:
  python system_monitor_abs.py
  python system_monitor_abs.py --interval 30 --csv cpu_log.csv
  python system_monitor_abs.py --pid 12345 --csv cpu_log.csv
"""

import time
import csv
import argparse
from datetime import datetime
from typing import Optional, Dict, Any

import psutil


def _cpu_times_dict(t) -> Dict[str, float]:
    # psutil returns a namedtuple with platform-specific fields
    d = t._asdict()
    # ensure keys exist across platforms
    for k in ["user", "system", "idle", "iowait", "nice", "irq", "softirq", "steal", "guest", "guest_nice"]:
        d.setdefault(k, 0.0)
    return {k: float(v) for k, v in d.items()}


def _delta_times(curr: Dict[str, float], prev: Dict[str, float]) -> Dict[str, float]:
    out = {}
    for k in curr.keys():
        out[k] = float(curr.get(k, 0.0) - prev.get(k, 0.0))
    return out


def _sum_cpu_time(d: Dict[str, float]) -> float:
    # total CPU time across all categories (one aggregate CPU in psutil terms)
    return float(sum(d.values()))


def _safe_process(pid: int) -> Optional[psutil.Process]:
    try:
        p = psutil.Process(pid)
        # prime once so later cpu_percent() behaves predictably
        p.cpu_percent(interval=None)
        return p
    except Exception:
        return None


def monitor(interval: int = 30, csv_path: Optional[str] = None, pid: Optional[int] = None, per_core: bool = False):
    interval = max(1, int(interval))

    proc = _safe_process(pid) if pid is not None else None
    if pid is not None and proc is None:
        print(f"[WARN] Could not attach to PID={pid}. Will monitor system only.")
        pid = None

    # Prime cpu_percent for system
    psutil.cpu_percent(interval=None)

    csv_file = None
    writer = None
    if csv_path:
        csv_file = open(csv_path, "w", newline="")
        writer = csv.writer(csv_file)
        writer.writerow([
            "timestamp",

            # system CPU (absolute since boot)
            "sys_user_abs_s", "sys_system_abs_s", "sys_idle_abs_s", "sys_iowait_abs_s",
            "sys_total_abs_s",

            # system CPU (delta over interval)
            "sys_user_delta_s", "sys_system_delta_s", "sys_idle_delta_s", "sys_iowait_delta_s",
            "sys_total_delta_s",

            # system CPU percent (instant)
            "sys_cpu_percent",

            # memory
            "mem_percent", "mem_used_mb", "mem_avail_mb",

            # optional process metrics
            "proc_cpu_percent",
            "proc_user_abs_s", "proc_system_abs_s",
            "proc_user_delta_s", "proc_system_delta_s",
            "proc_rss_mb",
        ])
        csv_file.flush()
        print(f"[INFO] Logging CSV -> {csv_path}")

    print(f"[INFO] Interval: {interval}s | per_core={per_core} | pid={pid if pid else 'None'}")
    print("[INFO] Press Ctrl+C to stop.\n")

    # Baselines (absolute)
    prev_sys = _cpu_times_dict(psutil.cpu_times())

    prev_proc = None
    if proc is not None:
        try:
            pt = proc.cpu_times()
            prev_proc = {"user": float(pt.user), "system": float(pt.system)}
        except Exception:
            prev_proc = None

    # Optional per-core baselines
    prev_cores = None
    if per_core:
        prev_cores = [_cpu_times_dict(t) for t in psutil.cpu_times(percpu=True)]

    try:
        while True:
            # sleep first so "delta" corresponds to interval window
            time.sleep(interval)

            ts = datetime.now().strftime("%Y-%m-%d %H:%M:%S")

            # System CPU absolute
            curr_sys = _cpu_times_dict(psutil.cpu_times())
            delta_sys = _delta_times(curr_sys, prev_sys)

            sys_abs_total = _sum_cpu_time(curr_sys)
            sys_delta_total = _sum_cpu_time(delta_sys)

            # System CPU percent over ~instant (psutil keeps internal delta)
            sys_cpu_pct = psutil.cpu_percent(interval=None)

            # Memory absolute
            vm = psutil.virtual_memory()
            mem_percent = float(vm.percent)
            mem_used_mb = float(vm.used) / (1024 * 1024)
            mem_avail_mb = float(vm.available) / (1024 * 1024)

            # Process metrics
            proc_cpu_pct = ""
            proc_user_abs = ""
            proc_sys_abs = ""
            proc_user_delta = ""
            proc_sys_delta = ""
            proc_rss_mb = ""

            if proc is not None:
                try:
                    proc_cpu_pct = proc.cpu_percent(interval=None)  # percent since last call
                    pt = proc.cpu_times()
                    curr_proc = {"user": float(pt.user), "system": float(pt.system)}
                    if prev_proc is not None:
                        dproc = _delta_times(curr_proc, prev_proc)
                        proc_user_delta = dproc["user"]
                        proc_sys_delta = dproc["system"]
                    proc_user_abs = curr_proc["user"]
                    proc_sys_abs = curr_proc["system"]
                    prev_proc = curr_proc

                    proc_rss_mb = float(proc.memory_info().rss) / (1024 * 1024)
                except (psutil.NoSuchProcess, psutil.AccessDenied):
                    print(f"[WARN] Lost PID={pid}. Process metrics disabled.")
                    proc = None

            # Optional per-core reporting (absolute + delta)
            core_lines = []
            if per_core and prev_cores is not None:
                curr_cores = [_cpu_times_dict(t) for t in psutil.cpu_times(percpu=True)]
                for i, (c, p) in enumerate(zip(curr_cores, prev_cores)):
                    d = _delta_times(c, p)
                    core_lines.append(
                        f"  core{i}: user {c['user']:.1f}s (Δ{d['user']:.2f}) | "
                        f"sys {c['system']:.1f}s (Δ{d['system']:.2f}) | "
                        f"idle {c['idle']:.1f}s (Δ{d['idle']:.2f})"
                    )
                prev_cores = curr_cores

            # Print summary
            print("------------------------------------------------------------")
            print(f"[{ts}]")
            print(f"System CPU %: {sys_cpu_pct:.1f}%")
            print(f"System CPU abs (s since boot): user={curr_sys['user']:.2f} system={curr_sys['system']:.2f} "
                  f"idle={curr_sys['idle']:.2f} iowait={curr_sys.get('iowait',0.0):.2f} total={sys_abs_total:.2f}")
            print(f"System CPU delta (last {interval}s): user={delta_sys['user']:.2f} system={delta_sys['system']:.2f} "
                  f"idle={delta_sys['idle']:.2f} iowait={delta_sys.get('iowait',0.0):.2f} total={sys_delta_total:.2f}")
            print(f"Memory: {mem_percent:.1f}% | used={mem_used_mb:.1f} MB | avail={mem_avail_mb:.1f} MB")

            if proc is not None:
                print(f"Process (PID {pid}) CPU %: {float(proc_cpu_pct):.1f}%")
                print(f"Process CPU abs: user={float(proc_user_abs):.2f}s system={float(proc_sys_abs):.2f}s")
                if proc_user_delta != "" and proc_sys_delta != "":
                    print(f"Process CPU delta (last {interval}s): user={float(proc_user_delta):.2f}s system={float(proc_sys_delta):.2f}s")
                print(f"Process RSS: {float(proc_rss_mb):.1f} MB")

            if core_lines:
                print("Per-core (abs + delta):")
                for line in core_lines:
                    print(line)

            # CSV
            if writer is not None:
                writer.writerow([
                    ts,
                    f"{curr_sys['user']:.6f}",
                    f"{curr_sys['system']:.6f}",
                    f"{curr_sys['idle']:.6f}",
                    f"{curr_sys.get('iowait', 0.0):.6f}",
                    f"{sys_abs_total:.6f}",

                    f"{delta_sys['user']:.6f}",
                    f"{delta_sys['system']:.6f}",
                    f"{delta_sys['idle']:.6f}",
                    f"{delta_sys.get('iowait', 0.0):.6f}",
                    f"{sys_delta_total:.6f}",

                    f"{sys_cpu_pct:.3f}",
                    f"{mem_percent:.3f}",
                    f"{mem_used_mb:.3f}",
                    f"{mem_avail_mb:.3f}",

                    f"{proc_cpu_pct:.3f}" if proc_cpu_pct != "" else "",
                    f"{proc_user_abs:.6f}" if proc_user_abs != "" else "",
                    f"{proc_sys_abs:.6f}" if proc_sys_abs != "" else "",
                    f"{proc_user_delta:.6f}" if proc_user_delta != "" else "",
                    f"{proc_sys_delta:.6f}" if proc_sys_delta != "" else "",
                    f"{proc_rss_mb:.3f}" if proc_rss_mb != "" else "",
                ])
                csv_file.flush()

            prev_sys = curr_sys

    except KeyboardInterrupt:
        print("\n[INFO] Stopped.")

    finally:
        if csv_file is not None:
            csv_file.close()


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("--interval", type=int, default=30, help="Seconds between samples (default 30)")
    ap.add_argument("--csv", type=str, default=None, help="Optional CSV output path")
    ap.add_argument("--pid", type=int, default=None, help="Optional process PID to monitor")
    ap.add_argument("--per-core", action="store_true", help="Also print per-core absolute + delta CPU times")

    args = ap.parse_args()
    monitor(interval=args.interval, csv_path=args.csv, pid=args.pid, per_core=args.per_core)