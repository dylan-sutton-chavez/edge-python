"""
Importing a #[plugin_class] from Edge Python.
    Build slugify_mod.wasm `cargo build --release --target wasm32-unknown-unknown -p slugify-mod`
"""

from "./slugify_mod.wasm" import Slugger

s = Slugger()
s.add("Hello World")
s.add("From Edge Python")

print(s.build()) # -> hello-world-from-edge-python
print(s.shout()) # -> HELLO-WORLD-FROM-EDGE-PYTHON!
print(s.total_len()) # -> 27
print(s.repeat(2)) # -> hello-world-from-edge-pythonhello-world-from-edge-python

# Mutating state via pop returns the last part as Option -> str|None.
print(s.pop()) # -> python
print(s.pop()) # -> edge
print(s.build()) # -> hello-world-from

# Errors raised inside the module surface as typed Python exceptions.
try:
    print(s.repeat(-1))
except ValueError as e:
    print("caught:", e)
