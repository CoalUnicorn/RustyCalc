# Welcome Dialog & Templates — Design

**Date:** 2026-05-06
**Status:** Approved (pre-implementation)
**Scope tier:** A — minimum viable parity with IronCalc's welcome flow.

## Goal

When a new user opens RustyCalc with an empty `localStorage` workbook
registry, present a modal that offers a *Blank workbook* and two prebuilt
templates. Picking any tile lands the user on a real, persisted workbook
named after their choice.

This replaces the silent fall-through to `create_new()` that today gives
first-time users an unexplained empty `Workbook 1`.

## Non-goals (explicit)

- URL deep-links (`?example=…`, `?model=…`) — deferred.
- Coach-mark / guided tour overlays.
- Localization. English-only strings inline; no i18n extraction yet.
- Re-opening the dialog from a menu after the first session.
- Visual differentiation between the *Blank* tile and the template tiles.

## Triggering condition

Show the dialog **whenever the workbook registry is empty.** This covers:

- First launch (registry has never been populated).
- Returning user who has deleted every workbook.

`localStorage::load_registry().is_empty()` — equivalently, the existing
`storage::load_selected()` returning `None` — is the predicate.

The dialog is *not* available from any menu. There is no other entry
point.

## Architecture

### New module

```
src/welcome/
  mod.rs        // public Welcome component, dismissal/pick handlers
  template.rs   // Template type + per-template UserModel builders
```

The dialog view itself lives in `mod.rs`; we reuse
`crate::components::modal::Modal` for chrome (backdrop, Esc, ✕,
click-outside), so a separate `dialog.rs` is unjustified.

### Touched modules

- `src/app.rs` — bootstrap branches on registry state instead of falling
  through to `create_new()`. Provides `show_welcome: RwSignal<bool>` to
  the welcome component (local signal — no other consumer).
- `src/storage.rs` — gains a small helper for "persist an
  already-constructed `UserModel` under a fresh UUID with a given name."
  Existing `create_new_from(model)` is close, but the welcome flow needs
  to set the name explicitly rather than inheriting from the model.

## Bootstrap (`src/app.rs`)

Replace the current two lines:

```rust
let (uuid, model) = storage::load_selected().unwrap_or_else(storage::create_new);
// …
wb_state.current_uuid.set(Some(uuid));
```

with:

```rust
let (initial_uuid, model, show_welcome_initial) = match storage::load_selected() {
    Some((id, m)) => (Some(id), m, false),
    None => {
        // Ephemeral "Untitled" so the grid renders behind the modal.
        // NOT persisted; current_uuid stays None until the user picks a tile,
        // which keeps the existing debounced_save / beforeunload guards as no-ops.
        let m = UserModel::new_empty("Untitled", "en", "UTC", "en")
            .expect("blank ephemeral model");
        (None, m, true)
    }
};
let show_welcome = RwSignal::new(show_welcome_initial);
// …
wb_state.current_uuid.set(initial_uuid);
```

Render the welcome alongside the workbook:

```rust
view! {
    <div id="app">
        <LeftDrawer />
        <Workbook />
        <Welcome show=show_welcome />
    </div>
}
```

`Welcome` uses `<Show when=…>` internally to conditionally mount the
`Modal`, so when `show_welcome.set(false)` runs, the document-level
listeners installed by `Modal` are dropped.

### Verification needed during implementation

`<Workbook />` will render with the ephemeral model in `model_ctx` but
with `wb_state.current_uuid == None`. Confirm Workbook gracefully
handles the no-uuid state (or, if not, fix that as part of this work
— it is on the critical path for the welcome flow).

## Templates (`src/welcome/template.rs`)

```rust
pub struct Template {
    pub id: &'static str,
    pub title: &'static str,
    pub description: &'static str,
}

impl Template {
    /// Build the populated model for this template.
    /// `id == "blank"` returns a `UserModel::new_empty`.
    pub fn build(&self) -> UserModel<'static> {
        match self.id {
            "blank"    => UserModel::new_empty(self.title, "en", "UTC", "en")
                              .expect("blank model"),
            "mortgage" => build_mortgage(self.title),
            "travel"   => build_travel(self.title),
            _          => unreachable!("unknown template id"),
        }
    }
}

pub const TEMPLATES: &[Template] = &[
    Template {
        id: "blank",
        title: "Blank workbook",
        description: "Start with an empty spreadsheet.",
    },
    Template {
        id: "mortgage",
        title: "Mortgage calculator",
        description: "Estimate payments, interest, and overall cost.",
    },
    Template {
        id: "travel",
        title: "Travel expenses tracker",
        description: "Track trip costs and stay on budget.",
    },
];

fn build_mortgage(name: &'static str) -> UserModel<'static> { /* … */ }
fn build_travel(name: &'static str) -> UserModel<'static> { /* … */ }
```

### Template content (initial sketch)

**Mortgage calculator** — single sheet, ~25 cells:
- Inputs (B2:B4): Loan amount, Annual rate, Term (years).
- Computed (B6:B9): Monthly payment (`PMT`), Total interest, Total cost,
  Number of payments.
- Headers in column A. Currency / percent number formats applied via
  `set_user_input` plus `apply_to_range_with_format` (or whichever
  formatting helper our `ironcalc-patterns` skill recommends).

**Travel expenses tracker** — single sheet, ~30 cells:
- Header row 1: Date, Category, Description, Amount.
- ~10 sample expense rows.
- Totals row using `SUM` on the Amount column.
- Date / currency formats on the appropriate columns.

