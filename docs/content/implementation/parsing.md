---
title: "Parsing"
description: "Single-pass parser, SSA emission, and bytecode shape."
---

## Overview

The parser is single-pass. It consumes the lexer token stream and emits bytecode straight into an `SSAChunk`, with no intermediate AST. Each construct is parsed and lowered in one traversal. Along the way it handles SSA versioning, phi-node insertion at control-flow joins, and structural diagnostics. Lex-time errors merge into the parser's diagnostic stream for a single ordered report.

Expression parsing uses Pratt precedence climbing. Each operator declares left and right binding power, and `expr_bp(min_bp)` pulls in everything bound at least as tightly as `min_bp`.

## Bytecode model

Each instruction is a tagged 4-byte record:

```rust
pub struct Instruction {
    pub opcode: OpCode, // 1 byte (#[repr(u8)])
    pub operand: u16,   // 2 bytes
}
```

The operand is a 16-bit slot whose meaning depends on the opcode. Common shapes:

| OpCode | Operand interpretation |
|---------------------------|------------------------------------------------------|
| `LoadConst` | constant pool index |
| `LoadName` / `StoreName` | name slot index |
| `Add`, `Sub`, ... | unused (the inline cache keys on ip) |
| `Call` | `(num_kw << 8) \| num_pos` |
| `BuildList` / `BuildTuple` / `BuildSet` | element count |
| `BuildDict` | key-value pair count |
| `BuildSlice` | parts count (2 or 3) |
| `Jump` / `JumpIfFalse` | target instruction index |
| `ForIter` | jump target on iterator exhaustion |
| `Phi` | target slot (sources stored in `chunk.phi_sources`) |
| `UnpackSequence` | element count |
| `UnpackEx` | `(before << 8) \| after` |
| `MakeFunction` | function index in `chunk.functions` |

Operands, the constant pool, the name table, and the instruction stream per chunk are all capped at `u16::MAX` (65,535).

## Expression parsing

`expr_bp(min_bp)` runs the Pratt loop. `parse_atom` advances one token and routes by kind:

```text
Name                     -> name() (handles assignment, walrus, calls)
String / FstringStart    -> string_group() (adjacent str and f-string literals concatenate)
Int / Float              -> emit numeric constant (ints widen i64 -> i128, beyond ±2^127 is a parse error)
True/False/None/Ellipsis -> emit dedicated load opcode
Lbrace                   -> brace_literal() (dict, set, comprehension)
Lsqb                     -> list_literal() (list, comprehension)
Lpar                     -> grouped expr, tuple, generator, or empty tuple
Lambda                   -> parse_lambda()
```

After an atom, `postfix_tail()` handles trailers (subscript, attribute, call), iterating until none apply, plus store tails on the last trailer (`xs[0].v = 7`, `xs[0][1] += 1`). So `fns[0](-3)`, `obj.method()`, `(lambda x: x)(3)`, and `compose(f, g)(x)` all parse uniformly.

`*args` and `**kwargs` are accepted in call position. Starred unpacking also works in list (`[*a, *b]`), set (`{*s}`), and dict (`{**d1, **d2}`) literals, lowering via `ListExtend` / `SetUpdate` / `DictUpdate`. Tuple-literal unpacking (`(1, *xs, 2)`) is not supported.

## Operator precedence

Each binary operator declares `(l_bp, r_bp, OpCode)` in `binding_power`. Higher binding pulls tighter. Only `**` is right-associative (`r_bp < l_bp`). Everything else is left-associative.

| Level | Operators | Notes |
|-------|------------------------------------------|----------------------|
| 1/2 | `or` | short-circuit |
| 3/4 | `and` | short-circuit |
| 5 | unary `not` | prefix only |
| 7/8 | `==` `!=` `<` `>` `<=` `>=` `in` `not in` `is` `is not` | chainable |
| 9/10 | `\|` | bitwise |
| 11/12 | `^` | bitwise |
| 13/14 | `&` | bitwise |
| 15/16 | `<<` `>>` | shifts |
| 17/18 | `+` `-` | additive |
| 19/20 | `*` `/` `%` `//` | multiplicative |
| 21 | unary `-` `~` `await` | prefix |
| 22/21 | `**` | right-associative |

`infix_bp` handles comparison chaining (`a < b < c`). When a comparison opcode is followed by another comparison token, the parser stores the middle value in a synthetic `#cmp_N` slot, emits the first comparison, short-circuits on false, and reuses the stored value for the next comparison.

Associativity and chaining are observable in the results:

```python
print(2 ** 3 ** 2)
print(1 < 2 < 3)
print(1 < 2 > 3)
```

```text Output
512
True
False
```

## Short-circuit lowering

