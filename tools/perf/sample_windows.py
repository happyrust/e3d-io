"""Sample a child x64 process with native DbgHelp; no third-party packages.

Build with CARGO_PROFILE_RELEASE_DEBUG=true. Only threads belonging to the
child are suspended, and every suspension is paired with ResumeThread.
Usage: python sample_windows.py OUTPUT.json EXECUTABLE [arguments...]
"""
import collections
import ctypes as c
from ctypes import wintypes as w
import json
import pathlib
import subprocess
import sys
import time


class ThreadEntry(c.Structure):
    _fields_ = [("size", w.DWORD), ("usage", w.DWORD), ("tid", w.DWORD), ("pid", w.DWORD), ("priority", w.LONG), ("delta", w.LONG), ("flags", w.DWORD)]


class Address(c.Structure):
    _fields_ = [("offset", c.c_ulonglong), ("segment", w.WORD), ("mode", w.DWORD)]


class KdHelp(c.Structure):
    _fields_ = [("thread", c.c_ulonglong), ("thunk", w.DWORD), ("callback", w.DWORD), ("next_callback", w.DWORD), ("frame", w.DWORD), ("ki_call", c.c_ulonglong), ("ke_call", c.c_ulonglong), ("base", c.c_ulonglong), ("limit", c.c_ulonglong), ("build", c.c_ulonglong), ("retpoline_size", c.c_ulonglong), ("retpoline_offset", c.c_ulonglong), ("retpoline_stub_size", c.c_ulonglong), ("retpoline_stub_offset", c.c_ulonglong), ("reserved", c.c_ulonglong * 2)]


class StackFrame(c.Structure):
    _fields_ = [("pc", Address), ("ret", Address), ("frame", Address), ("stack", Address), ("backing", Address), ("function", c.c_void_p), ("params", c.c_ulonglong * 4), ("far", w.BOOL), ("virtual", w.BOOL), ("reserved", c.c_ulonglong * 3), ("kd", KdHelp)]


class Symbol(c.Structure):
    _fields_ = [("size", w.ULONG), ("type_index", w.ULONG), ("reserved", c.c_ulonglong * 2), ("index", w.ULONG), ("symbol_size", w.ULONG), ("module", c.c_ulonglong), ("flags", w.ULONG), ("value", c.c_ulonglong), ("address", c.c_ulonglong), ("register", w.ULONG), ("scope", w.ULONG), ("tag", w.ULONG), ("name_len", w.ULONG), ("max_name", w.ULONG), ("name", c.c_char * 1)]