Exact cell layouts will be finalized during implementation; the spec
budgets ~20–40 cells per template.

### API used

Per `ironcalc-patterns`: `UserModel::set_user_input(sheet, row, col, value)`
for cell content, formula strings starting with `=`. No reaching into
`Model` / `EvaluationMode::Immediate` — the `UserModel` defaults are
fine for a one-shot template build.

## Storage helper (`src/storage.rs`)

Add:

```rust
/// Persist an already-constructed model under a fresh UUID. The
/// workbook's display name is read from the model itself
/// (`UserModel::get_name`). Sets the new UUID as selected. Used by the
/// welcome dialog after the user picks a tile.
pub fn create_from_template(model: UserModel<'static>)
    -> (WorkbookId, UserModel<'static>);
```

Implementation: mint UUID, write `WorkbookMeta { name: model.get_name(),
group: Ungrouped, modified: now() }`, persist model bytes, set selected.
Essentially the body of today's `create_new` minus the `Workbook N` name
generation, which is provided here by the template's `new_empty(self.title, …)`.

`create_new` itself is unchanged — it remains the path used by the "+
new workbook" flow elsewhere in the app.

> Note: today's `storage::create_new_from(model)` is close to this, but
> has `#[allow(dead_code)]` and is unused. Implementation step may
> rename and reuse it rather than introducing a parallel helper.

## Welcome component (`src/welcome/mod.rs`)

```rust
#[component]
pub fn Welcome(show: RwSignal<bool>) -> impl IntoView {
    let model_ctx: StoredValue<UserModel<'static>, LocalStorage> = expect_context();
    let wb_state: WorkbookState = expect_context();

    let pick = move |t: &'static Template| {
        let (uuid, model) = storage::create_from_template(t.build());
        wb_state.current_uuid.set(Some(uuid));
        model_ctx.set_value(model);
        show.set(false);
    };

    let dismiss = move |()| {
        // Treat dismiss-without-pick as "Blank workbook".
        pick(&TEMPLATES[0]);
    };

    view! {
        <Show when=move || show.get()>
            <Modal
                title="Choose a template".to_string()
                on_close=Callback::new(dismiss)
                size=ModalSize::Medium
            >
                <div class="welcome-tiles">
                    {TEMPLATES.iter().map(|t| view! {
                        <button
                            class="welcome-tile"
                            on:click=move |_| pick(t)
                        >
                            <h3 class="welcome-tile-title">{t.title}</h3>
                            <p class="welcome-tile-desc">{t.description}</p>
                        </button>
                    }).collect_view()}
                </div>
            </Modal>
        </Show>
    }
}
```

The single `pick` closure handles all four entry points. Dismissal
funnels into the same code path as clicking the *Blank* tile — both
result in a persisted blank workbook named `Blank workbook` (since the
*Blank* tile's title is `"Blank workbook"`, not `Workbook 1`). This is
a deliberate departure from `create_new`'s `Workbook N` scheme, on the
grounds that "Blank workbook" is a clearer trail-of-breadcrumbs in the
sidebar than `Workbook 1` for a user who arrived via the welcome flow.

`★ Note ─ closure capture: every tile's `on:click` borrows `pick`,
which itself closes over `show`, `wb_state`, `model_ctx`. Because
`pick` is `Fn` (signal setters are `Copy`), passing `&Template` and
calling it from each tile is fine without `Rc<Fn>`. If the borrow
checker pushes back during implementation, fall back to cloning a
`Callback<&Template>` per tile.`

## Styling

Two CSS rules in the existing global stylesheet (no new file):

```css
.welcome-tiles {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: var(--md-gap, 12px);
    padding: 8px 0;
}

.welcome-tile {
    /* button reset + tile look using existing tokens */
}
.welcome-tile:hover { /* token-based hover */ }
.welcome-tile-title { /* token-based title style */ }
.welcome-tile-desc  { /* token-based muted style */ }
```

Theme tokens from the existing system (light/dark) are used directly —
no welcome-specific dark-mode overrides.

## Behavior summary

| User action                          | Result                                   |
|--------------------------------------|------------------------------------------|
| First load, registry empty           | Ephemeral `Untitled` model behind modal  |
| Click *Blank workbook* tile          | Persist as `Blank workbook`, close modal |
| Click *Mortgage calculator* tile     | Build mortgage model, persist as `Mortgage calculator`, close modal |
| Click *Travel expenses tracker* tile | Build travel model, persist as `Travel expenses tracker`, close modal |
| Press Esc                            | Same as *Blank workbook*                 |
| Click backdrop                       | Same as *Blank workbook*                 |
| Click ✕                              | Same as *Blank workbook*                 |
| Subsequent launches                  | Normal `load_selected()` path; no modal  |

## Testing

- Unit-level: `Template::build("mortgage")` returns a `UserModel` whose
  formulas evaluate (e.g. `PMT` cell is non-zero). Same for travel.
- Storage-level: after `create_from_template("Mortgage calculator", model)`,
  the registry has exactly one entry, selected UUID is set, model bytes
  round-trip. Existing `storage` test patterns apply.
- UI smoke (manual, per CLAUDE.md UI testing rule): clear localStorage,
  reload, see modal; click each tile; verify the resulting workbook
  matches expectations and survives a reload.

## Open follow-ups (deferred, out of this spec)

- Add a *Templates…* entry point so returning users can re-open the
  dialog.
- URL deep-links (`?example=mortgage`).
- Localized template strings.
- More templates contributed by users / tests.
