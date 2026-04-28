#!/bin/bash
# Lint script to check for forbidden inline fetch/api calls in pages
# Per AGENTS.md: All API access must go through hooks in src/lib/queries/
#
# EXCEPTIONS (must be commented in code):
#   - File downloads (blob responses)
#   - SSE/streaming
#   - One-shot probes that must not be cached

set -e

PAGES_DIR="src/pages"
echo "🔍 Checking $PAGES_DIR for forbidden inline fetch/api calls..."
echo ""

EXIT_CODE=0
VIOLATIONS=()

# Check for fetch( calls that are NOT .refetch() (TanStack Query)
while IFS=: read -r file line content; do
    # Skip test files
    [[ "$file" == *.test.tsx ]] && continue

    # Skip .refetch() (TanStack Query pattern - allowed)
    [[ "$content" == *.refetch* ]] && continue

    # Skip navigator.clipboard, navigator.share (browser APIs)
    [[ "$content" == *navigator.* ]] && continue

    # Skip lazy imports
    [[ "$content" == *"lazy("* ]] && continue

    # Skip fetch inside strings
    [[ "$content" == *'"'*fetch*'"'* ]] && continue

    # Check for exception comments in the file (skip if exception is documented)
    if grep -qE "Exception|exception|file download|SSE/streaming|streaming" "$file" 2>/dev/null; then
        continue
    fi

    # Skip commented out lines
    [[ "$content" == *"//"* ]] && continue

    basename=$(basename "$file")
    VIOLATIONS+=("  $basename:$line: $content")
    EXIT_CODE=1
done < <(grep -rn "[^a-zA-Z0-9_]fetch\s*(" "$PAGES_DIR"/*.tsx 2>/dev/null | head -10)

# Print violations
if [ ${#VIOLATIONS[@]} -gt 0 ]; then
    echo "⚠️  Potential fetch() violations found (may have exceptions):"
    printf '%s\n' "${VIOLATIONS[@]}"
    echo ""
fi

# Summary
page_count=$(ls -1 "$PAGES_DIR"/*.tsx 2>/dev/null | grep -v ".test.tsx" | wc -l | tr -d ' ')

echo "✅ Lint check complete"
echo ""
echo "Pages checked: $page_count"
echo ""

if [ ${#VIOLATIONS[@]} -gt 0 ]; then
    echo "Note: Some violations may be legitimate exceptions:"
    echo "  - File downloads (blob responses)"
    echo "  - SSE/streaming"
    echo "  - Browser APIs (clipboard, share, etc.)"
    echo ""
    echo "These should be commented in code per AGENTS.md"
    echo ""
    exit 1
fi

exit 0
