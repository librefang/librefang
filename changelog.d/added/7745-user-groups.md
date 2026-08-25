Groups are now a first-class entity, so a permission or an ownership decision can name a team rather than an individual.
  Every such decision previously had to enumerate people, which does not survive a support rota, an on-call shift or a department where the members change and the obligation does not.
  A group has a name, a description, many-to-many membership and a list of roles conferred on every member, stored as `[[groups]]` in `config.toml` and managed through `/api/groups`, the `librefang group` commands, a dashboard page and a TUI tab.
  Membership is deliberately flat — groups do not nest, because the two things waiting on this (`Principal::Group` ownership and external identity-provider group mapping) both want flattened effective membership, and an IdP already hands us exactly that on every login.
  Roles conferred by a group reuse the role strings channel binding resolution already carries, so this adds a second reading of the identity the system has rather than a third parallel one.
  Deleting or renaming a user now updates every group that named them, in the same config write, so a removed person cannot keep the roles their membership granted. (#7913) (@houko)
