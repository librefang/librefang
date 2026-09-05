Apply the memory namespace ACL to `GET /api/memory/items/{memory_id}/history`, which was the one proactive-memory read that built no guard at all.
A user whose configured `memory_access` excludes the `proactive` namespace was refused by every other memory read and served prior versions of a memory by this one, and because the denial never happened it left no `PermissionDenied` row in the audit chain where a privilege probe would otherwise be visible.
The route now runs the same read check as its siblings and audits the denial (#8196) (@houko)
