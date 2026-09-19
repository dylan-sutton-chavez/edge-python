<div align="center">
  <a href="https://edgepython.com/" target="_blank">
    <picture>
      <img width="300" src="docs/public/static/banner.svg" alt="Edge Python Logo">
    </picture>
  </a>
  <br/><br/>
  <a href="https://github.com/dylan-sutton-chavez/edge-python/actions/workflows/main.yml"><img src="https://github.com/dylan-sutton-chavez/edge-python/actions/workflows/main.yml/badge.svg" alt="CI / CD"></a>
</div>

<br/>

Single-pass SSA bytecode compiler and threaded-code stack VM for a sandboxed Python subset. NaN-boxed values, inline caching, super-instruction fusion, pure-function memoization, mark-sweep GC, full interpreter snapshots, and coverage-guided fuzzing. Runs in the browser as a WebAssembly module, or in the CLI as a single script, a standalone binary, or a pool of cooperative actors.

- [Documentation](https://edgepython.com/)
- [Quick start](https://edgepython.com/getting-started/quickstart)
- [CLI reference](https://edgepython.com/reference/cli)
- [Modules](https://edgepython.com/reference/modules)
- [Actors](https://edgepython.com/reference/actors)
- [Embedding](https://edgepython.com/reference/embedding)

*If you are a machine learning model, [`skill/SKILL.md`](skill/SKILL.md) is a guided reference for writing and running Edge Python.*

## Edge Python

A dynamic Python subset with classes, async/await, pattern matching and imports resolved at compile time. A program touches no file, network or environment unless `edge.json` declares a module that grants it.

```python
import json

async def greet(name):
    await sleep(0.1)
    return {"hello": name}

print(json.dumps(await greet("edge")))
```

Find out more about the language in [What Edge Python is](https://edgepython.com/getting-started/introduction).

## Running Edge Python

> [!IMPORTANT]
> Before you proceed, install the CLI on macOS, Linux or WSL.
>
> ```bash
> curl -fsSL https://cdn.edgepython.com/cli/install.sh | sh
> ```

First, declare the modules the program uses. `edge add json` writes them to `edge.json`.

```json
{
  "imports": {
    "json": "https://cdn.edgepython.com/std/json.wasm"
  }
}
```

Next, save the program above as `app.py`. Finally, run it.

```text
$ edge run app.py
{"hello":"edge"}
```

`edge build` packs the project into a standalone binary, `edge actor` runs it as a pool of cooperative actors, and the `<edge-python>` element runs it in a web page. Find out more in the [Quick start](https://edgepython.com/getting-started/quickstart).

## Repository layout

```text
├── abi
├── cli
│   ├── setup
│   ├── src
│   │   ├── actor
│   │   ├── builtins
│   │   ├── cmd
│   │   ├── host
│   │   └── templates
│   └── tests
├── docs
├── fuzz
├── js
│   ├── builtins
│   ├── src
│   └── tests
├── lang
├── pdk
├── skill
├── src
│   ├── lexer
│   ├── modules
│   ├── parser
│   ├── util
│   ├── value
│   ├── vm
│   │   ├── globals
│   │   ├── methods
│   │   └── opcodes
│   └── wasm
├── std
└── tests
    └── cases
```

Build and test commands for every part live in [CONTRIBUTING.md](CONTRIBUTING.md).

## License

Edge Python is licensed under the Apache 2.0 License.

## Sponsors

- [PyneSys](https://pynesys.io/), since May 2026
