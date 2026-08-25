/** @vitest-environment jsdom */

import { describe, expect, it } from 'vitest'
import type { Detail, RegistryCategory } from '../useRegistry'
import { fuzzySubseq, resolvePastedItem } from './SearchDialog'

function item(category: RegistryCategory, id: string) {
  const detail: Detail = {
    id,
    name: id,
    description: '',
    category,
    icon: '',
  }
  return { kind: 'item' as const, category, item: detail }
}

describe('SearchDialog helpers', () => {
  it('penalizes subsequences whose matching span is not contiguous', () => {
    expect(fuzzySubseq('abc', 'abc')).toBe(80)
    expect(fuzzySubseq('abc', 'a--b--c')).toBe(72)
    expect(fuzzySubseq('abc', 'acb')).toBe(0)
  })

  it('requires category qualification when a pasted bare id is ambiguous', () => {
    const hits = [item('skills', 'shared'), item('plugins', 'shared'), item('agents', 'unique')]

    expect(resolvePastedItem('shared', hits)).toBeUndefined()
    expect(resolvePastedItem('unique', hits)?.category).toBe('agents')
    expect(resolvePastedItem('https://librefang.ai/zh/plugins/shared?tab=readme', hits)?.category).toBe('plugins')
  })
})
