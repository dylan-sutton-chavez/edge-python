'use client'

// CodeJar editor (Shiki + auto-pair/Tab/Ctrl+Enter) → Nextra Pre/Code (plain). `code`/`output`: snippet source & default terminal text (base64, remark plugin).

import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { Pre, Code, Button } from 'nextra/components'
import { run } from './worker'
import { escapeHtml } from './shiki'

function fromB64(b64) {
    if (!b64) return ''
    try {
        return new TextDecoder().decode(Uint8Array.from(atob(b64), (c) => c.charCodeAt(0)))
    } catch {
        return ''
    }
}

// Terminal control chars: `\r`/`\b`/`\t`/`\f` move the cursor and `\n` breaks the line.
function applyTerminalControls(text) {
    if (!/[\r\b\t\f]/.test(text)) return text
    const lines = []
    let line = ''
    let col = 0
    const put = (c) => { line = line.slice(0, col) + c + line.slice(col + 1); col += 1 }
    for (const ch of text) {
        if (ch === '\n') { lines.push(line); line = ''; col = 0 }
        else if (ch === '\r') { col = 0 }
        else if (ch === '\b') { if (col > 0) col -= 1 }
        else if (ch === '\f') { lines.push(line); line = ''; col = 0 }
        else if (ch === '\t') { do { put(' ') } while (col % 4) } // tab stop 4, matching the editor
        else { put(ch) }
    }
    lines.push(line)
    return lines.join('\n')
}

const PlayIcon = () => (
    <svg viewBox="0 0 24 24" stroke="currentColor" strokeWidth="2.5" fill="none" height="11.5" aria-hidden="true" focusable="false">
        <path d="M5 5a2 2 0 0 1 3.008-1.728l11.997 6.998a2 2 0 0 1 .003 3.458l-12 7A2 2 0 0 1 5 19z" />
    </svg>
)

export function Playground({ code, output }) {
    const edRef = useRef(null)
    const editorRef = useRef(null)
    const defaultText = fromB64(output).replace(/\n$/, '')
    const defaultCode = fromB64(code).replace(/\n$/, '')
    // Stable ref: React 19 compares dangerouslySetInnerHTML by object identity, so a fresh `{__html}` each render re-applies innerHTML and wipes CodeJar/Shiki's spans (e.g. while running).
    const seedHtml = useMemo(() => ({ __html: escapeHtml(defaultCode) }), [defaultCode])
    const [result, setResult] = useState(null) // null = showing default; else { text, error, ms }
    const [running, setRunning] = useState(false)
    const [phase, setPhase] = useState(null) // cold-start and exec phase, host, worker or running

    // Live guard: the mount effect captures runCode once, so a `running` state read here would be the first render's `false` forever — Ctrl+Enter could then fire concurrent runs. A ref reads the current value through that stale closure.
    const runningRef = useRef(false)
    const runCode = useCallback(async (src) => {
        if (runningRef.current) return
        runningRef.current = true
        setRunning(true)
        // Byte-stream contract: each chunk is raw stdout (print already includes its own `end`); concatenate, don't join.
        let buf = ''
        // Don't clear the terminal up front (collapse + flicker); keep old text until the first chunk replaces it.
        try {
            const res = await run(src, (chunk) => {
                buf += chunk
                setResult({ text: buf, error: '', ms: 0 })
            }, setPhase)
            setResult({ text: buf, error: res.error, ms: res.ms })
        } catch (e) {
            setResult({ text: buf, error: String(e?.message ?? e), ms: 0 })
        } finally {
            runningRef.current = false
            setRunning(false)
            setPhase(null)
        }
    }, [])

    // Mount the CodeJar editor + Shiki highlighter client-side (lazy: keeps codejar/shiki out of SSR).
    useEffect(() => {
        let editor
        let highlight
        let disposed = false
        Promise.all([import('./editor.js'), import('./shiki.js')]).then(([{ createEditor }, { createHighlight }]) => {
            if (disposed || !edRef.current) return
            highlight = createHighlight(() => editor && editor.setCode(editor.getCode()))
            editor = createEditor({
                ed: edRef.current,
                defaultCode,
                onRun: (src) => runCode(src),
                highlight,
            })
            editorRef.current = editor
        })
        // Tear down CodeJar and (critically) the Shiki MutationObserver on <html>; that observer holds a closure over the editor, so without dispose() each unmounted Playground stays pinned in memory.
        return () => {
            disposed = true
            editor?.destroy()
            highlight?.dispose()
            editorRef.current = null
        }
    }, []) // eslint-disable-line react-hooks/exhaustive-deps

    const onRunClick = () => runCode(editorRef.current?.getCode() ?? defaultCode)

    const fmt = (ms) => (ms < 1000 ? `${ms.toFixed(0)}ms` : `${(ms / 1000).toFixed(2)}s`)
    const liveText = result ? applyTerminalControls(result.text).replace(/\n$/, '') : ''
    const differs = result && !result.error && liveText !== defaultText
    const termBody = result ? [liveText, result.error].filter(Boolean).join('\n') : defaultText
    const phaseLabel = { host: 'loading host…', worker: 'initializing worker…', running: 'running…' }
    const header = running
        ? `Output · ${phaseLabel[phase] ?? 'running…'}`
        : !result
            ? 'Output · expected'
            : result.error
                ? 'Output · failed'
                : differs
                    ? 'Output · differs'
                    : `Output · ${fmt(result.ms)}`

    return (
        <div className="ep-pg my-5">
            {/* Input: CodeJar editor, framed like a Nextra code block. */}
            <div className="ep-editor overflow-hidden rounded-md border border-gray-300 bg-white text-sm dark:border-neutral-700 dark:bg-black">
                {/* Seed the plain code into the HTML so it's visible on first paint (before editor.js/shiki load). `whitespace-pre` keeps line breaks pre-mount; CodeJar takes over on mount. Stable `seedHtml` ref -> React won't re-apply innerHTML and clobber CodeJar's DOM on later re-renders. */}
                <div ref={edRef} className="ep-ed py-2 font-mono whitespace-pre" aria-label="Python source editor" suppressHydrationWarning dangerouslySetInnerHTML={seedHtml}/>
            </div>

            {/* Output: thin header (status + Run) over Nextra's Pre/Code body (plain text, no highlight). */}
            <div className="ep-output mt-3" aria-live="polite">
                <div className="flex items-center justify-between gap-2 rounded-t-md border border-b-0 border-gray-300 bg-gray-100 pl-3 pr-1 py-1 dark:border-neutral-700 dark:bg-neutral-900">
                    <span className="font-mono text-xs text-gray-700 dark:text-gray-200">{header}</span>
                    <Button variant="outline" onClick={onRunClick} className="ep-run text-xs flex items-center px-2.5 py-1.5 gap-1.5 transition" disabled={running} title="Run code" aria-label="Run code"><PlayIcon/>Run</Button>
                </div>
                <Pre>
                    <Code>{termBody}</Code>
                </Pre>
            </div>
        </div>
    )
}
