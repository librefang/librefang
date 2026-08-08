`browser_read_page` no longer drops the destination of every link nested inside a list item, and no longer returns a single card from a feed or search-results page.
The extraction script's `li` branch flattened the item to `textContent` and returned before descending, so a nested anchor reached the model as bare text with no URL — 1,100 of 1,723 anchors on the Rust Wikipedia article and 11 of 11 on a DuckDuckGo results page.
Clicking the text was not a fallback for those: 408 of the 1,100 do not resolve to themselves under `browser_click`'s substring matcher, so there was neither a URL nor a working text handle.
The branch now recurses and folds its children back onto one line, so a bullet still renders as a bullet and the links inside it keep their identity.
Root selection used `querySelector('main, article, [role="main"], .content, #content')`, which returns the *first* match — on a page built from sibling `<article>` cards that is one card, measured at 13.7% of a DuckDuckGo results page.
Selection now climbs to the ancestor holding repeated sibling `article` elements, the way Readability resolves the same shape by walking to the common ancestor of its close-scoring candidates.
A page with a `main` or `[role="main"]` landmark is unaffected, and a page with a single `article` still selects it (#6624, #6745) (@nevgenov)
