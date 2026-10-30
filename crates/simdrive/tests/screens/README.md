These 480 × 800 screenshots come from the golden `scripts/sim-e2e.sh` scenario
in Foundation SDK 1.1.0's standard pinned Nix shell, using the corrected RGBA
simulator capture tool documented in `patches/keyos/README.md`.

- `home.png`: after importing the fixture federation, before a review.
- `review.png`: the golden Lightning invoice review.
- `result.png`: the golden ecash token result (known test seed, no real funds).

They exercise the actual control geometry alongside the synthetic shifted-footer,
partial-draw, malformed-screen, and wrong-color regression cases. The token is
public test data from the repository's golden vectors, never a live payment.
