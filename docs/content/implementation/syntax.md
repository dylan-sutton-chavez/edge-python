---
title: "Syntax"
description: "Single-pass parser, SSA emission, and bytecode shape."
---

## Overview

Single-pass parser. It consumes the lexer token stream and emits bytecode straight into an `SSAChunk`, with no intermediate AST. Each construct is parsed and lowered in one traversal.

It also handles:

- SSA versioning
- phi-node insertion at control-flow joins
- structural diagnostics

Lex-time errors merge into the parser's diagnostic stream for a single ordered report.

Expression parsing uses Pratt. Each operator declares left/right binding power. `expr_bp(min_bp)` pulls in everything bound at least as tightly as `min_bp`.

## Bytecode model

Each instruction is a tagged 4-byte record:

```rust
pub struct Instruction {
  pub opcode: OpCode, // 1 byte (#[repr(u8)])
  pub operand: u16, // 2 bytes
}
```

The operand is a 16-bit slot. Its meaning depends on the opcode. Common shapes:

| OpCode | Operand interpretation |
|---------------------------|------------------------------------------------------|
| `LoadConst` | constant pool index |
| `LoadName` / `StoreName` | name slot index |
| `Add`, `Sub`, ... | unused (IC keyed by ip) |
| `Call` | `(num_kw << 8) \| num_pos` |
| `BuildList` / `BuildTuple` / `BuildSet` | element count |
| `BuildDict` | key-value pair count |
| `BuildSlice` | parts count (2 or 3) |
| `Jump` / `JumpIfFalse` | target instruction index |
| `ForIter` | jump target on iterator exhaustion |
| `Phi` | target slot; sources stored in `chunk.phi_sources` |
| `UnpackEx` | `(before << 8) \| after` |
| `MakeFunction` | function index in `chunk.functions` |

Operands and the constant pool, name table, and instruction stream per chunk are all capped at `u16::MAX` (65,535).

## Expression parsing

`expr_bp(min_bp)` runs the Pratt loop. `parse_atom` advances one token and routes by kind:

```text
Name                     -> name() (handles assignment, walrus, calls)
String                   -> emit Str constant; concatenate adjacent String tokens
Int / Float              -> emit numeric constant; ints widen i64->i128, beyond ±2¹²⁷ is a parse error
True/False/None/Ellipsis -> emit dedicated load opcode
FstringStart             -> fstring()
Lbrace                   -> brace_literal() (dict, set, comprehension)
Lsqb                     -> list_literal() (list, comprehension)
Lpar                     -> grouped expr, tuple, generator, or empty tuple
Lambda                   -> parse_lambda()
```

After an atom, `postfix_tail()` handles trailers (subscript, attribute, call), iterating until none apply. So `fns[0](-3)`, `obj.method()`, `(lambda x: x)(3)`, and `compose(f, g)(x)` all parse uniformly.

`*args` / `**kwargs` are accepted in **call** position. Starred unpacking also works in list (`[*a, *b]`), set (`{*s}`), and dict (`{**d1, **d2}`) literals, lowering via `ListExtend` / `SetUpdate` / `DictUpdate`; tuple-literal unpacking (`(1, *xs, 2)`) is not.

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

`infix_bp` handles comparison chaining (`a < b < c`). When a comparison opcode is followed by another comparison token, the parser:

- stores the middle value in a synthetic `#cmp_N` slot
- emits the first comparison
- short-circuits on false
- reuses the stored value for the next comparison

## Short-circuit lowering

`and` / `or` lower to `JumpIfFalseOrPop` / `JumpIfTrueOrPop`. These superinstructions peek the stack top. They pop only if execution continues; otherwise they jump and leave the value on the stack:

```text
a and b

LoadName a
JumpIfFalseOrPop -> end
LoadName b
end:
```

Preserves operand identity (returns the actual value, not a coerced bool) without an extra opcode.

## Conditional expression lowering

`a if cond else b` parses value-first: the value is textually first and there is no AST to reorder. On reaching `if`, the parser drains the value's instructions and re-emits them after the condition, shifting internal jump targets, so the condition runs first and only one branch executes (CPython order):

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

