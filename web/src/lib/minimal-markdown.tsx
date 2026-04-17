import type { ReactNode } from 'react'

// Minimal Markdown renderer — enough for registry READMEs. Supports:
//   # / ## / ### headings, paragraphs, bullet and numbered lists,
//   inline `code`, **bold**, *italic*, [text](href), and fenced ``` code
// blocks. No HTML, no tables, no images. ~120 lines, zero dependencies.
//
// The tradeoff: authors of registry READMEs are expected to use plain
// Markdown. Anything exotic (HTML blocks, MDX components) renders as raw
// text — that's fine for the scope here and safer than sanitizing HTML.

interface Span { text: string; bold?: boolean; italic?: boolean; code?: boolean; href?: string }

// Escape arbitrary text for safe inclusion in keys/content.
function escapeKey(s: string): string {
  return s.slice(0, 48).replace(/[^A-Za-z0-9_-]/g, '_')
}

function renderInline(line: string): ReactNode[] {
  // Tokenize into spans: code > links > bold > italic, in that priority.
  // A single linear scan; recursion-free to keep the code small.
  const spans: Span[] = []
  let rest = line
  while (rest.length > 0) {
    // `code`
    const codeM = rest.match(/^([\s\S]*?)`([^`]+)`/)
    // [text](href)
    const linkM = rest.match(/^([\s\S]*?)\[([^\]]+)\]\(([^)]+)\)/)
    // **bold**
    const boldM = rest.match(/^([\s\S]*?)\*\*([^*]+)\*\*/)
    // *italic* (single asterisk, not part of **)
    const italM = rest.match(/^([\s\S]*?)(?<!\*)\*([^*]+)\*(?!\*)/)

    const candidates = [
      codeM && { idx: codeM.index ?? 0, pre: codeM[1]!, body: codeM[2]!, consume: codeM[0]!.length, kind: 'code' as const },
      linkM && { idx: linkM.index ?? 0, pre: linkM[1]!, body: linkM[2]!, consume: linkM[0]!.length, kind: 'link' as const, href: linkM[3]! },
      boldM && { idx: boldM.index ?? 0, pre: boldM[1]!, body: boldM[2]!, consume: boldM[0]!.length, kind: 'bold' as const },
      italM && { idx: italM.index ?? 0, pre: italM[1]!, body: italM[2]!, consume: italM[0]!.length, kind: 'ital' as const },
    ].filter(Boolean) as { idx: number; pre: string; body: string; consume: number; kind: 'code' | 'link' | 'bold' | 'ital'; href?: string }[]

    if (candidates.length === 0) {
      spans.push({ text: rest })
      break
    }
    // Pick the candidate whose pre-text is shortest (i.e. the earliest
    // occurrence in the remaining string).
    candidates.sort((a, b) => a.pre.length - b.pre.length)
    const pick = candidates[0]!
    if (pick.pre) spans.push({ text: pick.pre })
    const next: Span = { text: pick.body }
    if (pick.kind === 'code') next.code = true
    else if (pick.kind === 'bold') next.bold = true
    else if (pick.kind === 'ital') next.italic = true
    else if (pick.kind === 'link') next.href = pick.href
    spans.push(next)
    rest = rest.slice(pick.pre.length + pick.consume)
  }
  return spans.map((s, i) => {
    const key = `${i}-${escapeKey(s.text)}`
    if (s.href) {
      return (
        <a key={key} href={s.href} target="_blank" rel="noopener noreferrer" className="text-cyan-600 dark:text-cyan-400 hover:underline">
          {s.text}
        </a>
      )
    }
    if (s.code) return <code key={key} className="px-1 py-0.5 rounded bg-black/10 dark:bg-white/10 text-[0.9em] font-mono">{s.text}</code>
    if (s.bold) return <strong key={key}>{s.text}</strong>
    if (s.italic) return <em key={key}>{s.text}</em>
    return <span key={key}>{s.text}</span>
  })
}

export function renderMarkdown(md: string): ReactNode[] {
  const lines = md.split('\n')
  const out: ReactNode[] = []
  let i = 0
  let blockIdx = 0
  while (i < lines.length) {
    const line = lines[i]!
    // Fenced code block
    if (line.startsWith('```')) {
      const start = i + 1
      let end = start
      while (end < lines.length && !lines[end]!.startsWith('```')) end++
      out.push(
        <pre key={`code-${blockIdx++}`} className="overflow-x-auto text-xs font-mono p-3 my-3 bg-surface-100 border border-black/10 dark:border-white/5 rounded">
          <code>{lines.slice(start, end).join('\n')}</code>
        </pre>
      )
      i = end + 1
      continue
    }
    // ATX headings
    const h = line.match(/^(#{1,3})\s+(.*)$/)
    if (h) {
      const level = h[1]!.length
      const text = h[2]!
      const cls = level === 1 ? 'text-lg font-bold mt-6 mb-2' : level === 2 ? 'text-base font-bold mt-5 mb-2' : 'text-sm font-semibold mt-4 mb-2'
      const Tag = level === 1 ? 'h3' : level === 2 ? 'h4' : 'h5'
      out.push(<Tag key={`h-${blockIdx++}`} className={cls}>{renderInline(text)}</Tag>)
      i++
      continue
    }
    // Bullet list
    if (/^[-*+]\s+/.test(line)) {
      const items: string[] = []
      while (i < lines.length && /^[-*+]\s+/.test(lines[i]!)) {
        items.push(lines[i]!.replace(/^[-*+]\s+/, ''))
        i++
      }
      out.push(
        <ul key={`ul-${blockIdx++}`} className="list-disc pl-6 my-3 space-y-1 text-sm text-gray-700 dark:text-gray-300">
          {items.map((t, j) => <li key={j}>{renderInline(t)}</li>)}
        </ul>
      )
      continue
    }
    // Numbered list
    if (/^\d+\.\s+/.test(line)) {
      const items: string[] = []
      while (i < lines.length && /^\d+\.\s+/.test(lines[i]!)) {
        items.push(lines[i]!.replace(/^\d+\.\s+/, ''))
        i++
      }
      out.push(
        <ol key={`ol-${blockIdx++}`} className="list-decimal pl-6 my-3 space-y-1 text-sm text-gray-700 dark:text-gray-300">
          {items.map((t, j) => <li key={j}>{renderInline(t)}</li>)}
        </ol>
      )
      continue
    }
    // Blank line — paragraph break
    if (line.trim() === '') { i++; continue }
    // Paragraph: collect consecutive non-empty, non-block lines
    const paraLines: string[] = [line]
    i++
    while (i < lines.length && lines[i]!.trim() !== ''
      && !/^[-*+]\s+/.test(lines[i]!) && !/^\d+\.\s+/.test(lines[i]!)
      && !lines[i]!.startsWith('#') && !lines[i]!.startsWith('```')) {
      paraLines.push(lines[i]!)
      i++
    }
    out.push(
      <p key={`p-${blockIdx++}`} className="my-3 text-sm text-gray-700 dark:text-gray-300 leading-relaxed">
        {renderInline(paraLines.join(' '))}
      </p>
    )
  }
  return out
}
