"""Writes THIRD-PARTY-LICENSES.txt for a nativeboat release.

It covers the third-party Rust crates compiled into nativeboat.exe, those in
the libraries the release's toolchain folder carries for building
applications, and the sources vendored in this repository, with the license
texts each crate ships. Where a crate offers a choice of licenses, MIT is
taken, then Apache-2.0. The Rust toolchain's own licenses travel in
toolchain/share/doc/rust.

    python scripts/third-party-licenses.py [OUTPUT]   # default: THIRD-PARTY-LICENSES.txt

It fails when a crate's license is not one this script knows to be
permissive, so a new dependency cannot slip into a release unreviewed.
"""

import json
import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
# The crates whose dependencies a release contains: the compiler and the
# libraries scripts/package-toolchain.ps1 builds for applications.
SHIPPED = ['nativeboat-cli', 'nativeboat-layout', 'nativeboat-win32']
VENDOR_PARENTS = [
    'crates/nativeboat-cli/vendor',
    'crates/nativeboat-encoding-build/vendor',
    'crates/nativeboat-regexp-build/vendor',
    'crates/nativeboat-url-build/vendor',
]
# Vendored sources without a manifest of their own, by crate name: their
# licenses as their manifests declare them.
VENDORED_LICENSES = {
    'cfg-if': 'MIT OR Apache-2.0',
    'chardetng': 'Apache-2.0 OR MIT',
    'encoding_rs': '(Apache-2.0 OR MIT) AND BSD-3-Clause',
    'memchr': 'Unlicense OR MIT',
    'multiversion_no_op': 'Apache-2.0 OR MIT',
    'scopeguard': 'MIT OR Apache-2.0',
}
PERMISSIVE = ['MIT', 'Apache-2.0', 'Apache-2.0 WITH LLVM-exception', 'BSD-2-Clause', 'BSD-3-Clause',
              'ISC', 'Zlib', '0BSD', 'Unicode-3.0', 'Unicode-DFS-2016', 'BSL-1.0', 'Unlicense', 'CC0-1.0']
PREFERENCE = ['MIT', 'Apache-2.0', 'Apache-2.0 WITH LLVM-exception', 'BSD-3-Clause', 'BSD-2-Clause', 'ISC',
              'Zlib', '0BSD', 'BSL-1.0', 'Unlicense', 'CC0-1.0']
# Words in license file names that identify the license they hold.
FILE_KEYS = {
    'MIT': ['mit'], 'Apache-2.0': ['apache'], 'Apache-2.0 WITH LLVM-exception': ['apache', 'llvm'],
    'BSD-3-Clause': ['bsd', 'whatwg'], 'BSD-2-Clause': ['bsd'], 'ISC': ['isc'], 'Zlib': ['zlib'], '0BSD': ['0bsd'],
    'Unicode-3.0': ['unicode'], 'Unicode-DFS-2016': ['unicode'], 'BSL-1.0': ['bsl', 'boost'],
    'Unlicense': ['unlicense'], 'CC0-1.0': ['cc0'],
}
MIT_TEXT = '''Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
'''


def metadata():
    output = subprocess.run(
        ['cargo', 'metadata', '--format-version', '1', '--locked',
         '--filter-platform', 'x86_64-pc-windows-msvc', '--manifest-path', os.path.join(ROOT, 'Cargo.toml')],
        check=True, capture_output=True, text=True, encoding='utf-8').stdout
    return json.loads(output)


def shipped_packages(meta):
    packages = {package['id']: package for package in meta['packages']}
    nodes = {node['id']: node for node in meta['resolve']['nodes']}
    stack = [package['id'] for package in meta['packages'] if package['name'] in SHIPPED and not package['source']]
    seen = set()
    while stack:
        current = stack.pop()
        if current in seen:
            continue
        seen.add(current)
        for dependency in nodes[current]['deps']:
            # Normal and build dependencies; tests' are not shipped.
            if any(kind['kind'] in (None, 'build') for kind in dependency['dep_kinds']):
                stack.append(dependency['pkg'])
    return [packages[i] for i in seen if packages[i]['source']]


def vendored_packages(meta):
    registry = {(package['name'], package['version']): package for package in meta['packages']}
    found = []
    for parent in VENDOR_PARENTS:
        directory = os.path.join(ROOT, parent)
        for name in sorted(os.listdir(directory)):
            path = os.path.join(directory, name)
            manifest = os.path.join(path, 'Cargo.toml')
            if not os.path.isdir(path):
                continue
            if not os.path.isfile(manifest):
                crate, _, version = name.rpartition('-')
                package = registry.get((crate, version))
                found.append({
                    'name': crate, 'version': version,
                    'license': VENDORED_LICENSES.get(crate) or (package or {}).get('license', ''),
                    # License files beside the sources, else the registry copy's.
                    'manifest_path': manifest if license_files(path) or not package else package['manifest_path'],
                    'authors': (package or {}).get('authors', []),
                })
                continue
            text = open(manifest, encoding='utf-8').read()
            package = text[text.index('[package]'):]
            field = lambda key: (re.search(r'^' + key + r'\s*=\s*"([^"]*)"', package, re.M) or [None, ''])[1]
            authors = re.search(r'^authors\s*=\s*\[(.*?)\]', package, re.M | re.S)
            found.append({
                'name': field('name'), 'version': field('version'), 'license': field('license'),
                'manifest_path': manifest,
                'authors': re.findall(r'"([^"]*)"', authors.group(1)) if authors else [],
            })
    return found


