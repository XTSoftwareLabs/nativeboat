# Security policy

## Supported versions

NativeBoat is in alpha. Security fixes go into the next release; only the
latest release is supported.

## Reporting a vulnerability

Please do not report security problems in public issues, pull requests or
discussions.

Report them privately through GitHub: open the repository's **Security** tab
and choose **Report a vulnerability**
([direct link](https://github.com/XTSoftwareLabs/nativeboat/security/advisories/new)).

Include what you can of:

- the NativeBoat version (`nativeboat --version`) and Windows version;
- whether the problem is in `nativeboat` itself, in the toolchain it ships,
  or in applications it builds;
- the steps or a small project that reproduces it, and what an attacker
  gains.

We aim to acknowledge a report within a week, keep you informed while we
work on a fix, and credit you in the advisory unless you prefer otherwise.

## Scope

In scope: the `nativeboat` command, its release packages, and the runtime
code that NativeBoat compiles into applications, such as its Node.js,
Electron and web platform implementations.

Out of scope: vulnerabilities in an application's own code or its npm
dependencies that behave the same under Electron. Report those to the
application or package. Problems in third-party components NativeBoat
includes are welcome here too, so we can update them.