`and` and `or` lower to `JumpIfFalseOrPop` / `JumpIfTrueOrPop`. These peek the stack top and pop only when execution continues. Otherwise they jump and leave the value on the stack:

```text
a and b

LoadName a
JumpIfFalseOrPop -> end
LoadName b
end:
```

This preserves operand identity (the actual value is returned, not a coerced bool) without an extra opcode.

The returned value is the operand itself, whichever side decided the result:

```python
print(0 or "fallback")
print(1 and 2)
print("" or 0)
```

```text Output
fallback
2
0
```

## Conditional expression lowering

`a if cond else b` parses value-first, since the value is textually first and there is no AST to reorder. On reaching `if`, the parser drains the value's instructions and re-emits them after the condition, shifting internal jump targets. The condition runs first and only one branch executes:

```text
a if cond else b

LoadName cond
JumpIfFalse -> else
LoadName a
Jump -> end
else:
LoadName b
end:
```

## SSA versioning

Each binding emits a fresh slot with an incremented version. The parser keeps a `HashMap<String, u32>` of name to current version. Names in `chunk.names` are stored as `name_version`:

```python
x = 1  # x_1
x = 2  # x_2
y = x  # y_1, references x_2
```

```text
chunk.names = ["x_1", "x_2", "y_1"]
chunk.instructions:
  LoadConst 0   (1)
  StoreName 0   (x_1)
  LoadConst 1   (2)
  StoreName 1   (x_2)
  LoadName 1    (x_2)
  StoreName 2   (y_1)
```

Undefined names target version 0 (`x_0`), which the host fills before execution (the VM seeds globals like `print_0`). A name still unbound at load time raises `NameError`.

## Phi nodes at joins

At each control-flow boundary the parser pushes a `JoinNode { backup, then }` onto a stack:

```text
enter_block()  -> snapshot current versions into JoinNode.backup
mid_block()    -> snapshot post-then versions into JoinNode.then, restore baseline for else
commit_block() -> diff (then u post) against backup, emit Phi for each name that diverged
```

Each `Phi` carries the target slot (the new version after the join) in its operand. Source slots live in `chunk.phi_sources`, indexed by `chunk.phi_map[ip]` at runtime. This keeps `Instruction` at 4 bytes.

```python
cond = True
if cond:
    x = 1
else:
    x = 2
print(x)
```

```text Output
1
```

```text
LoadName cond_0
JumpIfFalse else_label
LoadConst 0 (1)
StoreName x_1
Jump end_label
else_label:
LoadConst 1 (2)
StoreName x_2
end_label:
Phi x_3 (sources: x_1, x_2)
LoadName x_3
CallPrint 1
```

At runtime `Phi` copies the first defined source slot into the target. Exactly one branch executed, so exactly one source is defined.

## Statement dispatch

`stmt()` peeks the leading token and routes:

```text
if       -> if_stmt (elif chain, optional else)
for      -> for_stmt_inner (sync iter, optional else)
while    -> while_stmt (break/continue patches)
match    -> match_stmt
def      -> func_def_inner
class    -> class_def (__init__, attributes, methods)
with     -> with_stmt_inner (multi-target, async variant)
try      -> try_stmt (except, else, finally, raise)
import   -> import_stmt (compile-time resolver lookup)
from     -> parse_from_stmt (named and star imports, same path)
yield    -> yield expr / yield from
async    -> async def / for / with
@        -> decorator stack + def or class
return   -> expr + ReturnValue
raise    -> expr + Raise / RaiseFrom
break    -> emits Jump, back-patched to the loop exit
continue -> jump to current loop start
del / global / nonlocal / pass -> direct emit
assert   -> Assert opcode (the ", msg" form lowers to a conditional raise of AssertionError(msg))
Name     -> name_stmt (assignment, augmented, indexed, attribute, call)
```

Each statement returns a bool telling whether it left a value on the stack. The driver emits `PopTop` after expression-shaped statements (`x.method()`, `1 + 2` at module level) but not after statement-shaped ones (assignment, control flow).

Decorators apply to `def` and `class`. The `@` arm peeks for `class` after the decorator list. Each decorator wraps via `Call,1` between `MakeFunction`/`MakeClass` and the final `StoreName`.

`raise X from Y` lowers to `RaiseFrom`, which pops the cause then the exception so `X` surfaces. A bare `raise` re-raises. `__cause__` and `__context__` are not exposed. `except` matching walks the exception parent table, so `except Exception` catches subclasses like `isinstance(e, Exception)` does.

Imports resolve at parse time through a host-injected resolver. No import opcodes reach the VM. See [Modules](/reference/modules).

## Lambda and function bodies

Lambdas and `def` both compile their body into a fresh `SSAChunk`:

