# React frontend (`mhm/src`)

React 19 + Vite 7 + TypeScript strict + Tailwind v4. UI primitives are shadcn-style on Base UI, in `components/ui/`. Icons are `lucide-react`, toasts are `sonner`, charts are `recharts`.

## Layout

- `App.tsx` owns the shell and routing; `main.tsx` mounts React.
- `app/` — shell composition: `MainShell`, `AuthGate`, `BootstrapGate`, `RuntimeStateProvider`, `AppUpdateRuntime`.
- `pages/` — one file per page; a page with sections becomes a folder with `index.tsx` (see `pages/settings/`).
- `stores/` — zustand: `useHotelStore`, `useAuthStore`.
- `hooks/`, `lib/`, `components/`, `contexts/`, `types/`.

## Calling the backend

Never call Tauri `invoke` directly for a PMS business write. Use the wrappers in `lib/command/`, re-exported from `lib/invokeCommand.ts`:

```ts
import { invokeCommand, invokeWriteCommand, createIdempotencyKey } from "@/lib/invokeCommand";
```

`invokeWriteCommand` attaches the idempotency key and canonical payload hash the backend requires. `tests/frontend-invoke-wrapper-guardrails.test.ts` parses this tree with the TypeScript compiler and fails the build on a raw `invoke` for a command that needs the wrapper. Read-only lookups (`get_*`, `check_availability`, `calculate_room_price_preview`) are on an explicit allow-list in that test — if you add a read command, add it there with a one-line reason.

Errors come back as structured `AppError` (`lib/appError/`). Do not stringify and re-parse them; surface `code` and let `i18n.ts` resolve the message.

## Pricing

`usePricePreview` calls `calculate_room_price_preview`. It is a preview: the authoritative amount is whatever the backend returns at commit time. Never recompute a total in the frontend to display alongside it.

Room types come from the backend as **display names containing spaces** (`"Standard Room"`, `"Deluxe Balcony"`). Never join or split a list of them on a character delimiter — use `JSON.stringify`. Use real multi-word names in fixtures; single-word placeholders hide the bug.

## Tests

Vitest + Testing Library + jsdom. Tests sit next to the code as `*.test.ts(x)`; cross-cutting suites and end-to-end flows live in `mhm/tests/` and `mhm/tests/e2e/`.

```bash
npm test                                    # whole suite
npm test -- src/pages/NightAudit.test.tsx   # one file
npm run test:watch
```

Prefer user-visible queries (`getByRole`, `getByText`) over test ids. Assert on what the operator sees, not on store internals.
