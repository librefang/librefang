Pressing Escape on a `ConfirmDialog` opened over a `Modal` now closes only the dialog, leaving the modal underneath open.
Both were `window` `keydown` handlers competing for the same event, and the modal's calls `stopImmediatePropagation`, so whichever registered first won outright — the modal closed, taking the dialog with it and dropping you out of the context you were working in.
Nothing was ever written, but the confirmation you were answering and the screen you were answering it from both vanished.
`ConfirmDialog` now joins the same Escape stack `Modal` already keeps, so the topmost layer wins by stacking order rather than by mount order, and this holds for all nineteen dashboard files that nest the two.
Enter is gated the same way: a non-destructive dialog with something stacked above it no longer confirms and fires its mutation from underneath (#8376) (@houko)
