The prompt's memory section now enforces an explicit character budget instead of relying on the product of two independent caps that nothing stated or checked.
A shortened memory is cut at a sentence boundary where one fits and at a word boundary where that keeps most of the text, so a bullet ends mid-token only when neither is available.
Memories dropped for space are reported with their count.
(#7867) (@nevgenov)
