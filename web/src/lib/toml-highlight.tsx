import type { ReactNode } from 'react'

// Line-based TOML tokenizer. Not comprehensive (no multiline string handling,
// no inline array detail), but handles the 95% of real registry manifests:
// [section] / [[array.section]] headers, keys, strings, numbers, booleans,
// arrays, and # comments. ~60 lines vs 30-50KB for Prism.

interface Span {
  className: string
  text: string
}

const STRING_RE = /"(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*'/
const NUM_RE = /-?\b\d+(?:\.\d+)?\b/
const BOOL_RE = /\btrue\b|\bfalse\b/

function highlightValue(rest: string): Span[] {
  const out: Span[] = []
  let remaining = rest
  while (remaining.length > 0) {
    if (remaining.startsWith('#')) {
      out.push({ className: 'tk-comment', text: remaining })
      break
    }
    const str = remaining.match(STRING_RE)
    if (str && str.index === 0) {
      out.push({ className: 'tk-str', text: str[0] })
      remaining = remaining.slice(str[0].length)
      continue
    }
    const num = remaining.match(NUM_RE)
    if (num && num.index === 0) {
      out.push({ className: 'tk-num', text: num[0] })
      remaining = remaining.slice(num[0].length)
      continue
    }
    const bool = remaining.match(BOOL_RE)
    if (bool && bool.index === 0) {
      out.push({ className: 'tk-bool', text: bool[0] })
      remaining = remaining.slice(bool[0].length)
      continue
    }
    // Punctuation / whitespace / identifiers — pass through up to the next
    // highlightable token.
    const next = remaining.search(/["'0-9#]|\btrue\b|\bfalse\b/)
    if (next === -1) {
      out.push({ className: 'tk-punct', text: remaining })
      break
    }
    if (next > 0) {
      out.push({ className: 'tk-punct', text: remaining.slice(0, next) })
      remaining = remaining.slice(next)
    } else {
      // Defensive: the regex class above matched a negative-lookbehind spot;
      // single-char advance so we don't loop forever on pathological input.
      out.push({ className: 'tk-punct', text: remaining[0]! })
      remaining = remaining.slice(1)
    }
  }
  return out
}

function highlightLine(line: string): Span[] {
  // Leading whitespace passthrough so indentation survives.
  const indentMatch = line.match(/^\s*/)
  const indent = indentMatch ? indentMatch[0] : ''
  const rest = line.slice(indent.length)
  const prefix: Span[] = indent ? [{ className: 'tk-punct', text: indent }] : []

  if (!rest) return prefix
  if (rest.startsWith('#')) return [...prefix, { className: 'tk-comment', text: rest }]
  // Section header: [x.y] or [[x]]
  const header = rest.match(/^(\[{1,2})([^\]]+)(\]{1,2})\s*(.*)$/)
  if (header) {
    const [, openBr, name, closeBr, trailing] = header
    return [
      ...prefix,
      { className: 'tk-punct', text: openBr! },
      { className: 'tk-header', text: name! },
      { className: 'tk-punct', text: closeBr! },
      ...(trailing ? highlightValue(trailing) : []),
    ]
  }
  // key = value
  const kv = rest.match(/^([A-Za-z_][A-Za-z0-9_.\-]*)(\s*=\s*)(.*)$/)
  if (kv) {
    const [, key, eq, value] = kv
    return [
      ...prefix,
      { className: 'tk-key', text: key! },
      { className: 'tk-punct', text: eq! },
      ...highlightValue(value || ''),
    ]
  }
  return [...prefix, { className: 'tk-punct', text: rest }]
}

export function highlightToml(source: string): ReactNode {
  const lines = source.split('\n')
  return lines.map((line, i) => (
    <span key={i}>
      {highlightLine(line).map((span, j) => (
        <span key={j} className={span.className}>{span.text}</span>
      ))}
      {i < lines.length - 1 && '\n'}
    </span>
  ))
}
