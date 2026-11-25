# Diagnostics for compiled applications

Applications that NativeBoat builds contain no JavaScript engine or browser,
so the usual developer tools are not there. Instead, the runtime reports what
it does on standard error (or into files) when these environment variables
are set. They cost nothing when unset, work for any application, and are
read when the program starts.

## Runtime traces

| Variable | Reports |
| --- | --- |
| `NATIVEBOAT_TRACE_EXIT` | Each reason the program may end: an explicit exit or quit, an uncaught exception, the host loop's end. |
| `NATIVEBOAT_TRACE_THROWS` | Every exception thrown, caught or not (an error's `name` and `message`), and the callees of calls made near the call depth limit, which show what recursed when a `RangeError: Maximum call stack size exceeded` follows. |
| `NATIVEBOAT_TRACE_MODULE_EVALUATION` | Each module as it is evaluated, for startup order and failures. |
| `NATIVEBOAT_TRACE_IPC` | Listeners pages add, and messages sent between processes and windows. |
| `NATIVEBOAT_PROFILE_CALLS` | Calls compiled code makes, by call site (the callee as the source writes it): count, time by themselves (without the profiled calls they make) and time including what they call. Every five seconds the sites that took longest by themselves so far are listed. |

## Pages

| Variable | Reports |
| --- | --- |
| `NATIVEBOAT_TRACE_INPUT` | Pointer, key and text input a page receives, its targets, and whether the page cancelled it; range rectangles scripts read, with the boxes around them. |
| `NATIVEBOAT_TRACE_LAYOUT` | Time spent on style (with elements styled and matches and styles reused), layout (boxes built and reused) and scene building. |
| `NATIVEBOAT_TRACE_PAINT` | Each painted area of each surface, the time rendering it and copying it to the window. |
| `NATIVEBOAT_DUMP_PAGE=<file>` | The page's description, rewritten at most once a second when it changes: every element's box and main computed styles, then every surface (position, size, scrolled content size, scrollbars) and its display list. |
| `NATIVEBOAT_DUMP_DOM=<file>` | The page's markup, for comparing a document with Chromium's. |

## Builds

| Variable | Effect |
| --- | --- |
| `NATIVEBOAT_JOBS=<n>` | How many generated crates compile at once; `1` keeps memory low. |
| `NATIVEBOAT_KEEP_GOING` | Compiles every generated crate and reports each one that fails, instead of stopping at the first failure. |
| `NATIVEBOAT_LINK_MAP=<file>` | Writes the linker map, so native stack samples can be symbolized. |
| `NATIVEBOAT_MODULE_MAP=<file>` | Appends which source file each compiled module function (`m<index>`) comes from, so profiles name files. Delete the file before a build. |

## Reading native profiles

Compiled functions are Rust symbols such as
`nativeboat_renderer_0_chunk_71::m4899::{closure}`: the module index after
`m` maps to a source file through `NATIVEBOAT_MODULE_MAP`. A stack sampler
over the running process (with the link map for symbols) shows where native
time goes; a deep recursion shows as a repeating cycle of such frames.

[`scripts/sample-stacks.py`](https://github.com/XTSoftwareLabs/nativeboat/blob/main/scripts/sample-stacks.py)
in the NativeBoat repository is such a sampler (Python, no other tools).
Build with `NATIVEBOAT_LINK_MAP=MyApp.map` and
`NATIVEBOAT_MODULE_MAP=modules.txt`, start the application, then:

```
python sample-stacks.py --pid <pid> --map MyApp.map --out report.txt --seconds 5 --modules modules.txt
```

It weights each sample by the CPU cycles the thread used since the one
before, so a call that blocks (an idle message loop, GDI waiting for the
compositor) counts by the processor time it took rather than by how long it
lasted. The report lists frames inclusive and as leaves, by samples and by
CPU share, and CPU share by source file (the innermost compiled frame's).
`report.txt.json` keeps every stack (leaf first) and `report.txt.cycles.json`
their weights; `--again report.txt` summarizes them again, say with another
module map. `--ready <file>` is written when sampling starts, so a script can
begin its input then.
