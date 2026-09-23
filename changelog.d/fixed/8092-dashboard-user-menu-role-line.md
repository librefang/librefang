Show the signed-in user's role under their name in the dashboard user menus instead of the dashboard auth mode.
Both the sidebar row and the avatar dropdown built that line from the auth mode, so a `hybrid` deployment labelled every user `hybrid · <host>` whatever their privilege, which says how the daemon accepts credentials rather than who is signed in.
The line now reads the RBAC role (`viewer` / `user` / `admin` / `owner`) that `/api/authz/whoami` already returned and the dashboard discarded, and both panels build it through one helper so they cannot disagree again. (#8092) (@houko)
