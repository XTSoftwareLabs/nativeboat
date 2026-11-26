"""Fails when a NativeBoat package carries source code.

    python scripts/check-no-sources.py <file or folder>...

Released compilers link the runtime libraries their toolchain ships
precompiled instead of compiling embedded sources. This
checks that no source files are in the folders, and that nativeboat.exe and
the precompiled libraries hold no readable Rust or JavaScript source text,
such as files embedded with `include_str!`.
"""
import os
import sys

# File types that are source code.
SOURCE_EXTENSIONS = {'.rs', '.js', '.mjs', '.cjs', '.ts', '.tsx', '.jsx'}

# Text that appears in source files but not in compiled code. Each marker is
# a construct with its surrounding layout, so names in compiled metadata do
# not match.
MARKERS = [
    b'\nuse crate::',
    b'\nuse super::',
    b'#![no_std]',
    b'\npub(crate) fn ',
    b'\n    pub fn ',
    b'\nimpl<',
    b"\n'use strict';",
    b'\nmodule.exports = ',
    b'\nconst primordials',
    b'\nfunction ',
]

# How many marker hits a file may have: the compiler program itself writes
# a few of these constructs into the crates and modules it generates (9 in
# 0.1.0-alpha.5), while embedded runtime sources have hundreds.
ALLOWED = 16


def scanned(path):
    name = os.path.basename(path).lower()
    return name == 'nativeboat.exe' or name.endswith('.rlib')


def files(path):
    if os.path.isfile(path):
        yield path
        return
    for directory, _, names in os.walk(path):
        for name in names:
            yield os.path.join(directory, name)


def problem(path):
    if os.path.splitext(path)[1].lower() in SOURCE_EXTENSIONS:
        return f'{path}: source file'
    if not scanned(path):
        return None
    with open(path, 'rb') as handle:
        data = handle.read()
    hits = {marker: data.count(marker) for marker in MARKERS}
    if sum(hits.values()) <= ALLOWED:
        return None
    found = ', '.join(
        f'{marker.decode().strip()!r} x{count}' for marker, count in hits.items() if count
    )
    return f'{path}: source text ({found})'


def main(paths):
    problems = []
    for path in paths:
        if not os.path.exists(path):
            problems.append(f'{path}: missing')
            continue
        problems.extend(filter(None, map(problem, files(path))))
    for line in problems:
        print(line)
    if problems:
        print(f'{len(problems)} file(s) carry source code')
        return 1
    print('No source code found')
    return 0


if __name__ == '__main__':
    if len(sys.argv) < 2:
        print(__doc__.strip())
        sys.exit(2)
    sys.exit(main(sys.argv[1:]))
