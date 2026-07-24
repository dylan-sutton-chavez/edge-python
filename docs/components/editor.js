/* 
Pure-text editor on CodeJar. We own keys, auto-pair, selection-wrap, paste/drop, copy/cut fencing; CodeJar renders + history. 
*/

import { CodeJar } from 'codejar';
import { escapeHtml } from './shiki';

const MAX_LINES = 999;
const TAB_SIZE = 2;

// `highlight(text) -> html` is injected (Shiki in the docs); CodeJar calls it on every render.
export function createEditor({ ed, defaultCode, onRun, highlight }) {

    // constants

    const PAIRS = { '(': ')', '[': ']', '{': '}', '"': '"', "'": "'" };
    const OPENERS = new Set(Object.keys(PAIRS));
    const CLOSERS = new Set(Object.values(PAIRS));
    const STRING_START = /^([fFrRbBuU]{0,2})("""|'''|"|')/;
    const WS = /[ \t]/;
    // `decreaseIndentPattern`, dedent on `:`.
    const DEDENT_RE = /^\s*(?:elif|else|except|finally|case)\b[^:]*:$/;

    // pure helpers

    // Walk from 0 to caret toggling code/string. `quote === ''` means code.
    const stringCtx = (src, caret) => {
        let i = 0, quote = '', isF = false;
        while (i < caret) {
            if (!quote) {
                if (src[i] === '#') {
                    const nl = src.indexOf('\n', i);
                    if (nl === -1 || nl >= caret) return { inStr: false, isF: false };
                    i = nl + 1; continue;
                }
                const m = src.slice(i).match(STRING_START);
                if (m && i + m[0].length <= caret) {
                    quote = m[2]; isF = /[fF]/.test(m[1]);
                    i += m[0].length; continue;
                }
                i++;
            } else {
                if (quote.length === 1 && src[i] === '\\') { i += 2; continue; }
                if (src.slice(i, i + quote.length) === quote) {
                    i += quote.length; quote = ''; isF = false; continue;
                }
                i++;
            }
        }
        return { inStr: !!quote, isF };
    };

    const lineAt = (text, caret) => {
        const start = text.lastIndexOf('\n', caret - 1) + 1;
        const nl = text.indexOf('\n', caret);
        const end = nl === -1 ? text.length : nl;
        return { start, end, body: text.slice(start, end), col: caret - start };
    };
    const firstNonWS = (s) => { for (let i = 0; i < s.length; i++) if (!WS.test(s[i])) return i; return s.length; };
    
    // `prevIndentTabStop`, distance to the previous tab stop from `col`.
    const prevTabDist = (col) => { const r = col % TAB_SIZE; return r === 0 ? TAB_SIZE : r; };

    // Full lines covered by `sel`; excludes the trailing line if `sel.end` sits exactly past a `\n`.
    const lineRange = (text, sel) => {
        const start = text.lastIndexOf('\n', sel.start - 1) + 1;
        const anchor = (sel.end > sel.start && text[sel.end - 1] === '\n') ? sel.end - 1 : sel.end;
        let end = text.indexOf('\n', anchor);
        if (end === -1) end = text.length;
        return { start, end };
    };

    // Strip common leading-whitespace from non-empty lines so a snippet pasted from deeper context lands flush.
    const dedentCommon = (s) => {
        const lines = s.split('\n');
        const nonEmpty = lines.filter(l => l.trim().length > 0);
        if (nonEmpty.length === 0) return s;
        const min = Math.min(...nonEmpty.map(l => l.match(/^[ \t]*/)[0].length));
        return min ? lines.map(l => l.slice(Math.min(l.length, min))).join('\n') : s;
    };

    // Round-trip helper for paste: strip our own ```python fence transparently.
    const unwrapFence = (s) => {
        const m = s.match(/^```python\n([\s\S]*?)\n```\s*$/);
        return m ? m[1] : s;
    };

    // AltGr emits a printable char while flagging Ctrl+Alt; real Ctrl+Alt shortcuts produce a letter/digit `key`.
    const isAltGrChar = (e) => e.ctrlKey && e.altKey && e.key.length === 1 && !/^[A-Za-z0-9]$/.test(e.key);

    // mutable state

    /* Caret right after last auto-pair; Backspace pair-collapse fires only when caret still matches. Cleared on external mutation. */
    let autoPairCaret = -1;
    // True while the IME has an open composition; we must stay out of the way.
    let composing = false;

    // transitions

    const transitions = {
        // Typed character: skip existing closer, or auto-close an opener.
        char: (text, caret, key) => {
            if (CLOSERS.has(key) && text[caret] === key) {
                return { text, caret: caret + 1 };
            }
            if (!OPENERS.has(key)) return null;
            const { inStr, isF } = stringCtx(text, caret);
            // Regular strings skip auto-close; f-strings still expand `{` to `{}` for expression interpolation.
            if (inStr && !(isF && key === '{')) return null;
            // Triple-quote opening: "" + " -> """""" (or '' + ' -> '''''').
            if ((key === '"' || key === "'") && caret >= 2 && text[caret - 2] === key && text[caret - 1] === key) {
                return {
                    text: text.slice(0, caret - 2) + key.repeat(6) + text.slice(caret),
                    caret: caret - 2 + 3,
                };
            }
            return {
                text: text.slice(0, caret) + key + PAIRS[key] + text.slice(caret),
                caret: caret + 1,
                autoPair: caret + 1,
            };
        },

        // Wrap a non-empty selection in opener/closer (`autoSurround`).
        wrapSelection: (text, sel, key) => ({
            text: text.slice(0, sel.start) + key + text.slice(sel.start, sel.end) + PAIRS[key] + text.slice(sel.end),
            caretStart: sel.start + 1,
            caretEnd: sel.end + 1,
        }),

        // Backspace: collapse fresh auto-pair, else `useTabStops` snap when caret sits inside leading whitespace.
        backspace: (text, caret) => {
            if (caret === 0) return null;
            if (caret === autoPairCaret && PAIRS[text[caret - 1]] === text[caret]) {
                return { text: text.slice(0, caret - 1) + text.slice(caret + 1), caret: caret - 1 };
            }
            if (!WS.test(text[caret - 1])) return null;
            const lnRow = lineAt(text, caret);
            if (lnRow.col === 0 || lnRow.col > firstNonWS(lnRow.body)) return null;
            const dist = prevTabDist(lnRow.col);
            return { text: text.slice(0, caret - dist) + text.slice(caret), caret: caret - dist };
        },

        // Tab: insert spaces to the next tab stop based on the current column.
        tab: (text, caret) => {
            const lnRow = lineAt(text, caret);
            const pad = ' '.repeat(TAB_SIZE - (lnRow.col % TAB_SIZE));
            return { text: text.slice(0, caret) + pad + text.slice(caret), caret: caret + pad.length };
        },

        // Shift+Tab: dedent the current line by up to one tab stop.
        shiftTab: (text, caret) => {
            const lnRow = lineAt(text, caret);
            const indent = firstNonWS(lnRow.body);
            if (indent === 0) return null;
            const removed = indent - Math.floor((indent - 1) / TAB_SIZE) * TAB_SIZE;
            const newCaret = lnRow.col >= removed ? caret - removed : lnRow.start;
            return {
                text: text.slice(0, lnRow.start) + lnRow.body.slice(removed) + text.slice(lnRow.end),
                caret: newCaret,
            };
        },

        // Enter: inherit indent, +1 level after `:`/openers. Between opener and matching closer, split closer (`IndentOutdent`).
        enter: (text, caret) => {
            const lnRow = lineAt(text, caret);
            const before = lnRow.body.slice(0, lnRow.col);
            const indent = before.match(/^[ \t]*/)[0];
            const extra = /[:\[({][ \t]*$/.test(before) ? ' '.repeat(TAB_SIZE) : '';
            const pad = indent + extra;
            const opener = before.replace(/[ \t]+$/, '').slice(-1);
            const splitBracket = '[({'.includes(opener) && PAIRS[opener] === text[caret];
            const tail = splitBracket ? `\n${pad}\n${indent}` : `\n${pad}`;
            return { text: text.slice(0, caret) + tail + text.slice(caret), caret: caret + 1 + pad.length };
        },

        // `:` after `else`/`elif`/`except`/`finally`/`case` head. Insert the colon, then dedent the line by one tab stop.
        colon: (text, caret) => {
            const inserted = text.slice(0, caret) + ':' + text.slice(caret);
            const lnRow = lineAt(inserted, caret + 1);
            if (!DEDENT_RE.test(lnRow.body)) return null;
            const remove = Math.min(firstNonWS(lnRow.body), TAB_SIZE);
            if (remove === 0) return { text: inserted, caret: caret + 1 };
            return {
                text: inserted.slice(0, lnRow.start) + inserted.slice(lnRow.start + remove),
                caret: caret + 1 - remove,
            };
        },

        // Tab on selection: prepend `TAB_SIZE` spaces to every line in the range.
        tabSelection: (text, sel) => {
            const { start, end } = lineRange(text, sel);
            const lines = text.slice(start, end).split('\n');
            const pad = ' '.repeat(TAB_SIZE);
            return {
                text: text.slice(0, start) + lines.map(l => pad + l).join('\n') + text.slice(end),
                caretStart: sel.start + TAB_SIZE,
                caretEnd: sel.end + lines.length * TAB_SIZE,
            };
        },

        // Shift+Tab on selection: dedent each line in the range by up to `TAB_SIZE`.
        shiftTabSelection: (text, sel) => {
            const { start, end } = lineRange(text, sel);
            const lines = text.slice(start, end).split('\n');
            let firstRm = 0, totalRm = 0;
            const dedented = lines.map((l, i) => {
                const rm = Math.min(firstNonWS(l), TAB_SIZE);
                totalRm += rm;
                if (i === 0) firstRm = rm;
                return l.slice(rm);
            });
            if (totalRm === 0) return null;
            return {
                text: text.slice(0, start) + dedented.join('\n') + text.slice(end),
                caretStart: Math.max(start, sel.start - firstRm),
                caretEnd: sel.end - totalRm,
            };
        },
    };

    // CodeJar wiring

    const jar = CodeJar(ed,
        (node) => { node.innerHTML = highlight(node.textContent); },
        { spellcheck: false, addClosing: false, catchTab: false, preserveIdent: false }
    );

    // CodeJar forces `white-space: pre-wrap` (wraps long lines); override so long lines scroll horizontally.
    ed.style.whiteSpace = 'pre';
    ed.style.overflowWrap = 'normal';

    // Write `text` and place the caret at [start, end]; clear auto-pair marker.
    const writeAndRestore = (text, start, end = start) => {
        jar.updateCode(text);
        jar.restore({ start, end });
        autoPairCaret = -1;
    };

    const apply = (result) => {
        if (!result) return false;
        writeAndRestore(result.text, result.caretStart ?? result.caret, result.caretEnd ?? result.caret);
        autoPairCaret = result.autoPair ?? -1;
        return true;
    };

    // Insert plain text at caret, replacing any selection; used by paste/drop after normalisation. Enforces `MAX_LINES`.
    const insertAtCaret = (str) => {
        const pos = jar.save();
        const text = jar.toString();
        let next = text.slice(0, pos.start) + str + text.slice(pos.end);
        const all = next.split('\n');
        if (all.length > MAX_LINES) next = all.slice(0, MAX_LINES).join('\n');
        writeAndRestore(next, Math.min(pos.start + str.length, next.length));
    };

    // Paste/drop pipeline: strip ```python fence, normalise CRLF+tabs, smart-dedent common indent, enforce `MAX_LINES`.
    const insertNormalized = (raw) => {
        const norm = unwrapFence(raw).replace(/\r\n?/g, '\n').replace(/\t/g, ' '.repeat(TAB_SIZE));
        insertAtCaret(dedentCommon(norm));
    };

    // listeners
    // One AbortController so destroy() detaches every listener below (jar.destroy sheds only CodeJar's).
    const ac = new AbortController();
    const signal = ac.signal;

    /* Mirrors `event.isComposing`. Tracked explicitly so the post-`compositionend` `keydown` no-ops on Linux/Wayland where it re-fires. */
    ed.addEventListener('compositionstart', () => { composing = true; }, { signal });
    ed.addEventListener('compositionend', () => { composing = false; }, { signal });

    ed.addEventListener('keydown', (e) => {
        // IME, dead-key, or AltGr printable: never intercept, each path corrupts accents/CJK input otherwise.
        if (composing || e.isComposing || e.keyCode === 229) return;
        if (e.key === 'Dead') return;
        if (isAltGrChar(e)) return;

        // Ctrl/Cmd+Enter is the only modifier shortcut we own.
        if ((e.ctrlKey || e.metaKey) && e.key === 'Enter') {
            e.preventDefault(); onRun(jar.toString()); return;
        }
        // Any other modifier-bearing keystroke (Ctrl+S, Cmd+C, Alt+arrow, ...) belongs to the browser / CodeJar.
        if (e.ctrlKey || e.metaKey || e.altKey) return;

        // Hard cap on document size: block Enter once we'd exceed MAX_LINES.
        if (e.key === 'Enter' && jar.toString().split('\n').length >= MAX_LINES) {
            e.preventDefault(); return;
        }

        const pos = jar.save();
        const text = jar.toString();
        let result;
        if (pos.start !== pos.end) {
            // Selection-aware: multi-line indent/dedent + bracket wrap.
            if (e.key === 'Tab' && e.shiftKey) result = transitions.shiftTabSelection(text, pos);
            else if (e.key === 'Tab') result = transitions.tabSelection(text, pos);
            else if (OPENERS.has(e.key)) result = transitions.wrapSelection(text, pos, e.key);
            else return;
        } else {
            result =
                e.key === 'Backspace' ? transitions.backspace(text, pos.start) :
                e.key === 'Enter' ? transitions.enter(text, pos.start) :
                e.key === 'Tab' && e.shiftKey ? transitions.shiftTab(text, pos.start) :
                e.key === 'Tab' ? transitions.tab(text, pos.start) :
                e.key === ':' ? transitions.colon(text, pos.start) :
                (OPENERS.has(e.key) || CLOSERS.has(e.key)) ? transitions.char(text, pos.start, e.key) :
                null;
        }

        // stopImmediatePropagation blocks CodeJars bubble keydown on the same element; otherwise it re-runs indent and drifts the caret.
        if (apply(result)) { e.preventDefault(); e.stopImmediatePropagation(); }
    }, { capture: true, signal });

    // Text mutation outside `apply()` invalidates the auto-pair marker so manually-typed pairs don't collapse on backspace.
    ed.addEventListener('input', () => { autoPairCaret = -1; }, { signal });

    // Capture phase: preventDefault before CodeJar's bubble paste, which skips when defaultPrevented; otherwise both insert and the paste doubles.
    ed.addEventListener('paste', (e) => {
        const raw = (e.clipboardData ?? window.clipboardData)?.getData('text');
        if (raw == null) return;
        e.preventDefault();
        insertNormalized(raw);
    }, { capture: true, signal });

    // Drag-and-drop of text or .py files. Fires `drop`, not `paste`.
    const hasTextOrFiles = (dt) => {
        if (!dt) return false;
        const types = Array.from(dt.types || []);
        return types.includes('text/plain') || types.includes('Files');
    };
    ed.addEventListener('dragover', (e) => { if (hasTextOrFiles(e.dataTransfer)) e.preventDefault(); }, { signal });
    ed.addEventListener('drop', async (e) => {
        const dt = e.dataTransfer; if (!hasTextOrFiles(dt)) return;
        e.preventDefault();
        let raw = '';
        if (dt.files.length) {
            const f = dt.files[0];
            if (f.name.endsWith('.py') || (f.type || '').startsWith('text/')) {
                try { raw = await f.text(); } catch {}
            }
        } else {
            raw = dt.getData('text/plain') ?? '';
        }
        if (raw) insertNormalized(raw);
    }, { signal });

    /* Clipboard payload for copy/cut: collapsed selection falls back to current line; returns plain + html. */
    const copyPayload = () => {
        const sel = window.getSelection();
        let snippet = sel?.toString() ?? '';
        let collapsed = false;
        if (!snippet) {
            const lnRow = lineAt(jar.toString(), jar.save().start);
            snippet = lnRow.body;
            collapsed = true;
            if (!snippet) return null;
        }
        const trimmed = snippet.replace(/\n+$/, '');
        return {
            plain: '```python\n' + trimmed + '\n```',
            html: `<pre><code class="language-python">${escapeHtml(trimmed, true)}</code></pre>`,
            collapsed,
        };
    };

    const writeClipboard = (e, p) => {
        e.preventDefault();
        e.clipboardData.setData('text/plain', p.plain);
        e.clipboardData.setData('text/html', p.html);
    };

    ed.addEventListener('copy', (e) => {
        const p = copyPayload(); if (p) writeClipboard(e, p);
    }, { signal });

    // Capture + stop: CodeJar's bubble cut ignores defaultPrevented, so beat it here or the selection deletes twice.
    ed.addEventListener('cut', (e) => {
        const p = copyPayload(); if (!p) return;
        writeClipboard(e, p);
        e.stopImmediatePropagation();
        // Now remove source. Collapsed-cut deletes the whole line (newline included) so consecutive cuts compact upward.
        const text = jar.toString();
        const pos = jar.save();
        let next, caret;
        if (p.collapsed) {
            const lnRow = lineAt(text, pos.start);
            const removeEnd = lnRow.end < text.length ? lnRow.end + 1 : lnRow.end;
            next = text.slice(0, lnRow.start) + text.slice(removeEnd);
            caret = lnRow.start;
        } else {
            next = text.slice(0, pos.start) + text.slice(pos.end);
            caret = pos.start;
        }
        writeAndRestore(next, caret);
    }, { capture: true, signal });

    jar.updateCode(defaultCode);

    // destroy(): shed CodeJar's listeners and ours; caller also calls the highlighter's dispose().
    return { getCode: () => jar.toString(), setCode: (code) => jar.updateCode(code), destroy: () => { jar.destroy(); ac.abort(); } };
}
