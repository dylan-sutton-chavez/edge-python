---
title: "Design"
description: "Explanation of the compiler architectural design."
---

## Overview

Edge Python is a Rust based bytecode interpreter weighing less than 70 KB, implementing adaptive instruction specialization to achieve performance approaching compiled native code.

## Concepts

* Offset-based token representation: Tokens store (start, end, kind) as indices into source buffer, avoiding string copies and maintaining O(n) lexing.
* Monolithic bytecode emission: Opcodes generated directly from tokens in single pass, collapsing parser and codegen phases without AST materialization.
* Inline opcode specialization: Generic bytecode replaced at runtime with type-specialized variants once operand types stabilize, enabling branch-free dispatch.
* Template-driven code instantiation: Precompiled native code patches applied when hotspots detected, substituting runtime values without code generation overhead.
* Adaptive bytecode metamorphosis: Instruction stream continuously rewritten based on execution profiles, allowing bytecode to evolve across runs.

## Compilation Pipeline

```bash
source -> offset-indexed tokens -> monolithic bytecode -> [inline caching + type inference] -> template instantiation -> native execution
```

$$
\Gamma_{compiler} = \mathcal{T}_{patch} \circ \mathcal{S}_{specialize} \circ \mathcal{E}_{emit} \circ \mathcal{P}_{parse}
$$

## Architecture

```bash
├── Cargo.toml
├── README.md
├── src
│   ├── lib.rs
│   ├── main.rs
│   └── modules
│       ├── compiler.rs
│       ├── lexer.rs
│       ├── opcodes.rs
│       ├── parser.rs
│       └── vm.rs
└── tests
```

## Capabilities

| types         | keywords         | builtin         | lexical         |
|---------------|------------------|-----------------|-----------------|
| int           | control flow     | i/o             | identation      |
| float         | functions        | type conversion | fstring         |
| str           | classes          | inspection      | walrus op       |
| bool          | operators        | iteration       | comments        |
| list          | variables        | aggregation     | docstrings      |
| dict          | literals         | math            | complex numbers |
| tuple         | alias            | debugging       | underscore      |
| set           | try/exception    | reflection      |                 |
| none          | context          | advances        |                 |
|               | async/await      |                 |                 |
|               | module           |                 |                 |
|               | pattern matching |                 |                 |
|               | type aliases     |                 |                 |
|               | import           |                 |                 |

## References

* Structure and performance of efficient bytecode interpreters: dl.acm.org/doi/10.1145/1328195.1328197
* Adaptive instruction specialization in interpreters: 2211.07633
* Copy-and-patch JIT compilation: dl.acm.org/doi/10.1145/3485513