interface PopularNamedItem {
  name: string
  tags?: string[]
}

export function isPopular(item: PopularNamedItem | undefined): boolean {
  return item?.tags?.includes('popular') ?? false
}

export function comparePopularThenName(a: PopularNamedItem, b: PopularNamedItem): number {
  const popularityOrder = Number(isPopular(b)) - Number(isPopular(a))
  return popularityOrder || a.name.localeCompare(b.name)
}

export function sortByPopular<T extends PopularNamedItem>(items: readonly T[]): T[] {
  return [...items].sort(comparePopularThenName)
}
