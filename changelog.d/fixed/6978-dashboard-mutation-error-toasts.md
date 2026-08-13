Add a global React Query `MutationCache` error fallback so a rejected mutation without its own `onError` handler now surfaces a localized toast instead of failing silently.
Mutations that already register a mutation-specific `onError` are left untouched to avoid duplicate feedback (#6978) (@houko)