def alternatives(expression):
    """The license sets an SPDX expression allows: one per OR branch."""
    expression = expression.replace('/', ' OR ')
    depth, branches, current = 0, [], ''
    for token in re.split(r'(\(|\)|\s+OR\s+)', expression):
        if token == '(':
            depth += 1
        elif token == ')':
            depth -= 1
        elif re.fullmatch(r'\s+OR\s+', token or '') and depth == 0:
            branches.append(current)
            current = ''
            continue
        current += token or ''
    branches.append(current)
    result = []
    for branch in branches:
        branch = branch.strip()
        if branch.startswith('(') and branch.endswith(')') and ' AND ' not in branch:
            result.extend(alternatives(branch[1:-1]))
            continue
        parts = [part.strip().strip('()') for part in re.split(r'\s+AND\s+', branch)]
        expanded = [[]]
        for part in parts:
            options = alternatives(part) if ' OR ' in part else [[part]]
            expanded = [done + option for done in expanded for option in options]
        result.extend(expanded)
    return result


def choose(expression):
    allowed = [branch for branch in alternatives(expression) if all(part in PERMISSIVE for part in branch)]
    if not allowed:
        return None
    ranks = lambda branch: sorted((PREFERENCE.index(p) if p in PREFERENCE else len(PREFERENCE) for p in branch),
                                  reverse=True)
    return min(allowed, key=lambda branch: (ranks(branch), len(branch)))


def license_files(directory):
    return sorted(name for name in os.listdir(directory)
                  if re.match(r'(?i)(licen[cs]e|copying|copyright|notice|unlicense)', name)
                  and os.path.isfile(os.path.join(directory, name)))


def texts_for(package, chosen):
    directory = os.path.dirname(package['manifest_path'])
    files = license_files(directory)
    picked = []
    for name in files:
        lower = name.lower()
        if 'notice' in lower or lower.startswith('copyright'):
            picked.append(name)
            continue
        keys = [key for license in chosen for key in FILE_KEYS.get(license, [])]
        if any(key in lower for key in keys):
            picked.append(name)
    generic = [name for name in files if re.fullmatch(r'(?i)(licen[cs]e|copying|copyright)(\.(md|txt))?', name)]
    if not [name for name in picked if 'notice' not in name.lower()]:
        picked = generic + [name for name in picked if name not in generic]
    texts = [(name, open(os.path.join(directory, name), encoding='utf-8', errors='replace').read().strip())
             for name in dict.fromkeys(picked)]
    if not texts and chosen == ['MIT']:
        authors = ', '.join(re.sub(r'\s*<[^>]*>', '', author) for author in package.get('authors') or [])
        holder = authors or f'the {package["name"]} authors'
        texts = [('MIT License (the crate ships no license file)', f'Copyright (c) {holder}\n\n{MIT_TEXT}'.strip())]
    return texts


def main():
    output = sys.argv[1] if len(sys.argv) > 1 else os.path.join(ROOT, 'THIRD-PARTY-LICENSES.txt')
    meta = metadata()
    packages = {}
    for package in shipped_packages(meta) + vendored_packages(meta):
        packages.setdefault((package['name'], package['version']), package)
    problems = []
    sections = []
    for (name, version), package in sorted(packages.items()):
        expression = package.get('license') or ''
        chosen = choose(expression) if expression else None
        if chosen is None:
            problems.append(f'{name} {version}: license "{expression}" needs review')
            continue
        texts = texts_for(package, chosen)
        if not texts:
            problems.append(f'{name} {version}: no license text found for {" AND ".join(chosen)}')
            continue
        body = '\n\n'.join(f'--- {file} ---\n{text}' for file, text in texts)
        sections.append(f'{name} {version}\nLicense: {expression} (used under {" AND ".join(chosen)})\n\n{body}\n')
    if problems:
        sys.exit('Third-party licenses need attention:\n  ' + '\n  '.join(problems))
    header = (
        'Third-party software in NativeBoat\n\n'
        'NativeBoat contains the following third-party Rust crates. Each is listed with\n'
        'its license and the license texts it ships. The Rust compiler, its libraries\n'
        'and the rust-lld linker in the toolchain folder are under their own licenses,\n'
        'in toolchain/share/doc/rust.\n\n'
    )
    rule = '\n' + '=' * 78 + '\n\n'
    with open(output, 'w', encoding='utf-8', newline='\n') as file:
        file.write(header + rule.join(sections))
    print(f'Wrote {len(sections)} crates to {output}')


if __name__ == '__main__':
    main()
