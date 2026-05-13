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
- Hide gateway/MCP settings entry/panel from the normal app profile if it remains reachable through the normal settings UI.
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
<BootstrapGate>
  {({ shellReady }) => (
    <AppUpdateRuntime enabled={shellReady}>
      {(appUpdate) => (
        <AppUpdateProvider value={appUpdate}>
          <AuthGate>
            <RuntimeListeners
              enabled={shellReady}
              experimentalGatewayUi={isExperimentalGatewayUiEnabled}
            >
              {(runtimeState) => (
                <MainShell
                  appUpdate={appUpdate}
                  runtimeState={runtimeState}
                  experimentalGatewayUi={isExperimentalGatewayUiEnabled}
                />
              )}
            </RuntimeListeners>
          </AuthGate>
        </AppUpdateProvider>
      )}
    </AppUpdateRuntime>
  )}
</BootstrapGate>
```

The exact component names may vary during implementation, but the boundary should stay intact: gates decide whether the shell can render, runtime units own side effects, and the shell renders UI.

## Components

### `BootstrapGate`

Owns the initial `get_bootstrap_status` command, loading state, bootstrap status state, and `hydrateFromBootstrap`. It renders:

- loading screen while bootstrap status is pending,
- `OnboardingWizard` when setup is incomplete,
- children when setup is complete.

Onboarding completion updates local bootstrap state and hydrates the current user, matching current behavior.

### `AuthGate`

Owns the locked-mode session check and login fallback. When `bootstrap.app_lock_enabled` is true, it calls `checkSession()` and renders `LoginScreen` until authenticated. When app lock is disabled, it renders children as today.

This component does not change auth store state shape or login behavior.

### `AppUpdateRuntime`

Owns the `useAppUpdateController` call and the one-time silent update check after the shell is ready. This keeps update runtime behavior out of `App.tsx` while preserving the existing `AppUpdateProvider` contract for descendants.

### `RuntimeListeners`

Owns runtime side effects after gates clear:

- `db-updated` listener refreshes rooms and stats.
- `backup-status` listener drives backup indicator and backup failure alert state.
- crash recovery checks pending reports once and exposes send, dismiss, and export handlers.
- gateway status checks run only when experimental gateway UI is enabled.
- `mcp_reservation_created` listener and toast run only when experimental gateway UI is enabled.

Backup and crash reporting are normal profile runtime features, not experimental features.

### `MainShell`

Owns shell rendering only:

- sidebar navigation,
- header,
- active page switch,
- user badge/logout,
- app update badge and restart modal,
- backup status indicator and failure alert,
- crash report prompt,
- check-in sheets,
- toaster.

`MainShell` must not call `listen()` or runtime `invoke()` directly.

### `runtimeProfile`

Adds a small frontend profile module backed by a Vite constant:

- `__EXPERIMENTAL_GATEWAY_UI__`
- default value: `false`
- enabling env: `CAPYINN_EXPERIMENTAL_GATEWAY_UI=1|true|yes|on`

When false, the normal shell shows no gateway/MCP badge, does not call `gateway_get_status`, does not subscribe to `mcp_reservation_created`, and does not expose a gateway/MCP settings entry or panel.

## Data Flow

Bootstrap data flows from `BootstrapGate` into the gate tree. Auth readiness is still derived from the existing auth store and bootstrap status. Shell readiness remains equivalent to the current expression:

```ts
!bootstrapLoading &&
Boolean(bootstrap?.setup_completed) &&
(!bootstrap?.app_lock_enabled || isAuthenticated)
```

Runtime state flows from `RuntimeListeners` to `MainShell` through a small render-prop or context boundary. The state should include only UI-facing runtime data and handlers, such as backup status, visible backup failure, crash prompt state, crash handlers, and optional gateway status.

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
wc -l mhm/src/App.tsx
```

Expected results:

- tests pass,
- build passes,
- `App.tsx` has no direct runtime listener setup,
- any remaining matches in `App.tsx` are limited to imported component names or explicit composition props, not event or command ownership,
- `App.tsx` is materially shorter and reads as composition.

Focused test updates should cover:

- app update flow still waits for shell readiness,
- backup status integration behavior remains unchanged,
- crash prompt behavior remains unchanged,
- onboarding and locked-login gates still render in the same conditions,
- normal profile hides gateway/MCP UI and does not call `gateway_get_status`,
- normal profile does not expose the gateway/MCP settings entry or panel,
- experimental profile can show gateway/MCP UI when the build flag is enabled.

## Acceptance Criteria

- `App.tsx` is a composition root, not a runtime controller.
- Bootstrap and auth/onboarding gates live in named units.
- Runtime listeners live outside `App.tsx`.
- Normal profile shows no gateway/MCP badge, toast, settings panel, or settings entry by default.
- Experimental gateway/MCP UI appears only behind the explicit Vite build flag.
- Listener behavior is preserved where still enabled.
- No #140 invoke-wrapper migration is included.
- Validation commands pass.
