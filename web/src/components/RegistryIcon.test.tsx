import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import RegistryIcon from './RegistryIcon'

describe('RegistryIcon', () => {
  it('resolves lucide and brand icon names without case sensitivity', () => {
    const github = renderToStaticMarkup(<RegistryIcon icon="Lucide:GitHub" className="registry-size" />)
    const search = renderToStaticMarkup(<RegistryIcon icon="lucide:SEARCH" className="registry-size" />)

    expect(github).toContain('fill="currentColor"')
    expect(github).toContain('class="registry-size"')
    expect(search).toContain('class="lucide lucide-search registry-size"')
  })

  it('forwards common and fallback classes to legacy emoji icons', () => {
    const markup = renderToStaticMarkup(
      <RegistryIcon icon="🦊" className="shared-color w-10" fallbackClassName="text-4xl" />,
    )

    expect(markup).toContain('shared-color')
    expect(markup).toContain('w-10')
    expect(markup).toContain('text-4xl')
    expect(markup).not.toContain('text-xl')
  })

  it('warns during development when a normalized icon name is unknown', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})

    const markup = renderToStaticMarkup(<RegistryIcon icon="Lucide:Not-In-The-Map" />)

    expect(markup).toContain('lucide-box')
    expect(warn).toHaveBeenCalledWith('Unknown registry icon: Lucide:Not-In-The-Map')
    warn.mockRestore()
  })
})
