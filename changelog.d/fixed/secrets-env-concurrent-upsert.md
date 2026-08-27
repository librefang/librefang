Serialize `secrets.env` read-modify-write updates so concurrent credential saves cannot discard each other, and create Unix staging files with mode 0600 from the first open. (@xiaomo)
