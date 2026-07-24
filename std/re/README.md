# Edge Python Re

Regular expressions shipped as a `.wasm` plugin. Scripts see `re` as ordinary.

```python
from re import search, sub, findall

print(search(r'(\d+)-(\d+)', 'order 12-34 shipped')) # 12-34
print(sub(r'\s+', '_', 'a  b   c')) # a_b_c
print(findall(r'\w+', 'one two three')) # ['one', 'two', 'three']
```

## Design

A small backtracking engine, around 1k lines of Rust, no automaton or bytecode. It is Unicode aware without shipping Unicode tables: `\d`, `\w`, `\s`, and `(?i)` evaluate through the standard library char predicates at match time. The match runs over codepoints, so every offset is a codepoint index that lines up with how the runtime indexes strings.

The tradeoff is the backtracking tradeoff: no linear time guarantee. Instead of hanging, a step budget proportional to the input length watches for degradation. When a pattern blows the budget the call raises naming catastrophic backtracking, so the author can rewrite it.

## API

All functions take the pattern first. Flags are written inline, `(?i)` ignorecase, `(?s)` dotall, `(?m)` multiline.

| Function | Returns |
| --- | --- |
| `match(pattern, string)` | matched text anchored at the start, or `None` |
| `search(pattern, string)` | first matched text anywhere, or `None` |
| `fullmatch(pattern, string)` | matched text if it spans the whole string, or `None` |
| `findall(pattern, string)` | list of matches, or list of groups when the pattern has groups |
| `groups(pattern, string)` | capture groups of the first match as a list, or `None` |
| `span(pattern, string)` | `[start, end]` codepoint offsets of the first match, or `None` |
| `sub(pattern, repl, string)` | string with every match replaced, `\1` and `\g<name>` expand groups |
| `compile(pattern)` | pattern object exposing the functions above as methods, without the pattern argument |

```python
print(groups(r'(\w+)@(\w+)', 'a@b')) # ['a', 'b']
print(span(r'\d+', 'áé123')) # [2, 5], codepoint offsets

p = compile(r'\d+') # bad patterns raise here, at compile time
print(p.findall('a1 b22')) # ['1', '22']
```

Every pattern compiles once regardless of entry point: `compile` and the module-level functions share one compiled-pattern cache, so hot loops pay parsing a single time.

## Pattern syntax

Patterns are ordinary strings. Prefer raw strings (`r'...'`) so backslashes reach the engine intact.

**Characters:** `.` any char (also newline under `(?s)`); `\d` `\w` `\s` digit/word/whitespace (Unicode aware) and negations `\D` `\W` `\S`; `[abc]` `[a-z]` set or range; `[^abc]` negated set; `\n` `\t` `\xhh` `\uhhhh` escapes.

**Anchors:** `^` `$` start and end (per line under `(?m)`); `\b` `\B` word boundary and non-boundary.

**Quantifiers:** `*` `+` `?`; `{m}` `{m,n}` `{m,}`; trailing `?` for the lazy form (`*?`, `+?`).

**Groups and references:** `(...)` capturing; `(?:...)` non-capturing; `(?P<name>...)` named; `\1` `(?P=name)` backreference; `a|b` alternation.

**Lookaround and flags:** `(?=...)` `(?!...)` lookahead; `(?<=...)` `(?<!...)` lookbehind (fixed width only); `(?i)` `(?s)` `(?m)` inline flags.

## Not supported

Unicode property classes `\p{...}`, named character escapes `\N{...}`, atomic groups, possessive quantifiers, conditional patterns, scoped inline flags `(?i:...)`, variable width lookbehind. `\d` follows the Unicode numeric property, slightly wider than the CPython decimal set. Invalid patterns raise `ValueError`.

## When a pattern degrades

A step budget proportional to the input watches the work; when a pattern explodes the budget the engine raises instead of hanging:

```text
error: catastrophic backtracking: O(n^2) time or worse on this input, simplify nested quantifiers
  --> <input>:1:8
  |
1 | search(r'(a+)+$', user_input)
  |        ^^^^^^^^^
```

The nested `(a+)+` lets the engine split a run of `a` countless ways before failing at `$`; the flat form `a+$` matches the same text in linear time. This raises `RuntimeError`, not `ValueError`: a malformed pattern is a value problem, but a valid pattern that exhausts its budget is a runtime one (the same split engines like .NET draw with `RegexMatchTimeoutException`).

## How it works

Compiles to `wasm32-unknown-unknown` (`cdylib`) against the [wasm-pdk](https://github.com/dylan-sutton-chavez/edge-python/tree/main/wasm-pdk) `v0.1.0` ABI. `match` is a hand-written export (it is a Rust keyword); the rest go through `#[plugin_fn]`. Results cross the handle ABI as primitives and lists; plugin memory recycles per call through a static pool, like `json`.

## License

MIT OR Apache-2.0
