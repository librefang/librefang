Report a failed backup, restore or delete instead of leaving a confirmation dialog that appears to have done nothing.
`ConfirmDialog` keeps itself open when `onConfirm` rejects, and the only other place these failures surfaced was the inline `isError` notice inside the backups card — which sits behind the modal overlay, so the operator saw a stuck dialog and no reason.
All three dialogs now catch and toast, which also lets the dialog close; `runtime.delete_backup_error` is new, the other two strings already existed and were simply never shown. (#8107) (@houko)
