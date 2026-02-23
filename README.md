# NEAR Wallet - GPUI Edition

A native macOS NEAR wallet with Touch ID support, built with GPUI.

## Features

- ✅ Create new NEAR accounts
- ✅ Import existing accounts
- ✅ Send NEAR tokens
- ✅ Transaction history
- ✅ **Touch ID authentication**
- ✅ Encrypted wallet storage

## Build

\`\`\`bash
cargo build --release
\`\`\`

## Run

\`\`\`bash
cargo run --release
\`\`\`

## Touch ID

On macOS, imported wallets are encrypted and protected with Touch ID:
1. Import wallet
2. Wallet saved to \`~/.near-wallet-wallets.json\`
3. Lock wallet
4. Unlock with Touch ID

## License

MIT
