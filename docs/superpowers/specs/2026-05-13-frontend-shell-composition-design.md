# Frontend Shell Composition Design

Date: 2026-05-13

Issues: #138, #139

Planned PR title: `frontend: reduce App shell to composition`

## Goal

Reduce `mhm/src/App.tsx` to top-level composition by extracting runtime listeners and shell gates into named frontend units. The refactor should close both #138 and #139 without changing routing, auth behavior, store shape, backend command names, or PMS command semantics.

This work intentionally excludes #140. It must not normalize raw `invoke` usage or introduce a broader command wrapper migration.

## Scope

In scope:

- Extract runtime listeners from `App.tsx`.
- Extract bootstrap loading/onboarding behavior from `App.tsx`.
- Extract locked-app auth gating from `App.tsx`.
- Hide gateway/MCP shell UI and MCP toast listener from the normal app profile.
- Hide gateway/MCP settings entry/panel from the normal app profile.
- Add a Vite build flag for experimental gateway/MCP UI, defaulting off.
- Preserve existing backup, crash reporting, app update, DB refresh, routing, and sheet behavior.

Out of scope:

- React Router changes.
- Auth store or hotel store shape changes.
- Backend command, payload, or PMS write semantics changes.
- Gateway/MCP backend changes.
- `invokeCommand` or raw `invoke` cleanup from #140.
- Visual redesign of the shell.

## Architecture

`App.tsx` becomes a composition root. It should wire providers and named units, not directly own bootstrap branches, auth branches, event listeners, gateway status calls, backup state machines, or crash recovery handlers.

The intended structure is:

```tsx
<BootstrapStateProvider>
  <AppUpdateRuntime>
    <RuntimeStateProvider experimentalGatewayUi={isExperimentalGatewayUiEnabled}>
      <BootstrapGate>
        {({ bootstrap }) => (
          <AuthGate bootstrap={bootstrap}>
            <MainShell experimentalGatewayUi={isExperimentalGatewayUiEnabled} />
          </AuthGate>
        )}
      </BootstrapGate>
    </RuntimeStateProvider>
  </AppUpdateRuntime>
</BootstrapStateProvider>
```

The exact component names may vary during implementation, but the boundary should stay intact: gates decide whether the shell can render, runtime units own side effects, and the shell renders UI.

## Components

### `BootstrapStateProvider` and `BootstrapGate`

The bootstrap boundary owns the initial `get_bootstrap_status` command, loading state, bootstrap status state, and `hydrateFromBootstrap`.

`BootstrapGate` renders:

- loading screen while bootstrap status is pending,
- `OnboardingWizard` when setup is incomplete,
- children when setup is complete.

Onboarding completion updates local bootstrap state and hydrates the current user, matching current behavior.

### `AuthGate`

Owns the locked-mode session check and login fallback. It receives bootstrap status from the bootstrap boundary through props or context; it must not refetch bootstrap status. When `bootstrap.app_lock_enabled` is true, it calls `checkSession()` and renders `LoginScreen` until authenticated. When app lock is disabled, it renders children as today.

This component does not change auth store state shape or login behavior.

### `AppUpdateRuntime`

Owns the `useAppUpdateController` call and the one-time silent update check after the shell is ready. It consumes shell readiness from the bootstrap boundary and wraps descendants with the existing `AppUpdateProvider`, so `MainShell` and settings descendants can continue to use the app update context. This keeps update runtime behavior out of `App.tsx` while preserving the existing provider contract.

### `RuntimeStateProvider`

Owns runtime side effects and exposes UI-facing runtime state to `MainShell` through context or a render-prop boundary. It may mount before the shell gate renders so listener enablement must be controlled per listener, not by a single broad `shellReady` flag.

Per-listener enablement is the source of truth:

| Runtime concern | Enable condition | Reason |
| --- | --- | --- |
| `backup-status` listener | immediately while the app root is mounted | Preserves current unconditional listener behavior; the shell may render the indicator later, but events should still update runtime state. |
| `db-updated` listener | `isAuthenticated` | Matches current behavior and avoids store refresh work before an authenticated session exists. |
| crash recovery lookup and prompt state | `shellReady`, once per app mount | Matches current behavior; diagnostics must not block loading, onboarding, or login gates. |
| app update silent check | `shellReady`, once per app mount | Matches current behavior and keeps update checks out of pre-shell gates. |
| `gateway_get_status` | `isAuthenticated && experimentalGatewayUi` | Preserves the existing authenticated check when enabled while hiding gateway status from the normal profile. |
| `mcp_reservation_created` listener and toast | `isAuthenticated && experimentalGatewayUi` | Preserves the existing authenticated listener when enabled while hiding MCP toasts from the normal profile. |

