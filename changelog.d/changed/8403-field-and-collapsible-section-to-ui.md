`Field` and `CollapsibleSection` are shared `ui/` primitives now rather than private functions inside `AgentManifestForm`, which is the only reason the rest of the dashboard could not reach them.
That is what stops the app growing a fifth `Field`: four already existed across 88 call sites with three different type scales and two wrappers, and every collapsing pane reimplemented the pattern with its own `useState` and chevron.
Promoting the manifest editor's version rather than writing a new one carries its accessibility work to all of them — `required`, `invalid` and the `role="alert"` error slot — and keeps the keyboard handling and expanded state that native `<details>`/`<summary>` brings.
`Field` keeps its `<div>` wrapper, and that is load-bearing rather than an oversight (#5246): a `<label>` forwards every click inside its bounds to its first labelable control, which silently ate clicks aimed at composite widgets such as `MultiSelectCmdk`.
A new optional `htmlFor` gives label-click-to-focus back for callers that put the label beside the control instead, which is the one thing the `<div>` cost.
No production call site passes it yet, and the manifest editor renders as it did: the moved class strings are the same tokens in a different order, so nothing about the layout changes. (#8403) (@DaBlitzStein)
`Field` and `CollapsibleSection` are shared `ui/` primitives now rather than private functions inside `AgentManifestForm`, which is the only reason the rest of the dashboard could not reach them.
That is what stops the app growing a fifth `Field`: four already existed across 88 call sites with three different type scales and two wrappers, and every collapsing pane reimplemented the pattern with its own `useState` and chevron.
Promoting the manifest editor's version rather than writing a new one carries its accessibility work to all of them — `required`, `invalid` and the `role="alert"` error slot — and keeps the keyboard handling and expanded state that native `<details>`/`<summary>` brings.
`Field` keeps its `<div>` wrapper, and that is load-bearing rather than an oversight (#5246): a `<label>` forwards every click inside its bounds to its first labelable control, which silently ate clicks aimed at composite widgets such as `MultiSelectCmdk`.
A new optional `htmlFor` gives label-click-to-focus back for callers that put the label beside the control instead, which is the one thing the `<div>` cost.
No production call site passes it yet, and the moved class strings are the same tokens in a different order, so the manifest editor's layout is unchanged.
Two behaviours in the manifest editor do change on the way.
A section that a validation error opened now stays open once the error is fixed, where it used to snap shut, so clearing one of two errors no longer collapses the section still being worked in.
The required-field asterisk is now hidden from screen readers, so a label is announced as "Name" rather than "Name star"; it still shows on screen, and a control that needs its required state announced has to set `required` or `aria-required` itself.
(#8403) (@DaBlitzStein)
