# Modal dialogs

`Modal` is the generic dialog primitive in `src/components/modal.rs`. It owns the
structural concerns every modal shares (backdrop, Esc-to-close, click-outside,
focus on mount) and nothing else. Domain content goes in `children`.

## Usage

```rust
use crate::components::modal::{Modal, ModalSize};

#[component]
pub fn MyDialog() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let on_close = Callback::new(move |()| state.named_ranges_modal_open.set(false));

    view! {
        <Show when=move || state.named_ranges_modal_open.get()>
            <Modal
                title="My Dialog".to_string()
                on_close=on_close
                size=ModalSize::Medium
            >
                // domain content
            </Modal>
        </Show>
    }
}
```

The host owns the open/closed signal and mounts the modal conditionally with
`<Show>`. `Modal` does not track its own visibility — it assumes "I am
mounted, therefore I am open."

## Props

| Prop       | Type             | Default             | Notes                                              |
|------------|------------------|---------------------|----------------------------------------------------|
| `title`    | `String`         | required            | Rendered in the header next to the X icon          |
| `on_close` | `Callback<()>`   | required            | Fires on backdrop click, Esc, and the X button     |
| `size`     | `ModalSize`      | `ModalSize::Medium` | Maps to `.md-sm` / `.md-md` / `.md-lg` CSS classes |
| `children` | `Children`       | required            | Body content                                       |

## Behaviors

- **Backdrop click closes.** `leptos_use::on_click_outside` is anchored to the
  inner `.md-box`, so any pointerdown on the dim backdrop fires `on_close`.
  Clicks inside the box bubble normally.
- **Esc closes from anywhere.** The keydown listener registers on `document`,
  not the box, so a focused `<select>` popup or any child grabbing focus
  cannot eat the key.
- **Focus on mount.** An effect calls `.focus()` on the box once the node
  appears, so screen readers and keyboard users land inside the dialog rather
  than on the page beneath.
- **One close channel.** The X button, Esc, and outside-click all funnel into
  the same `on_close` callback. The host has exactly one place to react to
  "user wants this closed."

## Why mount conditionally

`Modal` registers a document-level keydown listener while mounted, and
`leptos_use` unbinds the listener on owner drop. Wrapping `<Modal />` in
`<Show>` is the cleanest way to get correct setup/teardown. Toggling
`display: none` on a permanently-mounted modal would leave the Esc listener
firing while the dialog is invisible.

## Sizing

`ModalSize` maps to a CSS class on `.md-box`:

```rust
pub enum ModalSize { Small, Medium, Large }
```

Width and padding live in stylesheets, not Rust string formatting. Add new
sizes by extending the enum and the CSS — the exhaustive match in
`css_class()` will fail to compile until covered.

## Real example

`src/components/named_ranges/mod.rs` is the canonical consumer:

- `WorkbookState::named_ranges_modal_open: Split<bool>` is the open signal
- `<Show when=...>{<Modal ... />}</Show>` mounts conditionally
- `on_close` clears the signal, which unmounts the modal and the listener with it

See also: [building-components.md](building-components.md) for the
`ContextMenu` pattern, which solves a similar problem (overlay positioning,
click-outside, focus return) for right-click menus.