def main():
    if sys.platform != "win32" or c.sizeof(c.c_void_p) != 8:
        raise SystemExit("Windows x64 Python required")
    output = pathlib.Path(sys.argv[1])
    command = sys.argv[2:]
    kernel = c.WinDLL("kernel32", use_last_error=True)
    dbg = c.WinDLL("dbghelp", use_last_error=True)
    def api(dll, name, args, result):
        fn = getattr(dll, name)
        fn.argtypes, fn.restype = args, result
        return fn
    close = api(kernel, "CloseHandle", [w.HANDLE], w.BOOL)
    snapshot = api(kernel, "CreateToolhelp32Snapshot", [w.DWORD, w.DWORD], w.HANDLE)
    first = api(kernel, "Thread32First", [w.HANDLE, c.POINTER(ThreadEntry)], w.BOOL)
    next_thread = api(kernel, "Thread32Next", [w.HANDLE, c.POINTER(ThreadEntry)], w.BOOL)
    open_thread = api(kernel, "OpenThread", [w.DWORD, w.BOOL, w.DWORD], w.HANDLE)
    suspend = api(kernel, "SuspendThread", [w.HANDLE], w.DWORD)
    resume = api(kernel, "ResumeThread", [w.HANDLE], w.DWORD)
    context = api(kernel, "GetThreadContext", [w.HANDLE, c.c_void_p], w.BOOL)
    open_process = api(kernel, "OpenProcess", [w.DWORD, w.BOOL, w.DWORD], w.HANDLE)
    initialize = api(dbg, "SymInitialize", [w.HANDLE, c.c_char_p, w.BOOL], w.BOOL)
    cleanup = api(dbg, "SymCleanup", [w.HANDLE], w.BOOL)
    from_addr = api(dbg, "SymFromAddr", [w.HANDLE, c.c_ulonglong, c.POINTER(c.c_ulonglong), c.POINTER(Symbol)], w.BOOL)
    walk = api(dbg, "StackWalk64", [w.DWORD, w.HANDLE, w.HANDLE, c.POINTER(StackFrame), c.c_void_p, c.c_void_p, c.c_void_p, c.c_void_p, c.c_void_p], w.BOOL)
    api(dbg, "SymSetOptions", [w.DWORD], w.DWORD)(0x2 | 0x4 | 0x10)
    counts = collections.Counter()
    failures = collections.Counter()
    started = time.monotonic()
    with output.with_suffix(".stdout.json").open("w", encoding="utf-8") as stdout, output.with_suffix(".stderr.log").open("w", encoding="utf-8") as stderr:
        child = subprocess.Popen(command, stdout=stdout, stderr=stderr)
        process = open_process(0x410, False, child.pid)
        if not process:
            child.terminate()
            raise c.WinError(c.get_last_error())
        handles = {}
        symbols = {}
        try:
            time.sleep(0.1)
            if not initialize(process, str(pathlib.Path(command[0]).resolve().parent).encode(), True):
                raise c.WinError(c.get_last_error())
            def symbol(address):
                if address not in symbols:
                    data = c.create_string_buffer(c.sizeof(Symbol) + 4096)
                    info = c.cast(data, c.POINTER(Symbol))
                    info.contents.size, info.contents.max_name = c.sizeof(Symbol), 4096
                    displacement = c.c_ulonglong()
                    if from_addr(process, address, c.byref(displacement), info):
                        symbols[address] = c.string_at(c.addressof(data) + Symbol.name.offset, info.contents.name_len).decode("utf-8", "replace")
                    else:
                        symbols[address] = hex(address)
                return symbols[address]
            while child.poll() is None:
                snap = snapshot(4, 0)
                entry = ThreadEntry(size=c.sizeof(ThreadEntry))
                try:
                    valid = first(snap, c.byref(entry))
                    while valid:
                        if entry.pid == child.pid and entry.tid not in handles:
                            handle = open_thread(0x0002 | 0x0008 | 0x0040, False, entry.tid)
                            if handle:
                                handles[entry.tid] = handle
                        valid = next_thread(snap, c.byref(entry))
                finally:
                    close(snap)
                for tid, handle in handles.items():
                    if suspend(handle) == 0xffffffff:
                        failures["suspend"] += 1
                        continue
                    addresses = []
                    try:
                        storage = c.create_string_buffer(1232 + 16)
                        aligned = (c.addressof(storage) + 15) & ~15
                        c.c_uint32.from_address(aligned + 48).value = 0x100003
                        if context(handle, aligned):
                            frame = StackFrame()
                            frame.pc.offset = c.c_uint64.from_address(aligned + 248).value
                            frame.stack.offset = c.c_uint64.from_address(aligned + 152).value
                            frame.frame.offset = c.c_uint64.from_address(aligned + 160).value
                            frame.pc.mode = frame.stack.mode = frame.frame.mode = 3
                            for _ in range(16):
                                if not frame.pc.offset:
                                    break
                                addresses.append(frame.pc.offset)
                                if not walk(0x8664, process, handle, c.byref(frame), aligned, None, c.cast(dbg.SymFunctionTableAccess64, c.c_void_p), c.cast(dbg.SymGetModuleBase64, c.c_void_p), None):
                                    break
                        else:
                            failures["context"] += 1
                    finally:
                        resume(handle)
                    # Resolve symbols only after resuming the child.
                    if addresses:
                        counts[";".join(symbol(a) for a in addresses)] += 1
                time.sleep(0.01)
            code = child.wait()
        finally:
            for handle in handles.values():
                close(handle)
            cleanup(process)
            close(process)
            if child.poll() is None:
                child.terminate()
                child.wait()
    output.write_text(json.dumps({"tool":"native SuspendThread/GetThreadContext/StackWalk64/SymFromAddr", "interval_seconds":0.01,"command":command,"elapsed_seconds":time.monotonic()-started,"exit_code":code,"failures":dict(failures),"stacks":[{"count":v,"stack":k} for k,v in counts.most_common()]}, indent=2), encoding="utf-8")
    if code or not counts:
        raise SystemExit(code or "no samples captured")


if __name__ == "__main__":
    main()
