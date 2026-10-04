"""Opt-in local VieNeu smoke. Never downloads models or changes app settings."""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import time
import threading


def peak_working_set(process: subprocess.Popen[str]) -> int | None:
    if os.name != "nt":
        return None
    import ctypes
    from ctypes import wintypes
    class Counters(ctypes.Structure):
        _fields_ = [("cb", wintypes.DWORD), ("faults", wintypes.DWORD)] + [
            (key, ctypes.c_size_t) for key in ("peak", "working", "peakPaged", "paged", "peakNonPaged",
                                             "nonPaged", "pagefile", "peakPagefile")]
    counters = Counters()
    counters.cb = ctypes.sizeof(counters)
    # Windows venv python.exe is a redirector. Include its actual Python child.
    class Entry(ctypes.Structure):
        _fields_ = [("size", wintypes.DWORD), ("usage", wintypes.DWORD), ("pid", wintypes.DWORD),
                    ("heap", ctypes.c_size_t), ("module", wintypes.DWORD), ("threads", wintypes.DWORD),
                    ("parent", wintypes.DWORD), ("priority", wintypes.LONG), ("flags", wintypes.DWORD),
                    ("exe", wintypes.WCHAR * 260)]
    kernel = ctypes.windll.kernel32
    kernel.CreateToolhelp32Snapshot.restype = wintypes.HANDLE
    kernel.OpenProcess.restype = wintypes.HANDLE
    snapshot = kernel.CreateToolhelp32Snapshot(2, 0)
    entry = Entry()
    entry.size = ctypes.sizeof(entry)
    parents = {}
    try:
        ok = kernel.Process32FirstW(wintypes.HANDLE(snapshot), ctypes.byref(entry))
        while ok:
            parents[entry.pid] = entry.parent
            ok = kernel.Process32NextW(wintypes.HANDLE(snapshot), ctypes.byref(entry))
    finally:
        kernel.CloseHandle(wintypes.HANDLE(snapshot))
    pids = {process.pid}
    while True:
        descendants = {pid for pid, parent in parents.items() if parent in pids}
        if descendants.issubset(pids):
            break
        pids.update(descendants)
    total = 0
    for pid in pids:
        handle = kernel.OpenProcess(0x0410, False, pid)
        if not handle:
            continue
        try:
            if ctypes.windll.psapi.GetProcessMemoryInfo(wintypes.HANDLE(handle), ctypes.byref(counters), counters.cb):
                total += counters.peak
        finally:
            kernel.CloseHandle(wintypes.HANDLE(handle))
    return total or None


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cache", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--count", type=int, default=4, choices=range(1, 101))
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    phrases = ["Ma Đế.", "Cảnh giới này thật đáng kinh ngạc.", "Thánh Nhân.", "Đại Đế đã trở về, cuối cùng chúng ta cũng gặp lại nhau."]
    request = {"operation": "tts_generate", "cacheDir": str(args.cache.resolve()), "device": "cpu",
               "segments": [{"id": str(i), "text": text, "voice": "Ngọc Huyền",
                             "outputPath": str((args.output / f"raw-{i}.wav").resolve())}
                            for i, text in enumerate(phrases * 25)][:args.count]}
    env = {**os.environ, "HF_HUB_OFFLINE": "1", "HF_HUB_DISABLE_TELEMETRY": "1",
           "PYTHONPATH": str(Path(__file__).resolve().parents[1] / "python")}
    started = time.monotonic()
    events = []
    peak = 0
    with (args.output / "worker-stderr.log").open("w", encoding="utf-8") as log:
        with subprocess.Popen([sys.executable, "-m", "worker"], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                              stderr=log, encoding="utf-8", env=env) as process:
            timer = threading.Timer(600, process.kill)
            timer.start()
            try:
                process.stdin.write(json.dumps(request, ensure_ascii=False) + "\n")
                process.stdin.close()
                for line in process.stdout:
                    if line.strip():
                        events.append(json.loads(line))
                        peak = max(peak, peak_working_set(process) or 0)
                process.wait()
            finally:
                timer.cancel()
    completed = [e for e in events if e.get("type") == "segment_completed"]
    failures = [e for e in events if e.get("type") in {"error", "segment_failed"}]
    report = {"elapsedSeconds": time.monotonic() - started, "peakWorkerTreeRamBytes": peak or None,
              "completed": completed, "failures": failures}
    (args.output / "report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps({**report, "completed": len(completed)}, ensure_ascii=False, indent=2))
    if process.returncode or failures or len(completed) != args.count:
        raise SystemExit("Local VieNeu smoke failed; see worker-stderr.log")


if __name__ == "__main__":
    main()
