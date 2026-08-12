Move the identity-file path resolution, directory creation, and copy work performed by `POST /api/agents/{id}/clone` onto Tokio's blocking pool instead of running it inline on the async worker thread handling the request.
The request-local registry guards and error translator are released before awaiting the copy task, so cloning no longer holds those locks across blocking filesystem work.
Migrated `.identity/` files are still preferred over legacy workspace-root files, with the same fallback behaviour as before (#6987) (@houko)
