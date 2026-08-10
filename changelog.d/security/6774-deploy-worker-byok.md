The hosted Fly deploy flow no longer copies a shared OpenRouter credential into user-owned machines, where every deployer could inspect and reuse it.
  Deployers now provide their own key, and the Worker forwards only that caller-owned credential into the caller's Fly machine configuration (#6774) (@houko)
