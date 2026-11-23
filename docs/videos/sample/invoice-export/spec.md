---
type: Spec
title: Invoice export
description: Let account owners download their invoices as a CSV file.
tags: [billing, api]
verified:
  - { by: human:Product team, at: 2026-11-14 }
---

## Before the build

Export only the invoices the caller can see. Keep every amount in minor units so the totals stay exact.

- [x] Agree on the CSV columns with finance
- [ ] Put the export behind a feature flag

## Endpoints

| Endpoint | Behavior |
| --- | --- |
| POST /exports | Starts an export job |
| GET /exports/{id} | Returns the status and link |
| DELETE /exports/{id} | Cancels a running job |

## Rollout

Hand the steps in the [build plan](build-plan.md) to the factory and ship to one account before the next release.
