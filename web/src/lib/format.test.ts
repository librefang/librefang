import { describe, expect, it } from 'vitest'
import { fmtNum } from './format'

describe('fmtNum', () => {
  it('renders raw digits below 1k', () => {
    expect(fmtNum(0)).toBe('0')
    expect(fmtNum(1)).toBe('1')
    expect(fmtNum(999)).toBe('999')
  })

  it('renders thousands with one decimal place', () => {
    expect(fmtNum(1_000)).toBe('1.0k')
    expect(fmtNum(1_234)).toBe('1.2k')
    expect(fmtNum(12_345)).toBe('12.3k')
    expect(fmtNum(999_000)).toBe('999.0k')
  })

  it('cascades to m when k rounds to 1000', () => {
    expect(fmtNum(999_950)).toBe('1.0m')
    expect(fmtNum(999_999)).toBe('1.0m')
    expect(fmtNum(1_000_000)).toBe('1.0m')
    expect(fmtNum(2_345_678)).toBe('2.3m')
  })

  it('cascades to b when m rounds to 1000', () => {
    expect(fmtNum(999_950_000)).toBe('1.0b')
    expect(fmtNum(1_500_000_000)).toBe('1.5b')
  })

  it('clamps invalid input to 0', () => {
    expect(fmtNum(-1)).toBe('0')
    expect(fmtNum(NaN)).toBe('0')
    expect(fmtNum(Infinity)).toBe('0')
  })

  it('truncates fractional input below 1k', () => {
    expect(fmtNum(42.7)).toBe('42')
  })
})
