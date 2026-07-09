//! Slim section tab strip (top tier). Reads/writes the `active_section`
//! signal provided by `Toolbar` via context. Pure view state — clicking a
//! tab is the only thing that writes it.

use leptos::prelude::*;

use super::section::ToolbarSection;

#[component]
pub fn TabStrip() -> impl IntoView {
    let active = expect_context::<RwSignal<ToolbarSection>>();

    view! {
        <div class="tb-tabstrip" role="tablist">
            {ToolbarSection::all().into_iter().map(|s| {
                let class = move || if active.get() == s { "tb-tab active" } else { "tb-tab" };
                view! {
                    <button
                        class=class
                        role="tab"
                        on:click=move |_| active.set(s)
                    >
                        {s.label()}
                    </button>
                }
            }).collect_view()}
        </div>
    }
}
