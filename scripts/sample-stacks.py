"""Samples one thread of a running native build with unwound stacks, then
names the frames through the build's linker map (NATIVEBOAT_LINK_MAP).

Each sample is weighted by the CPU cycles the thread used since the one
before, so waits (a blocked GDI call, an idle message loop) count by the
processor time they took, not by how long they lasted. With the module map
(NATIVEBOAT_MODULE_MAP), compiled JavaScript is also summed by source file.

usage: sample-stacks.py --pid PID --map MarkText.map --out report.txt
           [--seconds 5] [--ready FILE] [--thread TID] [--modules modules.txt]
       sample-stacks.py --again report.txt --out report2.txt [--modules ...]

The report lists frames by samples (inclusive and leaf), by CPU share
(inclusive and leaf), source files by CPU share, and the commonest stack
tops; REPORT.json keeps every sample's frames, leaf first, for other
questions, and REPORT.cycles.json their weights. --ready names a file written once sampling starts, so a driver
can start its input then. Only reads the process; never suspends it longer
than one stack walk.
"""
import argparse
import bisect
import collections
import ctypes
import ctypes.wintypes as wt
import json
import re
import time

parser = argparse.ArgumentParser()
parser.add_argument("--pid", type=int)
parser.add_argument("--map")
parser.add_argument("--out", required=True)
parser.add_argument("--seconds", type=float, default=5.0)
parser.add_argument("--ready")
parser.add_argument("--thread", type=int, help="thread id; the process's first thread by default")
parser.add_argument("--modules", help="NATIVEBOAT_MODULE_MAP output")
parser.add_argument("--again", help="summarize an earlier REPORT's samples instead of sampling")
args = parser.parse_args()
if not args.again and not (args.pid and args.map):
    parser.error("--pid and --map are needed to sample")

kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
dbghelp = ctypes.WinDLL("dbghelp", use_last_error=True)
psapi = ctypes.WinDLL("psapi", use_last_error=True)
kernel32.OpenProcess.restype = wt.HANDLE
kernel32.OpenThread.restype = wt.HANDLE
kernel32.CreateToolhelp32Snapshot.restype = wt.HANDLE


class THREADENTRY32(ctypes.Structure):
    _fields_ = [("dwSize", wt.DWORD), ("cntUsage", wt.DWORD), ("th32ThreadID", wt.DWORD),
                ("th32OwnerProcessID", wt.DWORD), ("tpBasePri", wt.LONG), ("tpDeltaPri", wt.LONG),
                ("dwFlags", wt.DWORD)]


class ADDRESS64(ctypes.Structure):
    _fields_ = [("Offset", ctypes.c_uint64), ("Segment", wt.WORD), ("Mode", ctypes.c_int)]


class STACKFRAME64(ctypes.Structure):
    _fields_ = [("AddrPC", ADDRESS64), ("AddrReturn", ADDRESS64), ("AddrFrame", ADDRESS64),
                ("AddrStack", ADDRESS64), ("AddrBStore", ADDRESS64), ("FuncTableEntry", ctypes.c_void_p),
                ("Params", ctypes.c_uint64 * 4), ("Far", wt.BOOL), ("Virtual", wt.BOOL),
                ("Reserved", ctypes.c_uint64 * 3), ("KdHelp", ctypes.c_uint64 * 24)]


def first_thread(pid):
    snap = kernel32.CreateToolhelp32Snapshot(4, 0)
    entry = THREADENTRY32()
    entry.dwSize = ctypes.sizeof(entry)
    ok = kernel32.Thread32First(snap, ctypes.byref(entry))
    while ok:
        if entry.th32OwnerProcessID == pid:
            kernel32.CloseHandle(snap)
            return entry.th32ThreadID
        ok = kernel32.Thread32Next(snap, ctypes.byref(entry))
    raise SystemExit(f"process {pid} has no threads")


