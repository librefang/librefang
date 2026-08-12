Recover the agent context cache after a mutex poisoning event instead of permanently disabling it.
`get_cached` and `store_cached` used to give up silently once the lock was ever poisoned, which meant every future turn served no cached `context.md` and every write became a no-op for the remaining life of the process.
The cache now recovers the poisoned guard and logs a warning so the corrupted synchronization state stays observable (#7029) (@houko)
