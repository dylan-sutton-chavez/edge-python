# Security at Edge

Thanks for taking the time to report a vulnerability. Edge is a sandboxed runtime, so the sandbox boundary is the security boundary.

## Reporting

Email [dylan@edgepython.com](mailto:dylan@edgepython.com). Do not open a public GitHub issue for a security report.

Include a minimal reproducing script with the command used to run it, the expected and actual behavior, and the release version or commit hash.

## Scope

Everything in this repository is in scope. The compiler, the VM, the CLI, the ABI, native plugins, the standard library, and snapshot handling all count, whether the issue is a sandbox escape, memory corruption, or a budget bypass under `Limits::sandbox()`.

The only exception is the CDN, and the documentation website, which are served infrastructure rather than shipped code.

## Response

Reports are acknowledged as soon as possible. Fixes land on the main branch and ship in the next release.

Edge has not reached 1.0. Some areas are on the roadmap to be hardened further, and there are no monetary rewards at this time. Reporters are credited by name in the release notes unless they prefer to stay anonymous.

Only the latest release and the main branch receive security fixes.
