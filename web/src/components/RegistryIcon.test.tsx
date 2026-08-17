import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
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
})
