# AionCore Fleet Overlay (Strategy A)

Applies Multica-**semantic** Fleet tables into an AionCore checkout.
Source of behavior for P0–P3 demos remains `munder-fleet-a/src/fleet` until the Rust crate is fully wired.

## Fork point

See `FORK_POINT.md` (mirrors `docs/VERSIONS.md` in munder-fleet-a).

## Apply

```bash
./scripts/apply-fleet-overlay.sh
# from munder-fleet-a root; target defaults to refs/AionCore
```

## Remaining Rust wiring (org fork)

1. Add `aionui-fleet` crate (routes/service) or module under `aionui-project`.
2. Expose `/api/fleet/*` behind auth middleware; `--local` uses `system_default_user`.
3. Port DecisionGate onto tool confirmation path (P1).
4. Keep Apache-2.0 NOTICE; record fork SHA.

## License

AionCore: Apache-2.0. Multica: do **not** copy source — protocol rewrite only.