def sample():
    """Samples the thread and names the frames."""
    process = kernel32.OpenProcess(0x001F0FFF, False, args.pid)
    dbghelp.SymSetOptions(0x4)
    dbghelp.SymInitializeW(process, None, True)
    dbghelp.SymFunctionTableAccess64.restype = ctypes.c_void_p
    dbghelp.SymFunctionTableAccess64.argtypes = [wt.HANDLE, ctypes.c_uint64]
    dbghelp.SymGetModuleBase64.restype = ctypes.c_uint64
    dbghelp.SymGetModuleBase64.argtypes = [wt.HANDLE, ctypes.c_uint64]
    table_access = ctypes.WINFUNCTYPE(ctypes.c_void_p, wt.HANDLE, ctypes.c_uint64)(dbghelp.SymFunctionTableAccess64)
    module_base = ctypes.WINFUNCTYPE(ctypes.c_uint64, wt.HANDLE, ctypes.c_uint64)(dbghelp.SymGetModuleBase64)
    thread = kernel32.OpenThread(0x001F03FF, False, args.thread or first_thread(args.pid))

    samples = []
    weights = []
    cycles = ctypes.c_uint64()
    kernel32.QueryThreadCycleTime(thread, ctypes.byref(cycles))
    last_cycles = cycles.value
    if args.ready:
        open(args.ready, "w").write("ready")
    deadline = time.time() + args.seconds
    # CONTEXT is 1232 bytes and 16-byte aligned.
    buffer = ctypes.create_string_buffer(1232 + 16)
    context = (ctypes.addressof(buffer) + 15) & ~15
    while time.time() < deadline:
        if kernel32.SuspendThread(thread) == 0xFFFFFFFF:
            break
        ctypes.memset(context, 0, 1232)
        ctypes.c_uint32.from_address(context + 0x30).value = 0x10000B  # CONTEXT_FULL
        kernel32.GetThreadContext(thread, ctypes.c_void_p(context))
        frame = STACKFRAME64()
        frame.AddrPC.Offset = ctypes.c_uint64.from_address(context + 0xF8).value  # Rip
        frame.AddrStack.Offset = ctypes.c_uint64.from_address(context + 0x98).value  # Rsp
        frame.AddrFrame.Offset = ctypes.c_uint64.from_address(context + 0xA0).value  # Rbp
        frame.AddrPC.Mode = frame.AddrStack.Mode = frame.AddrFrame.Mode = 3  # flat
        pcs = []
        for _ in range(120):
            if not dbghelp.StackWalk64(0x8664, process, thread, ctypes.byref(frame), ctypes.c_void_p(context),
                                       None, table_access, module_base, None):
                break
            if frame.AddrPC.Offset == 0:
                break
            pcs.append(frame.AddrPC.Offset)
        kernel32.QueryThreadCycleTime(thread, ctypes.byref(cycles))
        weights.append(cycles.value - last_cycles)
        last_cycles = cycles.value
        kernel32.ResumeThread(thread)
        samples.append(pcs)
        time.sleep(0.002)

    # Frames in the executable are named from the map; others by module.
    modules = (ctypes.c_void_p * 512)()
    needed = wt.DWORD()
    psapi.EnumProcessModules(process, modules, ctypes.sizeof(modules), ctypes.byref(needed))
    image = modules[0]
    names = {}
    for index in range(needed.value // 8):
        text = ctypes.create_unicode_buffer(260)
        psapi.GetModuleBaseNameW(process, ctypes.c_void_p(modules[index]), text, 260)
        names[modules[index]] = text.value
    starts = sorted(names)
    symbols = []
    pattern = re.compile(r"^\s*[0-9a-f]{4}:[0-9a-f]{8}\s+(\S+)\s+([0-9a-f]{16})\s", re.I)
    with open(args.map, encoding="latin-1") as file:
        for line in file:
            match = pattern.match(line)
            if match:
                symbols.append((int(match.group(2), 16), match.group(1)))
    symbols.sort()
    addresses = [address for address, _ in symbols]

    def describe(address):
        preferred = address - image + 0x140000000
        if symbols and 0x140000000 <= preferred <= addresses[-1] + 0x100000:
            return symbols[bisect.bisect_right(addresses, preferred) - 1][1][:200]
        index = bisect.bisect_right(starts, address) - 1
        return f"{names[starts[index]]}+{address - starts[index]:#x}" if index >= 0 else hex(address)

    stacks = [[describe(pc) for pc in pcs] for pcs in samples]
    return stacks, weights


if args.again:
    stacks = json.load(open(args.again + ".json"))
    weights = json.load(open(args.again + ".cycles.json"))
else:
    stacks, weights = sample()
json.dump(stacks, open(args.out + ".json", "w"))
json.dump(weights, open(args.out + ".cycles.json", "w"))

# Compiled module functions are items `m<index>` of crates such as
# `nativeboat_renderer_0_chunk_48`, length-prefixed in v0 mangled names.
sources = {}
if args.modules:
    for line in open(args.modules, encoding="utf-8", errors="replace"):
        parts = line.rstrip("\n").split(" ", 2)
        if len(parts) == 3:
            sources[(parts[0], parts[1])] = parts[2]
crate_pattern = re.compile(r"(\d+)nativeboat_")


def source(frame):
    for match in crate_pattern.finditer(frame):
        length = int(match.group(1))
        start = match.start(1) + len(match.group(1))
        crate = frame[start:start + length]
        group = re.match(r"nativeboat_(\w+?)(?:_chunk_\d+|_forwarder_\d+_\d+)?$", crate)
        rest = frame[start + length:]
        digits = re.match(r"\d+", rest)
        if not (group and digits):
            continue
        item = rest[digits.end():digits.end() + int(digits.group())]
        if re.fullmatch(r"m\d+", item):
            return sources.get((group.group(1), item), f"{group.group(1)} {item}")
    return None


inclusive = collections.Counter()
leaf = collections.Counter()
cpu_inclusive = collections.Counter()
cpu_leaf = collections.Counter()
cpu_source = collections.Counter()
chains = collections.Counter()
for frames, weight in zip(stacks, weights):
    if not frames:
        continue
    leaf[frames[0]] += 1
    cpu_leaf[frames[0]] += weight
    chains[tuple(frame[:110] for frame in frames[:6])] += 1
    for symbol in set(frames):
        inclusive[symbol] += 1
        cpu_inclusive[symbol] += weight
    # The innermost compiled JavaScript frame owns the sample's time.
    owner = next((name for name in map(source, frames) if name), None)
    if owner:
        cpu_source[owner] += weight
total = sum(weights) or 1
with open(args.out, "w", encoding="utf-8") as out:
    out.write(f"{len(stacks)} samples, {total / 1e6:.0f} Mcycles\n\ninclusive samples:\n")
    for symbol, count in inclusive.most_common(120):
        out.write(f"{count:6} {symbol}\n")
    out.write("\nleaf samples:\n")
    for symbol, count in leaf.most_common(60):
        out.write(f"{count:6} {symbol}\n")
    out.write("\ninclusive CPU %:\n")
    for symbol, weight in cpu_inclusive.most_common(100):
        out.write(f"{100 * weight / total:6.1f} {symbol}\n")
    out.write("\nleaf CPU %:\n")
    for symbol, weight in cpu_leaf.most_common(60):
        out.write(f"{100 * weight / total:6.1f} {symbol}\n")
    if cpu_source:
        out.write("\nCPU % by compiled source file (innermost compiled frame):\n")
        for name, weight in cpu_source.most_common(40):
            out.write(f"{100 * weight / total:6.1f} {name}\n")
    out.write("\nstack tops:\n")
    for chain, count in chains.most_common(25):
        out.write(f"{count:6}\n")
        for frame in chain:
            out.write(f"         {frame}\n")
print("wrote", args.out)