```rust
self.with_fresh_chunk(|s| {
    s.ssa_versions = outer_versions.clone();
    for p in &params { s.ssa_versions.insert(param_base_name(p).to_string(), 0); }
    s.expr(); // or compile_block_body for def
    s.chunk.emit(OpCode::ReturnValue, 0);
});
```

Free variables (non-parameters with no local binding) are looked up in the outer chunk. `MakeFunction` captures matching slots from the enclosing scope into `captures` as shared cells, 1-element heap lists registered per call frame, so sibling closures over the same variable observe each other's `nonlocal` writes. Capture propagates through any depth (`A -> B -> C` where `C` references a variable in `A`).

Parameter slots are `Normal`, `Star` (`*args`), `DoubleStar` (`**kwargs`), and `KwOnly`. A lone `*` separator marks following params as keyword-only. Defaults live in `HeapObj::Func.defaults` and bind to the `=`-marked params in source order, so a default before `*args` and keyword-only defaults both apply correctly. Annotations (`x: T`, `-> T`) parse and drain to `chunk.annotations` for tooling only.

`compile_body` sets `body.is_pure`, the flag that gates template memoisation. A body is pure when it contains no impurity opcode (`StoreItem`, `DelItem`, `StoreAttr`, `DelAttr`, `CallPrint`, `CallInput`, `Global`, `Nonlocal`, `Raise`, `RaiseFrom`, `Yield`, `LoadAttr`) and reads no free (global or builtin) name. The runtime half of the check is described in [Design](/implementation/design#key-mechanisms).

## Type annotations

Annotations are parsed for source compatibility and discarded at runtime:

```python
counter: int = 0  # annotation parsed and stored, slot still gets 0

def f(x: int) -> int:
    return x
```

They are recorded in `chunk.annotations` for tooling. No code is emitted. `f.__annotations__` is not exposed at runtime, but `f.__name__` is.

## Comprehensions and generators

List, set, and dict comprehensions support multi-`for` and multi-`if`. They lower to `BuildList` / `BuildSet` / `BuildDict` plus a loop scaffold using `ListAppend` / `SetAdd` / `MapAdd`.

Generator expressions lower eagerly to `BuildList`, so `(i*2 for i in xs)` is operationally `[i*2 for i in xs]`. This is deliberate. Template memoisation needs hashable, finite arguments, and lazy generators would not memoise. For unbounded streams, write a `def` with `yield`, which produces a real `HeapObj::Coroutine`.

```python
g = (i * 2 for i in range(3))
print(g)
```

```text Output
[0, 2, 4]
```

Async comprehensions (`[x async for x in y]`) and starred unpacking inside comprehensions are unsupported.

## F-string lowering

An f-string lowers to constant chunks interleaved with `FormatValue` ops, finished by `BuildString`:

```python
name = "bo"
age = 3
print(f"hello {name}, age {age}")
```

```text Output
hello bo, age 3
```

```text
LoadConst "hello "
LoadName name_v
FormatValue 0
LoadConst ", age "
LoadName age_v
FormatValue 0
BuildString 4
CallPrint 1
```

The `FormatValue` operand is a small flags field:

- bit 0: set when a format spec string is on the stack just below the value
- bits 1-2: conversion (`0` none, `1` `!r`, `2` `!s`, `3` `!a`)

The VM applies the conversion first, then the spec mini-language `[[fill]align][sign][#][0][width][,|_][.precision][type]` with type chars `s d b o x X f F e E g G n % c`. `n` aliases `d` with no locale. The `=` self-documenting form (`{expr=}`) emits a literal `expr=` prefix. Adjacent string literals concatenate at parse time. Spec parse failures raise `ValueError` at runtime.

## Limits

| Constant | Value | Purpose |
|----------------------|-----------|----------------------------------------|
| `MAX_EXPR_DEPTH` | 200 | cap on recursive expression parsing |
| `MAX_INSTRUCTIONS` | 65,535 | cap on instructions per chunk |

Hitting `MAX_EXPR_DEPTH` produces the diagnostic `expression too deeply nested`. Hitting `MAX_INSTRUCTIONS` sets `chunk.overflow`, reported at end of parsing. The instruction stream is cleared rather than dispatched.

## References

1. Pratt. *Top Down Operator Precedence* (POPL 1973). Precedence climbing.
2. Cytron et al. *Efficiently Computing Static Single Assignment Form* (TOPLAS 1991). SSA and phi-nodes.
3. Nystrom. *Crafting Interpreters* ([craftinginterpreters.com](https://craftinginterpreters.com/)). Single-pass codegen patterns.
4. Casey et al. *Towards Superinstructions for Java Interpreters* (SCOPES 2003). LoadAttr+Call fusion.
