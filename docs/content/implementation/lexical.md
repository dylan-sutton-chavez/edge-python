---
title: "Lexical"
description: "Tokenization, indentation, f-strings, and source-level limits."
---

## Overview

Hand-written, LUT-driven scanner. It walks source as raw bytes and produces `Token { kind, line, start, end }`. Tokens carry byte indices, never text copies. The parser slices lazily for identifier and string content.

Linear time *O(n)*. Per-byte dispatch is branchless through two lookup tables. Lex-time diagnostics (unterminated strings, bad indent, unknown bytes, malformed underscores, oversized f-string nesting) collect in a `Vec<LexError>` returned alongside the token stream. The parser folds them in for a single coherent report.

A leading UTF-8 BOM (`EF BB BF`) is stripped before tokenization so the first identifier doesn't fuse with the marker.

## Token kinds

The token set tracks Python 3.13.12 closely. Categories implemented:

- **Keywords**: `False`, `None`, `True`, `and`, `as`, `assert`, `async`, `await`, `break`, `class`, `continue`, `def`, `del`, `elif`, `else`, `except`, `finally`, `for`, `from`, `global`, `if`, `import`, `in`, `is`, `lambda`, `nonlocal`, `not`, `or`, `pass`, `raise`, `return`, `try`, `while`, `with`, `yield`.
- **Soft keywords**: `type`, `match`, and `case` demote to `Name` when followed by `(`, `:`, `=`, `,`, `)`, `]`, `Newline`, or `EOF`, so `type()`, `match(...)`, and identifiers named like them stay usable. At statement start (`match x:`) they keep keyword force.
- **Wildcard**: Underscore (`_`) gets its own `Underscore` token. The parser distinguishes wildcard from name use.
- **Operators**: 1-, 2-, and 3-character operator forms (`+`, `==`, `**=`, `//=`, etc.).
- **Delimiters**: `( ) [ ] { } : , ; .`.
- **Literals**: `Name`, `Int`, `Float`, `String`, `Bytes`. There is no `Complex` token; a trailing `j` / `J` is **not** lexed as a complex suffix (see Numeric literals).
- **F-string segments**: `FstringStart`, `FstringMiddle`, `FstringEnd`.
- **Whitespace and structure**: `Comment`, `Newline`, `Indent`, `Dedent`, `Nl`, `Endmarker`.

## Dispatch tables

Two compile-time tables in `lexer/tables.rs`:

```rust
// Bit flags per byte: ID_START, ID_CONT, DIGIT, SPACE.
pub static BYTE_CLASS: [u8; 256] = { /* ... */ };

// Single-char operator dispatch.
pub static SINGLE_TOK: [u8; 128] = { /* ... */ };
pub const SINGLE_MAP: [TokenType; 24] = { /* ... */ };
```

Identifiers, digits, and whitespace use a `scan_while(pred)` driver looping over `BYTE_CLASS[b] & FLAG`. Single-char operators do `b -> SINGLE_TOK[b] -> SINGLE_MAP[i]`, two indexed loads. Keyword lookup is routed by `(length, first_byte)` to skip most `memcmp`s.

## Numeric literals

```python
42
1_000_000 # underscore separators
0xDEAD_BEEF # hex
0o777 # octal
0b1010_1010 # binary
3.14
.5 # leading-dot float
1e-5 # exponent
```

The scanner handles base prefixes (`0x` / `0o` / `0b`, case-insensitive), underscore separators, optional exponents, and leading-dot form. `Int` and `Float` are the only numeric token kinds.

Underscores must sit between digits. Leading, trailing, or doubled underscores raise `invalid '_' in numeric literal` or `consecutive '_' in numeric literal`. Empty radix body (`0x`, `0o`, `0b`) raises `missing digits in numeric literal`. Trailing dot (`5.`) is valid. Empty exponent body (`1e`) is left to the float parser to avoid false-positives in format specs.

Complex literals unsupported: `1j` lexes as `Int(1)` + `Name("j")`.

## String prefixes

```python
'plain' # str
b'bytes' # bytes (distinct Bytes token)
r'raw\n' # raw
u'unicode' # unicode
br'rawbytes' # raw bytes
RB'mixed' # any case combination
f'fstring' # f-string (separate token sequence)
fr'raw fstring' # raw f-string
"""triple""" # triple-quoted, single or double
```

A leading prefix is recognised before the opening quote by the identifier scanner, verified against `is_string_prefix` / `is_fstring_prefix` / `is_bytes_prefix`. Triple-quoted strings span newlines (bumping `line` per `\n`). Backslash escapes are consumed at lex time but decoded by the parser. Recognised escapes: `\n \t \r \a \b \f \v \\ \' \" \xHH \uHHHH \UHHHHHHHH` plus 1–3 digit octal (`\012` -> `\n`, `\101` -> `A`). `\N{NAME}` is unimplemented. The 200 KB Unicode-name database is too costly for the WASM artifact.

Errors anchor on the opening quote so the `^` marker points at the offender, not at end-of-line:

* `unterminated string literal`
* `unterminated triple-quoted string literal`
* `unterminated f-string literal`

## F-strings

F-strings decompose into a token sequence rather than a single `String` token. The parser consumes it directly:

```text
f'a {x} b {y + 1}!'

FstringStart
FstringMiddle("a ")
Lbrace
Name(x)
Rbrace
FstringMiddle(" b ")
Lbrace
Name(y) Plus Int(1)
Rbrace
FstringMiddle("!")
FstringEnd
```

Expression tokens between `{` and `}` are emitted by the main lexer, not the f-string scanner. Interpolations get the full expression grammar, without special casing.

`{{` and `}}` are escaped literal braces, with no `Lbrace` / `Rbrace`. They survive into `FstringMiddle` text and are unescaped by the parser.

Triple-quoted f-strings follow the same structure, with newlines embedded in middle segments. Nested f-strings are tracked via `fstring_stack` so each `}` resumes the right outer template. Nesting deeper than `MAX_FSTRING_DEPTH = 200` raises `f-string nesting depth exceeds maximum (200)`. EOF inside an open f-string raises `unterminated f-string literal` and synthesises a closing `FstringEnd` for a balanced sequence.

## Indentation

Edge Python uses an INDENT/DEDENT model. The scanner tracks a stack of column counts and emits structural tokens at line boundaries:

| Situation | Tokens emitted |
|-------------------------------------|---------------------------------------------------|
| Blank line or comment-only line | `Nl` |
| Inside `(...)`, `[...]`, `{...}` | `Nl` (no `Indent` / `Dedent`) |
| Indentation increased | `Indent`, `Newline` |
| Indentation decreased | `Dedent` (× n levels), `Newline` |
| Indentation unchanged | `Newline` |
| Dedent doesn't match an outer level | diagnostic `unindent does not match any outer indentation level` |
| Mixed tabs and spaces in indent | `Endmarker` (lex halt) + diagnostic |

The `nesting` counter is bumped by `(`, `[`, `{` and decremented by `)`, `]`, `}`. While `nesting > 0`, line breaks emit `Nl` and the indent stack is frozen, allowing multi-line expressions inside brackets without spurious INDENT/DEDENT.

At EOF the lexer drains remaining levels off `indent_stack` for clean block closure, then emits `Endmarker`. Backslash line continuation (`\` + newline) is consumed by the whitespace scanner, joining the two physical lines. ASCII bytes with no operator slot (`$`, `?`, `` ` ``) raise `unexpected character` and are skipped.

## Soft-keyword disambiguation

`type`, `match`, and `case` are soft keywords. Each collides with a builtin or identifier use (`type()`, a function named `match`, a `case` variable), so the lexer disambiguates by peeking the next token:

```python
type X = int # 'type' is a keyword (alias declaration)
type = None # 'type' is an identifier
match x: # 'match' is a keyword (statement)
match(s, p) # 'match' is an identifier (call)
```

If the token following the word is one of `(`, `:`, `=`, `,`, `)`, `]`, `Newline`, or `EOF`, it downgrades to `Name`. Otherwise it stays a keyword. A statement subject like `match x:` starts with a name or literal, so it keeps keyword force. A parenthesized subject like `match (a, b):` is the one case this heuristic misreads as a call.

`_` always emits as `Underscore`. The parser distinguishes wildcard from name use grammatically.

## Comments

`#` to end-of-line. Emitted as a `Comment` token (not discarded) so tools can round-trip source. The parser ignores `Comment` and `Nl` during `peek()`.

## Limits

Hard caps to prevent asymmetric DoS (small input exhausting memory/time). Hitting any halts lexing with `Endmarker`:

| Constant | Value | Purpose |
|---------------------|-----------|------------------------------------------|
| `MAX_SOURCE_SIZE` | 10 MiB | Reject oversized input upfront |
| `MAX_INDENT_DEPTH` | 100 | Cap on the indentation stack |
| `MAX_FSTRING_DEPTH` | 200 | Cap on nested f-string contexts |

These match the OWASP A04:2021 advice on resource exhaustion in interpreters.

## Why offset-based tokens

A `Token` carries a kind tag plus three byte offsets:

```rust
pub struct Token {
	pub kind: TokenType,
	pub line: usize,
	pub start: usize,
	pub end: usize,
}
```

The parser slices `&source[t.start..t.end]` lazily for identifier names, string content, or numeric literals. Result:

- Lexer never allocates a `String` per identifier.
- `lexeme(&t)` is a zero-copy `&str` that lives as long as the source buffer.
- Diagnostics get exact byte offsets for free; error column is a single `rfind('\n')`.

## References

- Python language reference, *Lexical analysis*: docs.python.org/3/reference/lexical_analysis
- OWASP, *Insecure Compiler Optimization*: owasp.org/www-community/vulnerabilities/Insecure_Compiler_Optimization
- Aho, Sethi & Ullman. *Compilers: Principles, Techniques and Tools* (1986). LUT-driven scanners.