Each binding emits a fresh slot with an incremented version. The parser keeps a `HashMap<String, u32>` of name -> current version. Names in `chunk.names` are stored as `name_version`:

```python
x = 1 # x_1
x = 2 # x_2
y = x # y_1, references x_2
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

Undefined names target version 0 (`x_0`), filled by the host before execution (VM seeds globals like `print_0`). Still unbound at load time -> `NameError`.

## Phi nodes at joins

At each control-flow boundary the parser pushes a `JoinNode { backup, then }` onto a stack:

```text
enter_block() -> snapshot current versions into JoinNode.backup (and reset to the same baseline for the if branch).
mid_block() -> snapshot post-then versions into JoinNode.then; restore baseline (max of backup, then-state) for else.
commit_block() -> diff (then ∪ post) against (backup), emit Phi for each name that diverged.
```

Each `Phi` carries the target slot (new version after join) in its operand. Source slots live in `chunk.phi_sources`, indexed by `chunk.phi_map[ip]` at runtime. This keeps `Instruction` at 4 bytes.

```python
if cond:
  x = 1
else:
  x = 2
print(x)
```

```text
LoadName cond_0
JumpIfFalse else_label
LoadConst 0 *(1)
StoreName x_1
Jump end_label
else_label:
LoadConst 1 *(2)
StoreName x_2
end_label:
Phi x_3 *(sources: x_1, x_2)
LoadName x_3
CallPrint 1
```

Runtime resolves `Phi` by reading whichever source slot is `Some`: exactly one branch executed.

## Statement dispatch

`stmt()` peeks the leading token and routes:

```text
if       -> if_stmt (with elif chain, optional else)
for      -> for_stmt_inner (sync iter, optional else)
while    -> while_stmt (with break/continue patches)
match    -> match_stmt
def      -> func_def_inner
class    -> class_def (__init__, attributes, methods)
with     -> with_stmt_inner (multi-target, async variant)
try      -> try_stmt (except, else, finally, raise)
import   -> import_stmt (compile-time resolver lookup)
from     -> parse_from_stmt (named / star imports, same path)
type     -> type-alias declaration
yield    -> yield expr / yield from
async    -> async def / for / with
@        -> decorator stack + def or class (peeks `class` after the @-list)
return   -> expr + ReturnValue
raise    -> expr + Raise / RaiseFrom
break    -> emits Jump, back-patched to the loop exit
continue -> jump to current loop_start
del / global / nonlocal / pass -> direct emit
assert   -> Assert opcode; the `, msg` form lowers to a conditional raise of AssertionError(msg)
Name     -> name_stmt (assignment, augmented, indexed, attribute, call)
```

Each statement returns a bool: did it leave a value on the stack. The driver emits `PopTop` after expression-shaped statements (`x.method()`, `1 + 2` at module level), but not after statement-shaped ones (assignment, control flow).

Decorators apply to `def` and `class`. The `@` arm peeks for `class` after the decorator stack. Each decorator wraps via `Call,1` between `MakeFunction`/`MakeClass` and the final `StoreName`.

`raise X from Y` lowers to `RaiseFrom`, pops cause then exception so `X` surfaces. Bare `raise` re-raises. `__cause__` / `__context__` not exposed. `except` matching walks `EXC_PARENTS` so `except Exception` catches subclasses like `isinstance(e, Exception)` does.

## Lambda and function bodies

Lambdas and `def` both compile their body into a *fresh* SSAChunk:

```rust
self.with_fresh_chunk(|s| {
  s.ssa_versions = outer_versions.clone();
  for p in &params { s.ssa_versions.insert(param_base_name(p).to_string(), 0); }
  s.expr(); // or compile_block_body for def
  s.chunk.emit(OpCode::ReturnValue, 0);
});
```

Free variables (non-parameters with no local binding) are looked up in the outer chunk. `MakeFunction` captures matching slots from the enclosing scope into `captures` as shared cells (1-element heap lists, registered per call frame by the enclosing slot), so sibling closures over the same variable observe each other's `nonlocal` writes. Nested `def`/`lambda` push their free names back into the parent's name table. Capture propagates through any depth (`A -> B -> C` where `C` references a var in `A`).

Parameter slots: `Normal`, `Star` (`*args`), `DoubleStar` (`**kwargs`). Lone `*` separator marks following params as keyword-only. Defaults live in `HeapObj::Func.defaults` and bind to the `=`-marked params in source order, so a default before `*args` and keyword-only defaults both apply correctly. Annotations (`x: T`, `-> T`) parse and drain to `chunk.annotations` (tooling-only).

`compile_body` checks impurity opcodes (`StoreItem`, `DelItem`, `StoreAttr`, `DelAttr`, `CallPrint`, `CallInput`, `Global`, `Nonlocal`, `Raise`, `RaiseFrom`, `Yield`, `LoadAttr`) to set `body.is_pure`, the flag that gates template memoisation ([Design](/implementation/design#concepts)). This static gate is complemented at runtime by an observed-impurity check that propagates through calls, so a side-effecting builtin reached as a first-class value (e.g. `print` passed to a wrapper) disqualifies the caller too.

## Type annotations

Parsed for source compatibility, discarded at runtime:

```python
counter: int = 0 # annotation 'int' parsed and stored, slot still gets 0
def f(x: int) -> int:
  # annotations on params and return parsed and skipped
  return x
