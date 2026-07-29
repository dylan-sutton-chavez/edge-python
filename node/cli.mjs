#!/usr/bin/env node
// cli.mjs — usage:
//   node cli.mjs run <script.py> [--wasm ./compiler.wasm]
//   node cli.mjs repl            [--wasm ./compiler.wasm]
//
// Defaults to ./compiler.wasm next to this file if --wasm isn't given.
// Set EDGE_WASM=/path/to/compiler.wasm as an alternative to --wasm.
//
// From Claude:  https://claude.ai/share/c7fb9bc5-8532-4cc9-8b42-24c71a0b57fc

import { createInterface } from 'node:readline';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { loadCompiler, makeInstance, runScript, replEval } from './engine.mjs';

const __dirname = path.dirname(fileURLToPath(import.meta.url));

function parseArgs(argv) {
  const args = { _: [] };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === '--wasm') args.wasm = argv[++i];
    else args._.push(a);
  }
  return args;
}

function resolveWasmPath(args) {
  return args.wasm || process.env.EDGE_WASM || path.join(__dirname, 'compiler.wasm');
}

async function cmdRun(args) {
  const file = args._[0];
  if (!file) {
    console.error('usage: edge-node run <script.py> [--wasm ./compiler.wasm]');
    process.exit(2);
  }
  const wasmPath = resolveWasmPath(args);
  const src = await readFile(file, 'utf8');

  const onLine = (text) => process.stdout.write(text);
  const wasmModule = await loadCompiler(wasmPath);
  const exports = await makeInstance(wasmModule, { onLine });

  const result = await runScript(exports, src);
  if (result.kind === 'error') {
    process.stderr.write(`error: ${result.message}\n`);
    process.exit(1);
  } else if (result.kind === 'exit') {
    process.exit(result.code);
  } else {
    if (result.out) process.stdout.write(result.out);
  }
}

async function cmdRepl(args) {
  const wasmPath = resolveWasmPath(args);
  const wasmModule = await loadCompiler(wasmPath);

  const onLine = (text) => process.stdout.write(text);
  let exports = await makeInstance(wasmModule, { onLine });

  console.log('EdgePython Node shim  ·  .reset to start fresh  ·  .exit or Ctrl+D to quit');
  const rl = createInterface({ input: process.stdin, output: process.stdout, prompt: '>>> ' });
  rl.prompt();

  rl.on('line', async (line) => {
    const trimmed = line.trim();
    if (trimmed === '.exit') {
      rl.close();
      return;
    }
    if (trimmed === '.reset') {
      exports = await makeInstance(wasmModule, { onLine });
      console.log('(reset)');
      rl.prompt();
      return;
    }
    if (trimmed === '') {
      rl.prompt();
      return;
    }

    try {
      const result = await replEval(exports, line);
      if (result.kind === 'error') {
        process.stderr.write(`error: ${result.message}\n`);
      } else if (result.kind === 'exit') {
        console.log(`(SystemExit: ${result.code})`);
      } else if (result.out) {
        process.stdout.write(result.out);
      }
    } catch (e) {
      process.stderr.write(`shim error: ${e?.message ?? e}\n`);
    }
    rl.prompt();
  });

  rl.on('close', () => {
    console.log();
    process.exit(0);
  });
}

async function main() {
  const [cmd, ...rest] = process.argv.slice(2);
  const args = parseArgs(rest);

  if (cmd === 'run') return cmdRun(args);
  if (cmd === 'repl') return cmdRepl(args);

  console.error('usage: edge-node <run|repl> [...args]');
  process.exit(2);
}

main().catch((e) => {
  console.error(e?.stack ?? String(e));
  process.exit(1);
});
