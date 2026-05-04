#!/usr/bin/env node
// Generate an Ed25519 keypair for plugin signing across the registry-worker
// and marketplace-worker. Prints the values you need to deploy to Cloudflare.
//
// Usage:
//   node web/workers/keygen.mjs
//
// Output:
//   PUBLIC KEY (raw 32-byte, base64) — paste into REGISTRY_PUBLIC_KEY var of
//     both wrangler.toml files.
//   PRIVATE KEY (PKCS#8, base64)     — feed to `wrangler secret put` for
//     each worker. NEVER commit this value.
//
// The public key is also what the daemon embeds (or fetches via TOFU) to
// verify signatures. See docs/architecture/plugin-signing.md.

import { generateKeyPairSync, createPublicKey } from 'node:crypto'

const { publicKey, privateKey } = generateKeyPairSync('ed25519')

// PKCS#8 DER for the private key — what crypto.subtle.importKey('pkcs8', ...)
// expects in the Workers runtime.
const privatePkcs8B64 = privateKey.export({ type: 'pkcs8', format: 'der' }).toString('base64')

// SPKI DER for the public key — but the daemon (ed25519_dalek) wants the raw
// 32-byte Ed25519 public key, base64. The last 32 bytes of the SPKI DER for
// Ed25519 are the raw key (the prefix is fixed: SEQ + algo OID + BIT STRING).
const spki = publicKey.export({ type: 'spki', format: 'der' })
const rawPub = spki.subarray(spki.length - 32)
const publicRawB64 = rawPub.toString('base64')

// Sanity check the round-trip — re-derive the SPKI from the raw bytes by
// rebuilding via Node, and confirm both base64s match.
const derivedSpki = createPublicKey({
  key: Buffer.concat([Buffer.from(spki.subarray(0, spki.length - 32)), rawPub]),
  format: 'der',
  type: 'spki',
}).export({ type: 'spki', format: 'der' })
if (!derivedSpki.equals(spki)) {
  console.error('FATAL: SPKI round-trip mismatch — keygen produced an invalid raw pubkey.')
  process.exit(1)
}

console.log('Ed25519 keypair generated.')
console.log('')
console.log('REGISTRY_PUBLIC_KEY (raw 32-byte, base64) — paste into both wrangler.toml [vars]:')
console.log(publicRawB64)
console.log('')
console.log('REGISTRY_PRIVATE_KEY (PKCS#8 DER, base64) — deploy as a secret:')
console.log(privatePkcs8B64)
console.log('')
console.log('Deploy commands:')
console.log('  cd web/workers/registry-worker')
console.log('  echo "<paste private key above>" | wrangler secret put REGISTRY_PRIVATE_KEY')
console.log('  cd ../marketplace-worker')
console.log('  echo "<paste private key above>" | wrangler secret put REGISTRY_PRIVATE_KEY')
console.log('')
console.log('After deploy, verify with:')
console.log('  curl https://librefang-registry.<account>.workers.dev/.well-known/registry-pubkey')
console.log('  curl https://librefang-marketplace.<account>.workers.dev/v1/pubkey')
console.log('')
console.log('STORE THE PRIVATE KEY SECURELY (1Password, secret manager). If lost or')
console.log('compromised, generate a new keypair, redeploy both workers, then update')
console.log('the daemon (rotate ~/.librefang/registry.pub or hardcoded constant).')