```

Recorded in `chunk.annotations: HashMap<String, String>` for tooling. No code emitted. `f.__annotations__` is not exposed at runtime, but `f.__name__` is (resolved in `resolve_attr`, also on type objects and classes).

## Comprehensions and generators

List/set/dict comprehensions support multi-`for` and multi-`if`. Lower to `BuildList` / `BuildSet` / `BuildDict` plus a loop scaffold using `ListAppend` / `SetAdd` / `MapAdd`.

Generator expressions `(i*2 for i in xs)` lower eagerly to `BuildList`, operationally equivalent to `[i*2 for i in xs]`. This is deliberate: template memoisation needs hashable, finite args, and lazy generators wouldn't memoise. For unbounded streams, write a `def` with `yield` (real `HeapObj::Coroutine`).

Async comprehensions (`[x async for x in y]`) and starred-unpack inside comprehensions unsupported.

## F-string lowering

```python
f"hello {name}, age {age}"
```

Lowers to:

```text
LoadConst "hello "
LoadName name_v
FormatValue 0
LoadConst ", age "
LoadName age_v
FormatValue 0
BuildString 4
```

`FormatValue`'s 16-bit operand is a small flags field:
- bit 0, set when a format spec string is on the stack just below the value (collected as the raw text between `:` and `}` and emitted as a constant).
- bits 1–2, conversion: `0` none, `1` `!r`, `2` `!s`, `3` `!a`.

VM applies conversion first, then the spec mini-language `[[fill]align][sign][#][0][width][,][.precision][type]` with type chars `s d b o x X f F e E g G n % c`. `n` aliases `d` (no locale). `=` self-documenting form (`{expr=}`) emits a literal `expr=` prefix. Adjacent string literals concatenate at parse time. Spec parse failures -> `ValueError` at runtime.

## Limits

| Constant | Value | Purpose |
|----------------------|-----------|----------------------------------------|
| `MAX_EXPR_DEPTH` | 200 | Cap on recursive expression parsing |
| `MAX_INSTRUCTIONS` | 65,535 | Cap on instructions per chunk |

`MAX_EXPR_DEPTH` -> diagnostic ("expression too deeply nested"). `MAX_INSTRUCTIONS` -> sets `chunk.overflow = true`, reported at end of parsing. The instruction stream is cleared rather than dispatched.

## References

- Pratt. *Top Down Operator Precedence* (POPL 1973). Precedence climbing.
- Cytron et al. *Efficiently Computing Static Single Assignment Form* (TOPLAS 1991). SSA, phi-nodes.
- Crafting Interpreters, by Robert Nystrom: craftinginterpreters.com, single-pass codegen patterns.
- Casey et al. *Towards Superinstructions for Java Interpreters* (SCOPES 2003). LoadAttr+Call fusion.
