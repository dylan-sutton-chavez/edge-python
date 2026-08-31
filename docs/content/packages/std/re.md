---
title: "re (web, native)"
description: "Regular expressions on a backtracking engine."
---

`re` is regular expressions on a backtracking engine. Import it by bare name, both runtimes resolve it with no manifest. To pin a different version, use a `packages.json` alias, see [Modules](/reference/modules#packagesjson).

Functions are `match`, `search`, `fullmatch`, `findall`, `groups`, `span`, and `sub`, all taking `(pattern, string)`. `compile(pattern)` returns a pattern object with the same operations as methods. Flags go inline: `(?i)`, `(?s)`, `(?m)`.

The match functions return the matched string or `None`, there are no Match objects. `groups` returns the capture groups as a list, `span` returns `[start, end]` in codepoint offsets, and `findall` with more than one group returns a list of lists. In `sub`, `\1` and `\g<name>` expand groups. The syntax covers `.`, Unicode-aware classes `\d` `\w` `\s` and their negations, sets and ranges, anchors `^` `$` `\b` `\B`, quantifiers `*` `+` `?` `{m,n}` with lazy forms, capturing, non-capturing, and named groups, backreferences `\1` and `(?P=name)`, alternation, lookahead, and fixed-width lookbehind. Not supported: Unicode property classes `\p{...}`, `\N{...}` escapes, atomic groups, possessive quantifiers, conditionals, and scoped flags `(?i:...)`. Invalid patterns raise `ValueError`. A per-call step budget raises `RuntimeError` on catastrophic backtracking instead of hanging, and a match that recurses past the stack allowance raises the same error instead of overflowing the stack. `compile` and the module-level functions share one compiled-pattern cache, so a hot loop parses each pattern once.

```python
from re import search, sub, findall

print(search(r'(\d+)-(\d+)', 'order 12-34'))
print(sub(r'\s+', '_', 'a  b   c'))
print(findall(r'\w+', 'one two three'))
```

```text Output
12-34
a_b_c
['one', 'two', 'three']
```

`groups` and `span` give the captures and the offsets, and `fullmatch` returns `None` when the whole string does not match:

```python
from re import groups, span, fullmatch

print(groups(r'(\w+)@(\w+)', 'mail ada@edge'))
print(span(r'\d+', 'abc 42 def'))
print(fullmatch(r'\d+', '123'))
print(fullmatch(r'\d+', '12a'))
```

```text Output
['ada', 'edge']
[4, 6]
123
None
```

`sub` expands backreferences, and flags go inline:

```python
from re import sub, search

print(sub(r'(\w+) (\w+)', r'\2 \1', 'first last'))
print(sub(r'(?P<y>\d{4})', r'[\g<y>]', 'in 2024'))
print(search(r'(?i)EDGE', 'the edge engine'))
```

```text Output
last first
in [2024]
edge
```

`compile` returns a pattern object with the same operations as methods:

```python
from re import compile

word = compile(r'\w+')
print(word.findall('one two'))
print(word.sub('#', 'one two three'))
```

```text Output
['one', 'two']
# # #
```