Backup and crash reporting are normal profile runtime features, not experimental features.

### `MainShell`

Owns shell rendering only:

- sidebar navigation,
- header,
- active page switch,
- user badge/logout,
- sidebar collapsed state, localStorage persistence, and resize handling, either directly or through a shell-local hook such as `useSidebarCollapse`,
- app update badge and restart modal,
- backup status indicator and failure alert,
- crash report prompt,
- check-in sheets,
- toaster.

`MainShell` must not call `listen()` or runtime `invoke()` directly. UI-local effects such as sidebar resize handling are allowed in `MainShell` or its local hooks, but not in `App.tsx`.

### `runtimeProfile`

Adds a small frontend profile module backed by a Vite constant:

- `__EXPERIMENTAL_GATEWAY_UI__`
- default value: `false`
- enabling env: `CAPYINN_EXPERIMENTAL_GATEWAY_UI=1|true|yes|on`

Implementation must update every existing global-define surface:

- `mhm/vite.config.ts`
- `mhm/vitest.config.ts`
- `mhm/src/vite-env.d.ts`

When false, the normal shell shows no gateway/MCP badge, does not call `gateway_get_status`, does not subscribe to `mcp_reservation_created`, and does not expose a gateway/MCP settings entry or panel.

## Data Flow

Bootstrap data flows from the bootstrap boundary into `BootstrapGate`, `AuthGate`, `RuntimeStateProvider`, and `AppUpdateRuntime` through props or context. `AuthGate` consumes that existing bootstrap state; it must not issue a second bootstrap fetch. Auth readiness is still derived from the existing auth store and bootstrap status. Shell readiness remains equivalent to the current expression:

```ts
!bootstrapLoading &&
Boolean(bootstrap?.setup_completed) &&
(!bootstrap?.app_lock_enabled || isAuthenticated)
```

Runtime state flows from `RuntimeStateProvider` to `MainShell` through a small render-prop or context boundary. The state should include only UI-facing runtime data and handlers, such as backup status, visible backup failure, crash prompt state, crash handlers, and optional gateway status.

Page navigation remains the existing `useHotelStore().activeTab` switch. No route model changes are introduced.

## Error Handling

Diagnostics lookups continue to be non-blocking. Crash recovery failures must not prevent shell rendering.

Backup failure behavior remains unchanged: failed jobs show a toast, display failed status, and show a persistent alert until dismissed or cleared by queue drain.

Gateway status lookup failures remain non-fatal and should only set the optional gateway UI status to off when experimental UI is enabled.

## Testing

Validation for the implementation should include:

```bash
cd mhm && npm test
cd mhm && npm run build
rg -n "gateway_get_status|mcp_reservation_created|backup-status|get_pending_crash_report" mhm/src/App.tsx
rg -n "useEffect|useState|useRef|listen\\(|invoke\\(|localStorage|addEventListener" mhm/src/App.tsx
wc -l mhm/src/App.tsx
```

Expected results:

- tests pass,
- build passes,
- `App.tsx` has no direct runtime listener setup,
- `App.tsx` has no hook-owned runtime or UI controller logic,
- any remaining runtime-token matches in `App.tsx` are limited to imported component names or explicit composition props, not event or command ownership,
- `App.tsx` is materially shorter and reads as composition.

Focused test updates should cover:

- app update flow still waits for shell readiness,
- backup status integration behavior remains unchanged,
- crash prompt behavior remains unchanged,
- onboarding and locked-login gates still render in the same conditions,
- normal profile hides gateway/MCP UI and does not call `gateway_get_status`,
- normal profile does not expose the gateway/MCP settings entry or panel,
- experimental profile can show gateway/MCP UI when the build flag is enabled,
- settings tests that currently navigate to `MCP Gateway` are updated to assert the normal hidden state and a separate experimental visible path,
- sidebar collapse persistence and resize behavior still work after moving UI-local effects out of `App.tsx`.

## Acceptance Criteria

- `App.tsx` is a composition root, not a runtime controller.
- Bootstrap and auth/onboarding gates live in named units.
- Runtime listeners live outside `App.tsx`.
- Sidebar collapse localStorage and resize behavior live outside `App.tsx`.
- Runtime listener enablement follows the per-listener table above.
- Normal profile shows no gateway/MCP badge, toast, settings panel, or settings entry by default.
- Experimental gateway/MCP UI appears only behind the explicit Vite build flag.
- Listener behavior is preserved where still enabled.
- No #140 invoke-wrapper migration is included.
- Validation commands pass.
