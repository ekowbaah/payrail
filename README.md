# payrail

Payment and ledger primitives in Rust.

The domain core handles integer minor-unit money, idempotent payment creation, authorization and capture transitions, balanced journal entries, replay-safe references, provider webhook deduplication, and settlement reconciliation.

```mermaid
flowchart LR
  Request --> Idempotency
  Idempotency --> Payment
  Payment --> Ledger
  Provider --> Inbox[Webhook inbox]
  Provider --> Reconciliation
  Ledger --> Reconciliation
```

```sh
cargo test
```

This repository does not process real funds, verify provider signatures, or store card data. It is a tested domain core, not a payment processor.
