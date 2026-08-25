import { describe, expect, it } from 'vitest'
import { comparePopularThenName, isPopular, sortByPopular } from './registry-sort'

describe('registry popularity ordering', () => {
  const items = [
    { name: 'Zulu' },
    { name: 'Beta', tags: ['popular'] },
    { name: 'Alpha', tags: ['popular'] },
  ]

  it('places popular items first and sorts each group by name', () => {
    expect(sortByPopular(items).map((item) => item.name)).toEqual(['Alpha', 'Beta', 'Zulu'])
    expect(items.map((item) => item.name)).toEqual(['Zulu', 'Beta', 'Alpha'])
  })

  it('exposes the same comparator and popularity predicate', () => {
    expect([...items].sort(comparePopularThenName).map((item) => item.name)).toEqual([
      'Alpha',
      'Beta',
      'Zulu',
    ])
    expect(isPopular(items[0])).toBe(false)
    expect(isPopular(items[1])).toBe(true)
  })
})
