*Update this documentation upon completion of the compiler (https://edgepython.com/resources/architecture)*

```bash

lexer.rs
  Tokenizes Python source into a stream of spanned Token variants.

parser.rs
  Single pass: consumes lexer tokens and emits bytecode directly. No abstract syntax tree built, fast and minimal memory.
```

*upx packer*